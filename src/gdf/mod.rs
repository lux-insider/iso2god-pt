pub mod diretorio;
pub mod volume;

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use diretorio::{TabelaDiretorio, PROFUNDIDADE_MAXIMA};
use volume::{DescritorVolume, TipoIso};

use crate::erro::{Erro, Resultado};

/// Tamanho fixo do setor em uma imagem GDF (não confundir com `TAMANHO_BLOCO`
/// do container GOD, que é diferente).
const TAMANHO_SETOR: u64 = 2048;

/// Setor onde a assinatura "MICROSOFT*XBOX*MEDIA" é procurada, em um dos três
/// deslocamentos possíveis definidos por `TipoIso::deslocamento_raiz`.
const SETOR_ASSINATURA: u64 = 32;

const ASSINATURA_XBOX_MEDIA: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";

/// Tamanho, em bytes, dos campos fixos do descritor de volume lidos logo após
/// a assinatura: identificador (20) + setor raiz (4) + tamanho raiz (4) +
/// timestamp de criação (8).
const TAMANHO_DESCRITOR_VOLUME: usize = 20 + 4 + 4 + 8;

/// Representa uma imagem ISO Xbox/Xbox 360 já aberta, com sua estrutura
/// GDF (Game Disc Format) analisada. Equivalente a `Chilano.Xbox360.Iso.GDF`.
pub struct Gdf {
    leitor: BufReader<File>,
    pub tipo: TipoIso,
    pub descritor: DescritorVolume,
    /// Árvore de diretórios da raiz. `None` só acontece se a leitura inicial
    /// do diretório raiz falhar (imagem corrompida) — nesse caso `abrir`
    /// ainda assim é bem-sucedido, igual ao construtor original, que
    /// registra a exceção em `Exceptions` mas não impede o objeto de existir.
    pub raiz: Option<TabelaDiretorio>,
}

impl Gdf {
    /// Abre a imagem ISO no caminho informado, localiza a assinatura GDF
    /// (determinando se é Xsf (Xbox original)/XGD1/XGD2/XGD3) e lê a tabela
    /// de diretório raiz. Equivalente ao construtor `GDF(FileStream)`, que
    /// chama `readVolume()` e em seguida `new GDFDirTable(...)` para a raiz.
    pub fn abrir(caminho: &Path) -> Resultado<Self> {
        let arquivo = File::open(caminho)?;
        let mut leitor = BufReader::new(arquivo);

        let tamanho_arquivo = leitor.get_ref().metadata()?.len();
        let (tipo, assinatura_confirmada) = detectar_tipo(&mut leitor)?;
        let descritor =
            ler_descritor_volume(&mut leitor, tipo, tamanho_arquivo, assinatura_confirmada)?;

        // O original tolera falha ao ler o diretório raiz (guarda a exceção
        // em `Exceptions` e segue em frente com um GDF "vazio"). Replicamos
        // essa tolerância aqui em vez de abortar `abrir`.
        let raiz = match ler_tabela_diretorio(
            &mut leitor,
            &descritor,
            descritor.setor_dir_raiz,
            descritor.tamanho_dir_raiz,
        ) {
            Ok(tabela) => Some(tabela),
            Err(e) => {
                eprintln!("aviso: falha ao ler o diretório raiz da GDF: {e}");
                None
            }
        };

        Ok(Self {
            leitor,
            tipo,
            descritor,
            raiz,
        })
    }

    /// Percorre recursivamente as entradas de diretório a partir da raiz,
    /// carregando subdiretórios sob demanda (e armazenando em cache) e
    /// retornando o setor seguinte ao último ocupado por qualquer arquivo ou
    /// diretório encontrado. Equivalente a `GDF.ParseDirectory` (chamado com
    /// `recursive: true`) seguido da leitura de `lastSector`.
    pub fn analisar_diretorios(&mut self) -> Resultado<u32> {
        let mut ultimo_setor = 0u32;
        let Some(mut raiz) = self.raiz.take() else {
            return Ok(ultimo_setor);
        };
        let resultado = processar_diretorio(
            &mut raiz,
            &mut self.leitor,
            &self.descritor,
            &mut ultimo_setor,
            0,
        );
        self.raiz = Some(raiz);
        resultado?;
        Ok(ultimo_setor)
    }

    /// Confere que a árvore de diretórios não aponta para além do fim da
    /// imagem. Uma única entrada corrompida (tamanho ou setor absurdo) faz
    /// `analisar_diretorios` devolver um "último setor ocupado" muito maior
    /// que a imagem inteira — e converter isso significa ler além do fim do
    /// arquivo, preencher com zero e gravar dezenas de GB de lixo a partir
    /// de uma imagem de poucos MB. Numa imagem íntegra todo arquivo cabe
    /// dentro do volume, então esta checagem nunca dispara à toa.
    pub fn validar_ultimo_setor(&self, ultimo_setor: u32) -> Resultado<()> {
        if ultimo_setor as u64 <= self.descritor.setores_volume as u64 {
            return Ok(());
        }
        Err(Erro::IsoInvalida(format!(
            "a árvore de diretórios aponta para o setor {ultimo_setor}, além do fim da imagem \
             (que tem {} setores): a imagem está truncada ou corrompida",
            self.descritor.setores_volume
        )))
    }

    /// Verifica se um caminho existe dentro da imagem (ex: "default.xex").
    /// Carrega subdiretórios sob demanda ao longo do caminho, sem precisar de
    /// uma chamada prévia a `analisar_diretorios`. Equivalente a `GDF.Exists`.
    ///
    /// Nota: fielmente ao original, isso só é confiável para verificar a
    /// existência de *arquivos*; caminhos que terminam em um diretório podem
    /// dar falso-negativo (ver comentário em `obter_pasta`). Esse é o único
    /// uso real feito pela ferramenta (`default.xex`, `default.xbe`).
    pub fn existe(&mut self, caminho: &str) -> bool {
        let ultimo_componente = caminho.rsplit('\\').next().unwrap_or(caminho);
        let Some(raiz) = self.raiz.as_mut() else {
            return false;
        };
        let pasta = match obter_pasta(raiz, caminho, &mut self.leitor, &self.descritor, 0) {
            Ok(Some(pasta)) => pasta,
            _ => return false,
        };
        pasta.encontrar(ultimo_componente).is_some()
    }

    /// Lê o conteúdo bruto de um arquivo dentro da imagem (ex:
    /// "default.xex"). Carrega subdiretórios sob demanda ao longo do
    /// caminho, igual a `existe`. Equivalente a `GDF.GetFile`.
    pub fn ler_arquivo(&mut self, caminho: &str) -> Resultado<Vec<u8>> {
        let ultimo_componente = caminho.rsplit('\\').next().unwrap_or(caminho);
        let nao_encontrado =
            || Erro::IsoInvalida(format!("arquivo '{caminho}' não encontrado na imagem"));

        let raiz = self.raiz.as_mut().ok_or_else(nao_encontrado)?;
        let (setor, tamanho) = {
            let pasta = obter_pasta(raiz, caminho, &mut self.leitor, &self.descritor, 0)?
                .ok_or_else(nao_encontrado)?;
            let entrada = pasta.encontrar(ultimo_componente).ok_or_else(nao_encontrado)?;
            (entrada.setor, entrada.tamanho)
        };

        validar_leitura(&self.descritor, setor, tamanho, &format!("o arquivo '{caminho}'"))?;

        let posicao =
            self.descritor.deslocamento_raiz + setor as u64 * self.descritor.tamanho_setor as u64;
        self.leitor.seek(SeekFrom::Start(posicao))?;
        let mut bytes = vec![0u8; tamanho as usize];
        self.leitor.read_exact(&mut bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                Erro::IsoInvalida(format!(
                    "não foi possível ler {tamanho} bytes do arquivo '{caminho}' \
                     (a imagem acabou antes do esperado)"
                ))
            } else {
                Erro::Io(e)
            }
        })?;
        Ok(bytes)
    }
}

/// Detecta apenas o layout do disco de uma imagem, sem ler a árvore de
/// diretórios nem imprimir aviso nenhum. É o que uma listagem precisa: abrir
/// dezenas de arquivos com `Gdf::abrir` só para mostrar uma coluna seria
/// caro e encheria a tela de avisos sobre imagens que o usuário nem escolheu.
///
/// `None` quer dizer "não é (ou não parece ser) uma imagem de Xbox" — o
/// palpite XGD3 da detecção só é aceito aqui se a assinatura estiver mesmo
/// no deslocamento dele.
pub fn tipo_da_imagem(caminho: &Path) -> Option<TipoIso> {
    let mut leitor = BufReader::new(File::open(caminho).ok()?);
    let (tipo, confirmada) = detectar_tipo(&mut leitor).ok()?;
    if confirmada {
        return Some(tipo);
    }
    let posicao = SETOR_ASSINATURA * TAMANHO_SETOR + tipo.deslocamento_raiz();
    assinatura_presente(&mut leitor, posicao).ok()?.then_some(tipo)
}

/// Recusa uma leitura cujo tamanho declarado não cabe no volume — **antes**
/// de alocar o buffer.
///
/// Sem isso, um campo corrompido de 0xFFFFFFFF faz o programa pedir 4 GiB ao
/// sistema. Em máquina folgada o Linux entrega páginas zeradas que ninguém
/// toca e o problema passa despercebido; sob limite de memória (contêiner,
/// VM, cgroup) a alocação falha e o Rust **aborta** o processo — sem erro
/// tratável, sem limpeza da saída pela metade, com core dump.
fn validar_leitura(
    descritor: &DescritorVolume,
    setor: u32,
    tamanho: u32,
    o_que: &str,
) -> Resultado<()> {
    let fim = setor as u64 * descritor.tamanho_setor as u64 + tamanho as u64;
    if fim <= descritor.tamanho_volume {
        return Ok(());
    }
    Err(Erro::IsoInvalida(format!(
        "{o_que} declara {tamanho} bytes a partir do setor {setor}, terminando em {fim}, além \
         do fim do volume ({} bytes): a imagem está truncada ou corrompida",
        descritor.tamanho_volume
    )))
}

/// Lê do disco o bloco de bytes de uma tabela de diretório (setor + tamanho)
/// e delega o parsing para `TabelaDiretorio::ler`. Equivalente à parte de
/// E/S do construtor `GDFDirTable(CBinaryReader, GDFVolumeDescriptor, uint,
/// uint)` — que aqui fica separada do parsing puro em `diretorio.rs`.
fn ler_tabela_diretorio(
    leitor: &mut BufReader<File>,
    descritor: &DescritorVolume,
    setor: u32,
    tamanho: u32,
) -> Resultado<TabelaDiretorio> {
    validar_leitura(descritor, setor, tamanho, "a tabela de diretório")?;

    let posicao = descritor.deslocamento_raiz + setor as u64 * descritor.tamanho_setor as u64;
    leitor.seek(SeekFrom::Start(posicao))?;
    let mut bytes = vec![0u8; tamanho as usize];
    leitor.read_exact(&mut bytes).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            Erro::IsoInvalida(format!(
                "não foi possível ler {tamanho} bytes da tabela de diretório no setor {setor} \
                 (a imagem acabou antes do esperado) — ela pode estar corrompida, truncada, ou \
                 o tipo de disco (Xsf/XGD1/XGD2/XGD3) foi detectado incorretamente"
            ))
        } else {
            Erro::Io(e)
        }
    })?;
    TabelaDiretorio::ler(&bytes, setor, tamanho)
}

/// Percorre recursivamente `tabela`, atualizando `ultimo_setor` e carregando
/// (e armazenando em cache) qualquer subdiretório ainda não lido.
/// Equivalente a `GDF.ParseDirectory`.
fn processar_diretorio(
    tabela: &mut TabelaDiretorio,
    leitor: &mut BufReader<File>,
    descritor: &DescritorVolume,
    ultimo_setor: &mut u32,
    profundidade: u32,
) -> Resultado<()> {
    if profundidade > PROFUNDIDADE_MAXIMA {
        return Err(diretorio::erro_profundidade_excedida());
    }

    for indice in 0..tabela.entradas.len() {
        let (setor, tamanho, eh_diretorio) = {
            let entrada = &tabela.entradas[indice];
            (entrada.setor, entrada.tamanho, entrada.eh_diretorio())
        };

        let setores_ocupados =
            (tamanho as u64).div_ceil(descritor.tamanho_setor as u64) as u32;
        if setor >= *ultimo_setor {
            // Os dois vêm da imagem, então numa entrada corrompida a soma
            // estoura o u32: pânico em debug e, pior, em release ela dá a
            // volta e pode cair num valor pequeno e plausível — que passaria
            // por `validar_ultimo_setor` e produziria um pacote truncado em
            // silêncio. Saturar mantém o absurdo visível.
            *ultimo_setor = setor.saturating_add(setores_ocupados);
        }

        if !eh_diretorio {
            continue;
        }

        if tabela.entradas[indice].subdiretorio.is_none() {
            // Tolera falha ao ler um subdiretório específico (ex: tamanho
            // declarado que ultrapassa o fim do arquivo numa ISO incomum ou
            // corrompida): registra um aviso e pula essa subárvore, em vez
            // de abortar a varredura inteira. Mesma resiliência do
            // `GDF.ParseDirectory` original, que envolve a travessia toda
            // num único try/catch e simplesmente para de descer numa
            // subárvore problemática, mantendo o que já foi calculado até
            // ali.
            match ler_tabela_diretorio(leitor, descritor, setor, tamanho) {
                Ok(sub) => tabela.entradas[indice].subdiretorio = Some(sub),
                Err(e) => {
                    eprintln!(
                        "aviso: falha ao ler subdiretório '{}' (setor {setor}): {e}",
                        tabela.entradas[indice].nome
                    );
                    continue;
                }
            }
        }
        let sub = tabela.entradas[indice].subdiretorio.as_mut().unwrap();
        processar_diretorio(sub, leitor, descritor, ultimo_setor, profundidade + 1)?;
    }

    Ok(())
}

/// Navega pela árvore de diretórios seguindo os componentes do caminho
/// (separados por `\`), carregando subdiretórios sob demanda. Replica
/// fielmente `GDF.GetFolder`, incluindo uma particularidade do original:
/// quando o *último* componente do caminho corresponde a um diretório (não a
/// um arquivo), a função retorna a subtabela desse diretório — não a tabela
/// que o contém. Isso é o esperado para localizar um arquivo (o chamador
/// então procura pelo nome do arquivo dentro da tabela retornada), mas
/// significa que `existe()` não confirma corretamente a existência de
/// diretórios por nome. Como a ferramenta original só usa isso para
/// localizar arquivos (`default.xex`/`default.xbe`), mantemos o
/// comportamento idêntico em vez de "corrigi-lo".
fn obter_pasta<'a>(
    tabela: &'a mut TabelaDiretorio,
    caminho: &str,
    leitor: &mut BufReader<File>,
    descritor: &DescritorVolume,
    profundidade: u32,
) -> Resultado<Option<&'a mut TabelaDiretorio>> {
    if caminho.is_empty() {
        return Ok(Some(tabela));
    }
    if profundidade > PROFUNDIDADE_MAXIMA {
        return Err(diretorio::erro_profundidade_excedida());
    }

    let (primeiro, resto) = match caminho.split_once('\\') {
        Some((a, b)) => (a, Some(b)),
        None => (caminho, None),
    };

    let Some(indice) = tabela
        .entradas
        .iter()
        .position(|e| e.nome.eq_ignore_ascii_case(primeiro))
    else {
        return Ok(None);
    };

    if !tabela.entradas[indice].eh_diretorio() {
        return Ok(Some(tabela));
    }

    if tabela.entradas[indice].subdiretorio.is_none() {
        let (setor, tamanho) = {
            let entrada = &tabela.entradas[indice];
            (entrada.setor, entrada.tamanho)
        };
        let sub = ler_tabela_diretorio(leitor, descritor, setor, tamanho)?;
        tabela.entradas[indice].subdiretorio = Some(sub);
    }
    let sub = tabela.entradas[indice].subdiretorio.as_mut().unwrap();

    match resto {
        None => Ok(Some(sub)),
        Some(resto) => obter_pasta(sub, resto, leitor, descritor, profundidade + 1),
    }
}

fn erro_nao_e_imagem_xbox() -> String {
    "não parece uma imagem de Xbox/Xbox 360: a assinatura \"MICROSOFT*XBOX*MEDIA\" não foi \
     encontrada em nenhum dos deslocamentos conhecidos (Xsf/XGD1/XGD2/XGD3)"
        .to_string()
}

/// Tenta ler exatamente 20 bytes na posição absoluta informada e compara com
/// a assinatura GDF. Um fim de arquivo prematuro é tratado como "não
/// encontrada" em vez de erro — mesmo comportamento de `BinaryReader.ReadBytes`
/// no original, que devolve um array truncado (e portanto nunca igual) ao
/// invés de lançar exceção.
fn assinatura_presente(leitor: &mut BufReader<File>, posicao: u64) -> Resultado<bool> {
    leitor.seek(SeekFrom::Start(posicao))?;
    let mut buffer = [0u8; 20];
    match leitor.read_exact(&mut buffer) {
        Ok(()) => Ok(&buffer == ASSINATURA_XBOX_MEDIA),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Determina XGD1/XGD2/XGD3/Xsf testando a assinatura em cada um dos
/// deslocamentos conhecidos, na mesma ordem do `GDF::readVolume()` original.
///
/// Nota: assim como no original, XGD3 é o resultado padrão quando nenhuma das
/// outras três assinaturas é encontrada — não há uma verificação positiva
/// para XGD3 em si. Quem confere isso é `ler_descritor_volume`, que valida a
/// assinatura no deslocamento escolhido antes de aceitar o descritor (ver
/// lá): sem essa checagem, um arquivo que não é uma imagem de Xbox passaria
/// por "XGD3" e só falharia bem mais adiante, com uma mensagem que não
/// explica nada.
/// Devolve também se a assinatura foi de fato encontrada: `false` significa
/// que caímos no palpite XGD3 sem confirmação nenhuma, e quem chama usa isso
/// para dar uma mensagem honesta caso o descritor não confira.
fn detectar_tipo(leitor: &mut BufReader<File>) -> Resultado<(TipoIso, bool)> {
    let base = SETOR_ASSINATURA * TAMANHO_SETOR;

    if assinatura_presente(leitor, base + TipoIso::Xsf.deslocamento_raiz())? {
        Ok((TipoIso::Xsf, true))
    } else if assinatura_presente(leitor, base + TipoIso::Xgd1.deslocamento_raiz())? {
        Ok((TipoIso::Xgd1, true))
    } else if assinatura_presente(leitor, base + TipoIso::Xgd2.deslocamento_raiz())? {
        Ok((TipoIso::Xgd2, true))
    } else {
        Ok((TipoIso::Xgd3, false))
    }
}

/// Lê os campos fixos do descritor de volume (identificador, setor/tamanho do
/// diretório raiz, timestamp de criação) a partir do deslocamento do tipo já
/// detectado, e calcula o tamanho/setores do volume a partir do tamanho do
/// arquivo. Equivalente à segunda metade de `GDF::readVolume()`.
fn ler_descritor_volume(
    leitor: &mut BufReader<File>,
    tipo: TipoIso,
    tamanho_arquivo: u64,
    assinatura_confirmada: bool,
) -> Resultado<DescritorVolume> {
    let deslocamento_raiz = tipo.deslocamento_raiz();
    let posicao = SETOR_ASSINATURA * TAMANHO_SETOR + deslocamento_raiz;

    leitor.seek(SeekFrom::Start(posicao))?;
    let mut bloco = [0u8; TAMANHO_DESCRITOR_VOLUME];
    leitor.read_exact(&mut bloco).map_err(|e| {
        if e.kind() != std::io::ErrorKind::UnexpectedEof {
            return Erro::Io(e);
        }
        // Sem assinatura confirmada, o arquivo acabar antes do descritor não
        // quer dizer "imagem truncada": quer dizer que ele nunca foi uma
        // imagem de Xbox, e a mensagem precisa dizer isso.
        if assinatura_confirmada {
            Erro::IsoInvalida("imagem truncada: acaba antes do descritor de volume GDF".into())
        } else {
            Erro::IsoInvalida(erro_nao_e_imagem_xbox())
        }
    })?;

    // Os 20 primeiros bytes do descritor são a própria assinatura. Para
    // Xsf/XGD1/XGD2 `detectar_tipo` já confirmou que ela está aqui; para
    // XGD3, que é só o palpite padrão, esta é a primeira (e única)
    // verificação — é o que separa "imagem XGD3" de "arquivo qualquer".
    if &bloco[0..20] != ASSINATURA_XBOX_MEDIA {
        return Err(Erro::IsoInvalida(erro_nao_e_imagem_xbox()));
    }

    let identificador = bloco[0..20].to_vec();
    let setor_dir_raiz = u32::from_le_bytes(bloco[20..24].try_into().unwrap());
    let tamanho_dir_raiz = u32::from_le_bytes(bloco[24..28].try_into().unwrap());
    let criacao_imagem = bloco[28..36].to_vec();

    let tamanho_volume = tamanho_arquivo.saturating_sub(deslocamento_raiz);
    let setores_volume = (tamanho_volume / TAMANHO_SETOR) as u32;

    Ok(DescritorVolume {
        identificador,
        setor_dir_raiz,
        tamanho_dir_raiz,
        criacao_imagem,
        tamanho_setor: TAMANHO_SETOR as u32,
        deslocamento_raiz,
        tamanho_volume,
        setores_volume,
    })
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::io::Write;

    /// Regressão: antes desta verificação, qualquer arquivo grande o
    /// bastante era aceito como "XGD3" e a falha só aparecia depois, numa
    /// mensagem sobre tabela de diretório corrompida — sem nunca dizer que
    /// o arquivo simplesmente não é uma imagem de Xbox.
    #[test]
    fn abrir_recusa_arquivo_que_nao_e_imagem_de_xbox() {
        let caminho = std::env::temp_dir().join("iso2god_teste_nao_e_iso.bin");
        let mut arquivo = File::create(&caminho).unwrap();
        arquivo.write_all(&vec![0xCDu8; 128 * 1024]).unwrap();
        drop(arquivo);

        let mensagem = match Gdf::abrir(&caminho) {
            Ok(_) => panic!("um arquivo qualquer não deveria abrir como GDF"),
            Err(e) => e.to_string(),
        };
        assert!(
            mensagem.contains("não parece uma imagem de Xbox"),
            "mensagem pouco clara para arquivo que não é ISO de Xbox: {mensagem}"
        );

        std::fs::remove_file(&caminho).ok();
    }

    /// O outro caminho da mesma proteção: arquivo grande o bastante para
    /// chegar ao deslocamento do XGD3, mas com lixo no lugar da assinatura.
    #[test]
    fn abrir_recusa_arquivo_grande_sem_assinatura_no_lugar_certo() {
        let caminho = std::env::temp_dir().join("iso2god_teste_grande_sem_assinatura.bin");
        let arquivo = File::create(&caminho).unwrap();
        // esparso: não ocupa 34 MB de verdade no disco
        arquivo.set_len(SETOR_ASSINATURA * TAMANHO_SETOR + 34_078_720 + 65_536).unwrap();
        drop(arquivo);

        let mensagem = match Gdf::abrir(&caminho) {
            Ok(_) => panic!("arquivo sem assinatura não deveria abrir como GDF"),
            Err(e) => e.to_string(),
        };
        assert!(
            mensagem.contains("não parece uma imagem de Xbox"),
            "mensagem pouco clara: {mensagem}"
        );

        std::fs::remove_file(&caminho).ok();
    }

    /// Regressão: uma entrada com setor e tamanho no limite do u32 estourava
    /// a soma em `processar_diretorio` (pânico em debug; em release, um
    /// valor embaralhado que podia parecer plausível). Agora satura, e a
    /// validação contra o tamanho do volume recusa a imagem.
    /// Regressão: um tamanho declarado de 4 GiB fazia o programa pedir 4 GiB
    /// ao sistema antes de qualquer verificação. Em máquina folgada isso
    /// passa (páginas zeradas nunca tocadas), mas sob limite de memória a
    /// alocação falha e o Rust **aborta** o processo — sem erro tratável e
    /// sem limpeza. Medido: `memory allocation of 4294967295 bytes failed`,
    /// exit 134 com core dump.
    #[test]
    fn tamanho_declarado_alem_do_volume_e_recusado_antes_de_alocar() {
        let caminho = std::env::temp_dir().join("iso2god_teste_alocacao.iso");
        let mut dados = vec![0u8; (SETOR_ASSINATURA * TAMANHO_SETOR) as usize + 400 * TAMANHO_SETOR as usize];
        let base = (SETOR_ASSINATURA * TAMANHO_SETOR) as usize;
        dados[base..base + 20].copy_from_slice(ASSINATURA_XBOX_MEDIA);
        dados[base + 20..base + 24].copy_from_slice(&100u32.to_le_bytes());
        // tabela de diretório raiz declarando 4 GiB num volume de 800 KiB
        dados[base + 24..base + 28].copy_from_slice(&u32::MAX.to_le_bytes());
        std::fs::write(&caminho, &dados).unwrap();

        // `abrir` tolera raiz ilegível (só avisa), então a checagem aparece aqui
        let gdf = Gdf::abrir(&caminho).expect("a imagem em si é válida");
        assert!(gdf.raiz.is_none(), "a raiz absurda não pode ter sido lida");

        let erro = ler_tabela_diretorio(
            &mut BufReader::new(File::open(&caminho).unwrap()),
            &gdf.descritor,
            100,
            u32::MAX,
        );
        let mensagem = match erro {
            Ok(_) => panic!("uma tabela de 4 GiB não cabe num volume de 800 KiB"),
            Err(e) => e.to_string(),
        };
        assert!(
            mensagem.contains("além do fim do volume"),
            "mensagem pouco clara: {mensagem}"
        );

        std::fs::remove_file(&caminho).ok();
    }

    #[test]
    fn entrada_no_limite_do_u32_satura_em_vez_de_estourar() {
        let caminho = std::env::temp_dir().join("iso2god_teste_setor_limite.iso");

        let mut entrada = Vec::new();
        entrada.extend_from_slice(&0u16.to_le_bytes());
        entrada.extend_from_slice(&1u16.to_le_bytes());
        entrada.extend_from_slice(&u32::MAX.to_le_bytes()); // setor
        entrada.extend_from_slice(&u32::MAX.to_le_bytes()); // tamanho
        entrada.push(0x00);
        entrada.push(b"limite.bin".len() as u8);
        entrada.extend_from_slice(b"limite.bin");
        while !entrada.len().is_multiple_of(4) {
            entrada.push(0xFF);
        }
        let mut raiz = entrada;
        raiz.resize(TAMANHO_SETOR as usize, 0xFF);

        let mut dados = vec![0u8; (SETOR_ASSINATURA * TAMANHO_SETOR) as usize + 400 * TAMANHO_SETOR as usize];
        let base = (SETOR_ASSINATURA * TAMANHO_SETOR) as usize;
        dados[base..base + 20].copy_from_slice(ASSINATURA_XBOX_MEDIA);
        dados[base + 20..base + 24].copy_from_slice(&100u32.to_le_bytes());
        dados[base + 24..base + 28].copy_from_slice(&(raiz.len() as u32).to_le_bytes());
        let inicio_raiz = 100 * TAMANHO_SETOR as usize;
        dados[inicio_raiz..inicio_raiz + raiz.len()].copy_from_slice(&raiz);
        std::fs::write(&caminho, &dados).unwrap();

        let mut gdf = Gdf::abrir(&caminho).expect("a imagem em si é válida");
        let ultimo = gdf.analisar_diretorios().expect("a travessia não pode entrar em pânico");
        assert_eq!(ultimo, u32::MAX, "a soma deveria saturar, não dar a volta");
        assert!(
            gdf.validar_ultimo_setor(ultimo).is_err(),
            "um último setor saturado não cabe no volume e tem que ser recusado"
        );

        std::fs::remove_file(&caminho).ok();
    }

    #[test]
    fn tipo_da_imagem_reconhece_e_recusa_sem_barulho() {
        let valida = std::env::temp_dir().join("iso2god_teste_tipo_valida.iso");
        let mut dados = vec![0u8; 128 * 1024];
        let base = (SETOR_ASSINATURA * TAMANHO_SETOR) as usize;
        dados[base..base + 20].copy_from_slice(ASSINATURA_XBOX_MEDIA);
        std::fs::write(&valida, &dados).unwrap();
        assert_eq!(tipo_da_imagem(&valida), Some(volume::TipoIso::Xsf));

        let qualquer = std::env::temp_dir().join("iso2god_teste_tipo_qualquer.bin");
        std::fs::write(&qualquer, vec![0xCDu8; 128 * 1024]).unwrap();
        assert_eq!(tipo_da_imagem(&qualquer), None);

        assert_eq!(tipo_da_imagem(Path::new("/nao/existe/mesmo.iso")), None);

        std::fs::remove_file(&valida).ok();
        std::fs::remove_file(&qualquer).ok();
    }

    #[test]
    fn abrir_aceita_imagem_com_assinatura_valida() {
        let caminho = std::env::temp_dir().join("iso2god_teste_assinatura_ok.iso");
        let mut dados = vec![0u8; 128 * 1024];
        let base = (SETOR_ASSINATURA * TAMANHO_SETOR) as usize; // Xsf: deslocamento raiz 0
        dados[base..base + 20].copy_from_slice(ASSINATURA_XBOX_MEDIA);
        std::fs::write(&caminho, &dados).unwrap();

        let gdf = Gdf::abrir(&caminho).expect("imagem com assinatura deveria abrir");
        assert_eq!(gdf.tipo, volume::TipoIso::Xsf);

        std::fs::remove_file(&caminho).ok();
    }
}
