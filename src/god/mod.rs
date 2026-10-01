pub mod cabecalho;
pub mod hashtable;
mod reconstrucao;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use cabecalho::{EscritorCabecalho, TipoConteudo};
use hashtable::{MasterHashTable, SubHashTable};

use crate::cli::{Plataforma, RemocaoPadding};
use crate::erro::{Erro, Resultado};
use crate::gdf::Gdf;
use crate::progresso::Reporter;
use crate::sistema;
use crate::terminal::fmt_bytes;
use crate::xbe;
use crate::xex;

/// Teto de threads de leitura/hash. Cada thread mantém dois descritores de
/// arquivo abertos (origem e parte de saída), então um `-j 5000` distraído
/// esbarraria no limite de descritores do processo no meio da conversão. O
/// teto é alto o bastante para qualquer máquina real e transforma um número
/// absurdo em aviso, não em falha.
pub const MAX_THREADS: usize = 64;

/// Máximo de blocos de 4096 bytes de dados por arquivo "Part" (Data0000,
/// Data0001, ...). 203 Sub Hash Tables * 204 blocos cada.
pub const BLOCOS_POR_PARTE: u32 = 41_412;

/// Tamanho de uma "Part" completamente cheia, em blocos de 4096 bytes,
/// contando a MHT (1 bloco) e as 203 Sub Hash Tables (1 bloco cada) além dos
/// 41412 blocos de dados: 1 + 203 + 41412 = 41616. Usado só para estimar o
/// tamanho total das partes gravado no cabeçalho — mesmo valor "mágico" do
/// original (`Iso2God.cs`, variável local sem nome próprio).
const BLOCOS_POR_PARTE_CHEIA: u64 = 41_616;

/// Parâmetros necessários para converter uma ISO em um container GOD.
/// Equivalente, em parte, a `Chilano.Iso2God.IsoEntry` +
/// `Chilano.Iso2God.IsoEntryID`. Os campos de identificação são opcionais:
/// quando não informados, `resolver_metadados` tenta detectá-los
/// automaticamente a partir do `default.xex` (Xbox 360) ou `default.xbe`
/// (Xbox original) antes de exigir que o usuário os informe manualmente.
#[derive(Debug, Clone)]
pub struct OpcoesConversao {
    pub origem: PathBuf,
    pub destino: PathBuf,
    pub padding: RemocaoPadding,
    pub numero_disco: bool,
    pub plataforma: Option<Plataforma>,
    pub title_id: Option<String>,
    pub media_id: Option<String>,
    pub titulo: Option<String>,
    pub disco: Option<u8>,
    pub total_discos: Option<u8>,
    pub plataforma_byte: Option<u8>,
    pub tipo_executavel_byte: Option<u8>,
    pub icone: Option<PathBuf>,
    /// Quantas threads usar para ler e calcular os hashes SHA1 dos blocos de
    /// dados durante a conversão. `0` significa "detectar automaticamente a
    /// quantidade de núcleos disponíveis" (ver `resolver_threads`).
    pub threads: usize,
    /// Se `true`, o progresso é relatado como uma linha JSON por evento em
    /// stdout (ver `crate::progresso`), em vez da barra de terminal colorida
    /// de sempre. Pensado para consumo por outro programa (ex: um script).
    pub progresso_json: bool,
}

/// Metadados do jogo já resolvidos — mesclando o que foi informado
/// explicitamente na CLI com o que `resolver_metadados` conseguiu detectar
/// automaticamente —, prontos para gravar no cabeçalho LIVE.
#[derive(Debug, Clone)]
pub struct MetadadosJogo {
    pub tipo_conteudo: TipoConteudo,
    pub title_id: String,
    pub media_id: String,
    pub titulo: String,
    pub disco: u8,
    pub total_discos: u8,
    pub plataforma_byte: u8,
    pub tipo_executavel_byte: u8,
    /// Bytes já em PNG do thumbnail do jogo, se algum foi informado
    /// manualmente ou detectado automaticamente.
    pub icone: Option<Vec<u8>>,
}

/// Orquestra a conversão completa de uma ISO para um container GOD.
pub fn converter(opcoes: &OpcoesConversao) -> Resultado<()> {
    let mut gdf = Gdf::abrir(&opcoes.origem)?;

    match opcoes.padding {
        RemocaoPadding::Nenhuma | RemocaoPadding::Parcial => {
            let metadados = resolver_metadados(opcoes, &mut gdf)?;
            converter_parcial(opcoes, &metadados, &mut gdf)
        }
        RemocaoPadding::Completa => converter_completa(opcoes, &mut gdf),
    }
}

/// Determina a plataforma/tipo de conteúdo e os metadados de identificação
/// do jogo, priorizando o que foi informado explicitamente na CLI e caindo
/// para detecção automática via `default.xex` (equivalente a
/// `IsoDetails.readXex`/`readXbe` + a checagem `iso.Exists("default.xex")`
/// de `IsoDetails_DoWork`) quando algo não foi informado.
fn resolver_metadados(opcoes: &OpcoesConversao, gdf: &mut Gdf) -> Resultado<MetadadosJogo> {
    let tipo_conteudo = match opcoes.plataforma {
        Some(Plataforma::Xbox) => TipoConteudo::XboxOriginal,
        Some(Plataforma::Xbox360) => TipoConteudo::GamesOnDemand,
        None if gdf.existe("default.xex") => TipoConteudo::GamesOnDemand,
        None if gdf.existe("default.xbe") => TipoConteudo::XboxOriginal,
        None => {
            return Err(Erro::IsoInvalida(
                "não foi possível determinar a plataforma automaticamente (a imagem não contém \
                 default.xex nem default.xbe); informe --plataforma manualmente"
                    .into(),
            ));
        }
    };

    let detectado = match tipo_conteudo {
        TipoConteudo::GamesOnDemand => detectar_do_xex(gdf),
        TipoConteudo::XboxOriginal => detectar_do_xbe(gdf),
    };

    let title_id = opcoes
        .title_id
        .clone()
        .or(detectado.title_id_hex)
        .ok_or_else(|| {
            Erro::IsoInvalida(
                "não foi possível determinar o Title ID automaticamente; informe --title-id".into(),
            )
        })?;

    let media_id = opcoes
        .media_id
        .clone()
        .or(detectado.media_id_hex)
        .ok_or_else(|| {
            Erro::IsoInvalida(
                "não foi possível determinar o Media ID automaticamente; informe --media-id".into(),
            )
        })?;

    // Validados agora, não na gravação do cabeçalho (que é a última etapa):
    // um ID com tamanho errado só apareceria depois da conversão inteira já
    // ter acontecido — e, antes desta checagem, nem aparecia: preenchia
    // parte do campo e produzia um pacote com identidade errada em silêncio.
    cabecalho::validar_id("o Title ID", &title_id)?;
    cabecalho::validar_id("o Media ID", &media_id)?;

    let disco = opcoes.disco.or(detectado.disco_numero).unwrap_or(1);
    let total_discos = opcoes.total_discos.or(detectado.disco_total).unwrap_or(1);
    let plataforma_byte = opcoes
        .plataforma_byte
        .or(detectado.plataforma_byte)
        .unwrap_or(0);
    let tipo_executavel_byte = opcoes
        .tipo_executavel_byte
        .or(detectado.tipo_executavel_byte)
        .unwrap_or(0);

    let titulo = opcoes
        .titulo
        .clone()
        .or(detectado.titulo)
        .unwrap_or_else(|| nome_a_partir_do_arquivo(&opcoes.origem));

    // O ícone informado na CLI é validado agora, antes de qualquer escrita:
    // ele só seria usado no cabeçalho, que é a ÚLTIMA etapa da conversão —
    // descobrir lá que o PNG não cabe significaria jogar fora uma conversão
    // inteira já concluída.
    let icone = match opcoes.icone.as_deref() {
        Some(caminho) => {
            let dados = fs::read(caminho).map_err(|e| {
                Erro::IsoInvalida(format!(
                    "não foi possível ler o ícone '{}': {e}",
                    caminho.display()
                ))
            })?;
            if dados.len() > cabecalho::MAX_ICONE {
                return Err(Erro::IsoInvalida(format!(
                    "o ícone '{}' tem {} e o campo de thumbnail do cabeçalho comporta no \
                     máximo {}; use uma imagem menor",
                    caminho.display(),
                    fmt_bytes(dados.len() as u64),
                    fmt_bytes(cabecalho::MAX_ICONE as u64)
                )));
            }
            Some(dados)
        }
        None => detectado.icone,
    };

    Ok(MetadadosJogo {
        tipo_conteudo,
        title_id,
        media_id,
        titulo,
        disco,
        total_discos,
        plataforma_byte,
        tipo_executavel_byte,
        icone,
    })
}

/// Metadados que foi possível detectar automaticamente a partir do
/// executável padrão da imagem (`default.xex` ou `default.xbe`). Qualquer
/// campo pode ficar `None` se não pôde ser lido/interpretado — nesse caso
/// `resolver_metadados` cai para o que foi informado na CLI, ou erra se
/// nada estiver disponível.
#[derive(Debug, Default)]
struct MetadadosDetectados {
    title_id_hex: Option<String>,
    media_id_hex: Option<String>,
    titulo: Option<String>,
    disco_numero: Option<u8>,
    disco_total: Option<u8>,
    plataforma_byte: Option<u8>,
    tipo_executavel_byte: Option<u8>,
    icone: Option<Vec<u8>>,
}

/// Lê e interpreta o `default.xex`, tolerando qualquer falha (arquivo
/// ilegível, cabeçalho inesperado, etc.) devolvendo campos em falta como
/// `None` em vez de abortar a conversão. Equivalente a `IsoDetails.readXex`:
/// Title ID, Media ID e disco vêm do cabeçalho do XEX; o nome de exibição e o
/// ícone, do recurso XDBF dentro da imagem (ver `xex::ler_titulo`).
fn detectar_do_xex(gdf: &mut Gdf) -> MetadadosDetectados {
    let bytes = match gdf.ler_arquivo("default.xex") {
        Ok(bytes) => bytes,
        Err(e) => {
            saida_erro!("aviso: não foi possível ler o default.xex da imagem: {e}");
            return MetadadosDetectados::default();
        }
    };
    let info = match xex::ler_info_execucao(&bytes) {
        Ok(info) => info,
        Err(e) => {
            saida_erro!("aviso: não foi possível interpretar o default.xex: {e}");
            return MetadadosDetectados::default();
        }
    };

    // Nome e ícone são cortesia: sem eles a conversão segue com o nome do
    // arquivo e sem ícone, como antes.
    let (titulo, icone) = match xex::ler_titulo(&bytes) {
        Ok(t) => {
            let icone = match t.icone_png {
                Some(png) if png.len() > cabecalho::MAX_ICONE => {
                    saida_erro!(
                        "aviso: o ícone do jogo tem {} e não cabe no cabeçalho (máximo {}); \
                         seguindo sem ícone",
                        fmt_bytes(png.len() as u64),
                        fmt_bytes(cabecalho::MAX_ICONE as u64)
                    );
                    None
                }
                outro => outro,
            };
            (t.titulo, icone)
        }
        Err(e) => {
            saida_erro!("aviso: não foi possível ler o nome e o ícone do jogo: {e}");
            (None, None)
        }
    };

    MetadadosDetectados {
        title_id_hex: Some(info.title_id_hex()),
        media_id_hex: Some(info.media_id_hex()),
        titulo,
        disco_numero: Some(info.disco_numero),
        disco_total: Some(info.disco_total),
        plataforma_byte: Some(info.plataforma),
        tipo_executavel_byte: Some(info.tipo_executavel),
        icone,
    }
}

/// Lê e interpreta o `default.xbe` (Xbox original), tolerando qualquer
/// falha da mesma forma que `detectar_do_xex`. Diferente do XEX, o
/// certificado do XBE já traz o nome de exibição embutido; e como discos de
/// Xbox original não têm um Media ID nativo, usamos um substituto derivado
/// do MD5 do arquivo inteiro (mesma solução do original). Equivalente a
/// `IsoDetails.readXbe` + o construtor de 4 argumentos de
/// `IsoDetailsResults`.
fn detectar_do_xbe(gdf: &mut Gdf) -> MetadadosDetectados {
    let bytes = match gdf.ler_arquivo("default.xbe") {
        Ok(bytes) => bytes,
        Err(e) => {
            saida_erro!("aviso: não foi possível ler o default.xbe da imagem: {e}");
            return MetadadosDetectados::default();
        }
    };
    let info = match xbe::ler_info_certificado(&bytes) {
        Ok(info) => info,
        Err(e) => {
            saida_erro!("aviso: não foi possível interpretar o default.xbe: {e}");
            return MetadadosDetectados::default();
        }
    };

    // Mesma regra do original: DiskNumber 0 vira "disco 1"; DiscCount não
    // existe nesse caminho e é sempre fixado em 1.
    let disco_numero = if info.disco_numero == 0 {
        1
    } else {
        info.disco_numero as u8
    };

    let icone = match xbe::extrair_thumbnail(&bytes) {
        // Thumbnails de XBE são pequenos (64x64), mas um arquivo estranho
        // poderia render um PNG maior que o campo do cabeçalho: nesse caso
        // seguimos sem ícone em vez de derrubar a conversão inteira.
        Ok(png) if png.len() > cabecalho::MAX_ICONE => {
            saida_erro!(
                "aviso: o thumbnail extraído do default.xbe tem {} e não cabe no cabeçalho \
                 (máximo {}); seguindo sem ícone",
                fmt_bytes(png.len() as u64),
                fmt_bytes(cabecalho::MAX_ICONE as u64)
            );
            None
        }
        Ok(png) => Some(png),
        Err(e) => {
            saida_erro!("aviso: não foi possível extrair o thumbnail do default.xbe: {e}");
            None
        }
    };

    MetadadosDetectados {
        title_id_hex: Some(info.title_id_hex()),
        media_id_hex: Some(xbe::media_id_substituto(&bytes)),
        titulo: Some(info.titulo).filter(|t| !t.is_empty()),
        disco_numero: Some(disco_numero),
        disco_total: Some(1),
        plataforma_byte: Some(0),
        tipo_executavel_byte: Some(0),
        icone,
    }
}

/// Deriva um título de exibição a partir do nome do arquivo da ISO (sem
/// extensão), usado quando `--titulo` não é informado e o nome de exibição
/// do próprio jogo (certificado do XBE ou XDBF do XEX) não pôde ser lido.
fn nome_a_partir_do_arquivo(origem: &Path) -> String {
    origem
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Jogo".to_string())
}

/// Conversão sem reconstrução da GDF: fatia o ISO original (já aberto e
/// analisado em `gdf`) em blocos de 4096 bytes, calcula as Sub/Master Hash
/// Tables, encadeia as partes e grava o cabeçalho LIVE. Equivalente a
/// `Iso2God.Iso2God_Partial` + `writeParts` + `calcMhtHashChain` +
/// `createConHeader`.
fn converter_parcial(
    opcoes: &OpcoesConversao,
    metadados: &MetadadosJogo,
    gdf: &mut Gdf,
) -> Resultado<()> {
    let inicio = Instant::now();

    let num_bytes = calcular_bytes_a_converter(opcoes, gdf)?;
    let blocos_necessarios: u32 = num_bytes
        .div_ceil(hashtable::TAMANHO_BLOCO as u64)
        .try_into()
        .map_err(|_| Erro::IsoInvalida("imagem grande demais para converter".into()))?;

    // Sem este corte, `escrever_partes` calcularia `blocos_nesta_parte - 1`
    // com zero: em debug isso é um pânico de subtração, e em release o `u32`
    // dá a volta e o programa tenta criar uma "parte" de vários terabytes,
    // falhando com um `File too large` que não diz nada ao usuário.
    if blocos_necessarios == 0 {
        return Err(Erro::IsoInvalida(
            "não há nada para converter: a análise não encontrou nenhum setor ocupado na \
             imagem (ela pode estar vazia, truncada ou com a estrutura GDF corrompida)"
                .into(),
        ));
    }

    let partes_necessarias = blocos_necessarios.div_ceil(BLOCOS_POR_PARTE).max(1);

    let tipo_conteudo = metadados.tipo_conteudo;

    let nome = nome_unico(
        &metadados.title_id,
        &metadados.media_id,
        metadados.disco,
        metadados.total_discos,
    );
    // Layout que o console procura: <TitleID>/<tipo de conteúdo>/<pacote>.
    // Sem o nível do Title ID, copiar a pasta para o console não encontra
    // nada — quem converte teria que criar esse diretório à mão.
    let pasta_conteudo = opcoes
        .destino
        .join(metadados.title_id.to_uppercase())
        .join(format!("{:08X}", tipo_conteudo.valor()));
    let pasta_dados = pasta_conteudo.join(format!("{nome}.data"));
    let arquivo_cabecalho = pasta_conteudo.join(&nome);

    let reporter = Reporter::novo(blocos_necessarios, opcoes.progresso_json);

    let threads = resolver_threads(opcoes.threads);
    if opcoes.threads > threads {
        reporter.aviso(&format!(
            "{} threads é mais do que faz sentido abrir de uma vez; usando {threads}",
            opcoes.threads
        ));
    }

    // Antes de criar qualquer arquivo: cabe? Descobrir isso agora custa uma
    // chamada de sistema; descobrir no meio custa a conversão inteira.
    verificar_espaco(&opcoes.destino, tamanho_estimado_saida(blocos_necessarios))?;

    fs::create_dir_all(&pasta_conteudo)?;
    // O cabeçalho é o que faz o pacote aparecer no console. O de uma
    // conversão anterior do mesmo jogo seria sobrescrito só no fim; até lá
    // ele ficava ao lado das partes novas pela metade, e um processo morto
    // no meio (SIGKILL, queda de energia) deixava um pacote com cara de
    // pronto e partes faltando. Sai antes de qualquer parte ser tocada.
    match fs::remove_file(&arquivo_cabecalho) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    if pasta_dados.exists() {
        reporter.aviso(&format!(
            "diretório de saída já existe, substituindo: {}",
            pasta_dados.display()
        ));
        fs::remove_dir_all(&pasta_dados)?;
    }
    fs::create_dir_all(&pasta_dados)?;

    // A partir daqui já existe saída no disco: qualquer falha (inclusive um
    // Ctrl+C) precisa passar pela limpeza, senão fica para trás uma pasta
    // com cara de pacote GOD pronto que na verdade está pela metade.
    let resultado = (|| -> Resultado<()> {
        reporter.fase("convertendo", "Convertendo ISO para GOD...");

        escrever_partes(
            &opcoes.origem,
            gdf.descritor.deslocamento_raiz,
            &pasta_dados,
            partes_necessarias,
            blocos_necessarios,
            threads,
            &reporter,
        )?;

        reporter.fim_dos_blocos("Blocos de dados gravados.");

        reporter.fase(
            "calculando_hash",
            "Calculando cadeia de hash entre as partes...",
        );
        let (tamanho_ultima_parte, hash_mht_final) =
            calcular_cadeia_mht(&pasta_dados, partes_necessarias)?;

        let tamanho_parte_completa = hashtable::TAMANHO_BLOCO as u64 * BLOCOS_POR_PARTE_CHEIA;
        let tamanho_partes =
            tamanho_ultima_parte + (partes_necessarias as u64 - 1) * tamanho_parte_completa;

        reporter.fase("gravando_cabecalho", "Gravando cabeçalho LIVE...");
        gravar_cabecalho(
            metadados,
            opcoes.numero_disco,
            &arquivo_cabecalho,
            blocos_necessarios,
            partes_necessarias,
            tamanho_partes,
            &hash_mht_final,
            &reporter,
        )
    })();

    if resultado.is_err() {
        descartar_saida_incompleta(&opcoes.destino, &pasta_dados, &arquivo_cabecalho, &reporter);
        return resultado;
    }

    let duracao = inicio.elapsed();
    reporter.concluido(
        &pasta_conteudo.to_string_lossy(),
        duracao.as_secs_f64(),
        &format!(
            "Concluído em {}m{}s. Pacote GOD gravado em: {}",
            duracao.as_secs() / 60,
            duracao.as_secs() % 60,
            pasta_conteudo.display()
        ),
    );

    Ok(())
}

/// Quanto o pacote GOD vai ocupar no destino para uma conversão de
/// `bytes_convertidos` bytes de dados. Exposto para o assistente poder
/// mostrar "necessário x livre" no resumo, antes de o usuário confirmar.
pub fn tamanho_estimado_do_pacote(bytes_convertidos: u64) -> u64 {
    let blocos = bytes_convertidos.div_ceil(hashtable::TAMANHO_BLOCO as u64);
    tamanho_estimado_saida(blocos.try_into().unwrap_or(u32::MAX))
}

/// Tamanho exato que a conversão vai ocupar no destino: a soma do tamanho de
/// cada "Part" (calculado do mesmo jeito que `escrever_partes` faz ao criar
/// o arquivo) mais o cabeçalho LIVE. Serve para a checagem de espaço livre.
fn tamanho_estimado_saida(blocos_necessarios: u32) -> u64 {
    let mut restantes = blocos_necessarios;
    let mut total = cabecalho::TAMANHO_CABECALHO_LIVE as u64;
    while restantes > 0 {
        let blocos_nesta_parte = restantes.min(BLOCOS_POR_PARTE);
        total += offset_bloco_na_parte(blocos_nesta_parte - 1) + hashtable::TAMANHO_BLOCO as u64;
        restantes -= blocos_nesta_parte;
    }
    total
}

/// Recusa a conversão quando o destino não tem espaço para ela. Se não for
/// possível descobrir o espaço livre (`espaco_livre` devolve `None`), segue
/// em frente: a checagem é uma cortesia, não um portão.
fn verificar_espaco(destino: &Path, necessario: u64) -> Resultado<()> {
    let Some(disponivel) = sistema::espaco_livre(destino) else {
        return Ok(());
    };
    if disponivel >= necessario {
        return Ok(());
    }
    Err(Erro::EspacoInsuficiente {
        destino: destino.display().to_string(),
        necessario: fmt_bytes(necessario),
        disponivel: fmt_bytes(disponivel),
    })
}

/// Apaga a saída pela metade depois de uma falha ou cancelamento. Uma pasta
/// `.data` incompleta é indistinguível de uma pronta para quem só olha o
/// gerenciador de arquivos — é pior do que não ter nada.
fn descartar_saida_incompleta(
    destino: &Path,
    pasta_dados: &Path,
    arquivo_cabecalho: &Path,
    reporter: &Reporter,
) {
    reporter.interromper_barra();
    let falhou_ao_limpar = fs::remove_dir_all(pasta_dados).is_err() && pasta_dados.exists();
    fs::remove_file(arquivo_cabecalho).ok();

    // Sobe apagando os diretórios que esta conversão criou e que ficaram
    // vazios (<TitleID>/<tipo>), sem nunca tocar no destino escolhido pelo
    // usuário — `remove_dir` falha em diretório não-vazio, então uma
    // conversão anterior no mesmo destino nunca é levada junto.
    let mut atual = arquivo_cabecalho.parent();
    while let Some(pasta) = atual {
        if pasta == destino || fs::remove_dir(pasta).is_err() {
            break;
        }
        atual = pasta.parent();
    }
    if falhou_ao_limpar {
        reporter.aviso(&format!(
            "não foi possível apagar a saída incompleta em {} — remova essa pasta antes de \
             usar o pacote",
            pasta_dados.display()
        ));
    } else {
        reporter.aviso("saída incompleta descartada.");
    }
}

/// Reconstrói a estrutura GDF do zero (removendo todo o padding, não só o
/// do final) numa ISO temporária, e então converte esse resultado para GOD
/// pelo mesmo caminho de `converter_parcial` — sem padding restante para
/// remover, já que a reconstrução elimina tudo, então equivale a usar
/// `RemocaoPadding::Nenhuma` sobre a ISO reconstruída. Equivalente a
/// `Iso2God.Iso2God_Full`, que ao final chama `Iso2God_Partial` sobre a ISO
/// recém-reconstruída (com `Options.Padding` ainda em `Full`, que
/// `Iso2God_Partial` trata da mesma forma que `None`: usa o volume inteiro).
fn converter_completa(opcoes: &OpcoesConversao, gdf: &mut Gdf) -> Resultado<()> {
    crate::progresso::anunciar_fase(
        "reconstruindo_gdf",
        "Reconstruindo a estrutura GDF (removendo todo o padding)...",
        opcoes.progresso_json,
    );

    let metadados = resolver_metadados(opcoes, gdf)?;

    // Quanto os dados úteis ocupam hoje: é o teto tanto para a ISO
    // reconstruída (que só encolhe) quanto para a base do pacote GOD. As
    // duas coisas convivem no destino ao mesmo tempo, então o espaço
    // necessário é a soma.
    let ultimo_setor = gdf.analisar_diretorios()?;
    gdf.validar_ultimo_setor(ultimo_setor)?;
    let bytes_uteis = ultimo_setor as u64 * gdf.descritor.tamanho_setor as u64;
    let blocos = bytes_uteis.div_ceil(hashtable::TAMANHO_BLOCO as u64);
    let necessario = bytes_uteis + tamanho_estimado_saida(blocos.try_into().unwrap_or(u32::MAX));
    verificar_espaco(&opcoes.destino, necessario)?;

    let nome_base = opcoes
        .origem
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "imagem".to_string());

    // A ISO reconstruída fica no próprio destino, não em `std::env::temp_dir()`:
    // em muitas distribuições /tmp é tmpfs (RAM), e uma imagem de vários GB
    // ali dentro estoura a memória; mesmo quando /tmp é disco, gravar GB no
    // disco interno enquanto o destino é um HD externo é surpresa ruim. O PID
    // no nome evita que duas conversões simultâneas escrevam no mesmo arquivo.
    fs::create_dir_all(&opcoes.destino)?;
    let caminho_reconstruida = opcoes.destino.join(format!(
        ".iso2god-{}-{nome_base}.reconstruida.iso",
        std::process::id()
    ));

    if let Err(e) = reconstrucao::reconstruir(gdf, &opcoes.origem, &caminho_reconstruida) {
        fs::remove_file(&caminho_reconstruida).ok();
        return Err(e);
    }

    crate::progresso::anunciar_fase(
        "gdf_reconstruida",
        "Estrutura reconstruída, convertendo para GOD...",
        opcoes.progresso_json,
    );

    let resultado = (|| -> Resultado<()> {
        let mut gdf_reconstruida = Gdf::abrir(&caminho_reconstruida)?;
        let mut opcoes_reconstruida = opcoes.clone();
        opcoes_reconstruida.origem = caminho_reconstruida.clone();
        opcoes_reconstruida.padding = RemocaoPadding::Nenhuma;
        converter_parcial(&opcoes_reconstruida, &metadados, &mut gdf_reconstruida)
    })();

    fs::remove_file(&caminho_reconstruida).ok();

    resultado
}

/// Determina quantos bytes do volume devem ser convertidos, de acordo com a
/// estratégia de remoção de padding. Equivalente ao cálculo de `num` em
/// `Iso2God_Partial`.
fn calcular_bytes_a_converter(opcoes: &OpcoesConversao, gdf: &mut Gdf) -> Resultado<u64> {
    match opcoes.padding {
        RemocaoPadding::Parcial => {
            let ultimo_setor = gdf.analisar_diretorios()?;
            gdf.validar_ultimo_setor(ultimo_setor)?;
            Ok(ultimo_setor as u64 * gdf.descritor.tamanho_setor as u64)
        }
        RemocaoPadding::Nenhuma => Ok(gdf.descritor.tamanho_volume),
        RemocaoPadding::Completa => Err(Erro::NaoImplementado(
            "converter_parcial não deve ser chamado com RemocaoPadding::Completa",
        )),
    }
}

fn caminho_parte(pasta_dados: &Path, indice: u32) -> PathBuf {
    pasta_dados.join(format!("Data{indice:04}"))
}

/// Quantos blocos de dados cabem numa Sub Hash Table.
const BLOCOS_POR_SHT: u32 = 204;

/// Deslocamento, dentro do arquivo de uma "Part", de onde começa a Sub Hash
/// Table de índice `indice_sht` (0-based).
fn offset_sht_na_parte(indice_sht: u32) -> u64 {
    hashtable::TAMANHO_TABELA as u64
        + indice_sht as u64
            * (hashtable::TAMANHO_TABELA as u64
                + BLOCOS_POR_SHT as u64 * hashtable::TAMANHO_BLOCO as u64)
}

/// Deslocamento, dentro do arquivo de uma "Part", de onde começa o bloco de
/// dados de índice `indice_local` (0-based, relativo ao início da parte).
fn offset_bloco_na_parte(indice_local: u32) -> u64 {
    let indice_sht = indice_local / BLOCOS_POR_SHT;
    let posicao_no_sht = indice_local % BLOCOS_POR_SHT;
    offset_sht_na_parte(indice_sht)
        + hashtable::TAMANHO_TABELA as u64
        + posicao_no_sht as u64 * hashtable::TAMANHO_BLOCO as u64
}

/// Resolve quantas threads usar de verdade: `0` significa "detectar
/// automaticamente" (via `std::thread::available_parallelism`, com 1 como
/// fallback se a detecção falhar); qualquer outro valor é usado como está.
/// Resolve quantas threads usar de verdade e limita o resultado a
/// `MAX_THREADS` (ver a constante).
fn resolver_threads(threads: usize) -> usize {
    let alvo = if threads == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    } else {
        threads
    };
    alvo.clamp(1, MAX_THREADS)
}

/// Lê o ISO de origem em blocos de 4096 bytes, gravando cada "Part" no
/// formato [MHT][SHT_0][204 blocos][SHT_1][204 blocos].... Dentro de cada
/// parte, a leitura+hash dos blocos de dados é dividida entre até `threads`
/// threads (cada uma com seu próprio descritor de arquivo, escrevendo
/// diretamente na posição final de cada bloco — sem placeholder/reescrita,
/// já que o tamanho final do arquivo já é conhecido de antemão); a
/// montagem das Sub/Master Hash Tables a partir dos hashes já calculados
/// continua sequencial (é rápida, e a ordem importa para o encadeamento).
/// Equivalente a `Iso2God.writeParts`, mas paralelizado.
fn escrever_partes(
    caminho_origem: &Path,
    deslocamento_raiz: u64,
    pasta_dados: &Path,
    partes_necessarias: u32,
    blocos_necessarios: u32,
    threads: usize,
    reporter: &Reporter,
) -> Resultado<()> {
    let threads = threads.max(1);
    let mut blocos_processados: u32 = 0;

    for indice_parte in 0..partes_necessarias {
        if sistema::cancelado() {
            return Err(Erro::Cancelado);
        }
        reporter.detalhe_bloco(&format!(
            "Escrevendo parte {} / {partes_necessarias}...",
            indice_parte + 1
        ));

        let blocos_nesta_parte = (blocos_necessarios - blocos_processados).min(BLOCOS_POR_PARTE);
        // Rede de segurança: `converter_parcial` já recusa uma conversão de
        // zero blocos, e sem blocos nesta parte não haveria arquivo a criar.
        if blocos_nesta_parte == 0 {
            break;
        }
        let shts_nesta_parte = (blocos_nesta_parte as u64).div_ceil(BLOCOS_POR_SHT as u64) as u32;
        // Não dá para usar `offset_sht_na_parte(shts_nesta_parte)` aqui: essa
        // função assume que todas as SHTs anteriores estão cheias (204
        // blocos), mas a ÚLTIMA SHT desta parte normalmente tem menos blocos
        // que isso. O tamanho real é o fim do último bloco de dados de
        // verdade.
        let tamanho_parte =
            offset_bloco_na_parte(blocos_nesta_parte - 1) + hashtable::TAMANHO_BLOCO as u64;

        let caminho_parte_atual = caminho_parte(pasta_dados, indice_parte);
        {
            let arquivo = File::create(&caminho_parte_atual)?;
            arquivo.set_len(tamanho_parte)?;
        }

        let hashes = ler_e_hashear_paralelo(
            caminho_origem,
            deslocamento_raiz,
            &caminho_parte_atual,
            blocos_processados,
            blocos_nesta_parte,
            threads,
            reporter,
        )?;

        montar_e_escrever_hashtables(&caminho_parte_atual, &hashes, shts_nesta_parte)?;

        blocos_processados += blocos_nesta_parte;
    }

    Ok(())
}

/// Divide os `blocos_nesta_parte` blocos desta parte em até `threads` faixas
/// contíguas, processa cada uma em uma thread separada (cada qual com seu
/// próprio descritor de arquivo de origem e de destino) e devolve os hashes
/// SHA1 de cada bloco, em ordem. Cada thread grava diretamente os bytes do
/// bloco na posição final dentro do arquivo da parte — só os hashes voltam
/// para a thread principal, que monta as hash tables depois.
fn ler_e_hashear_paralelo(
    caminho_origem: &Path,
    deslocamento_raiz: u64,
    caminho_parte: &Path,
    indice_global_inicial: u32,
    blocos_nesta_parte: u32,
    threads: usize,
    reporter: &Reporter,
) -> Resultado<Vec<[u8; 20]>> {
    let threads = threads.min(blocos_nesta_parte.max(1) as usize).max(1);
    let (tx, rx) = mpsc::channel::<Resultado<(u32, [u8; 20])>>();

    std::thread::scope(|escopo| -> Resultado<Vec<[u8; 20]>> {
        for id_thread in 0..threads {
            let tx = tx.clone();
            let inicio = (blocos_nesta_parte as usize * id_thread / threads) as u32;
            let fim = (blocos_nesta_parte as usize * (id_thread + 1) / threads) as u32;
            if inicio >= fim {
                continue;
            }

            escopo.spawn(move || {
                if let Err(e) = processar_faixa(
                    caminho_origem,
                    deslocamento_raiz,
                    caminho_parte,
                    indice_global_inicial,
                    inicio,
                    fim,
                    &tx,
                ) {
                    let _ = tx.send(Err(e));
                }
            });
        }
        drop(tx);

        let mut hashes: Vec<Option<[u8; 20]>> = vec![None; blocos_nesta_parte as usize];
        let mut erro = None;
        for mensagem in rx {
            match mensagem {
                Ok((indice_local, hash)) => {
                    hashes[indice_local as usize] = Some(hash);
                    reporter.inc(1);
                }
                Err(e) => {
                    erro.get_or_insert(e);
                }
            };
        }
        if let Some(e) = erro {
            return Err(e);
        }

        // a esta altura, nenhuma thread falhou, então todo índice foi preenchido
        Ok(hashes.into_iter().map(|h| h.unwrap()).collect())
    })
}

/// Lê, hasheia e grava (na posição final, já calculada) os blocos de dados
/// no intervalo local `[inicio, fim)` desta parte, usando descritores de
/// arquivo próprios (independentes das outras threads). Equivalente, para a
/// sua faixa, ao laço interno de `Iso2God.writeParts`.
fn processar_faixa(
    caminho_origem: &Path,
    deslocamento_raiz: u64,
    caminho_parte: &Path,
    indice_global_inicial: u32,
    inicio_local: u32,
    fim_local: u32,
    tx: &mpsc::Sender<Resultado<(u32, [u8; 20])>>,
) -> Resultado<()> {
    let mut origem = File::open(caminho_origem)?;
    let mut saida = OpenOptions::new().write(true).open(caminho_parte)?;
    let tamanho_bloco = hashtable::TAMANHO_BLOCO as usize;
    let mut buffer = vec![0u8; BLOCOS_POR_SHT as usize * tamanho_bloco];

    // Os blocos de um mesmo grupo (os até 204 que vêm depois de uma Sub Hash
    // Table) são contíguos na origem E na parte de saída: lê e grava o grupo
    // inteiro de uma vez, em vez de um seek+read+seek+write por bloco de
    // 4 KiB. Mesmo resultado byte a byte, com ~200x menos chamadas ao sistema.
    let mut indice_local = inicio_local;
    while indice_local < fim_local {
        // Consultado a cada grupo (no máximo 816 KiB de I/O): é o que permite
        // ao Ctrl+C parar num ponto conhecido em vez de matar o processo no
        // meio de uma escrita.
        if sistema::cancelado() {
            return Err(Erro::Cancelado);
        }
        let fim_do_grupo = (indice_local / BLOCOS_POR_SHT + 1) * BLOCOS_POR_SHT;
        let n = (fim_do_grupo.min(fim_local) - indice_local) as usize;
        let trecho = &mut buffer[..n * tamanho_bloco];

        let indice_global = indice_global_inicial + indice_local;
        origem.seek(SeekFrom::Start(
            deslocamento_raiz + indice_global as u64 * hashtable::TAMANHO_BLOCO as u64,
        ))?;
        ler_bloco(&mut origem, trecho)?;

        saida.seek(SeekFrom::Start(offset_bloco_na_parte(indice_local)))?;
        saida.write_all(trecho)?;

        for (k, bloco) in trecho.chunks_exact(tamanho_bloco).enumerate() {
            tx.send(Ok((indice_local + k as u32, hashtable::sha1(bloco))))
                .ok();
        }
        indice_local += n as u32;
    }

    Ok(())
}

/// Monta as Sub Hash Tables e a Master Hash Table a partir dos hashes já
/// calculados (em ordem) e grava tudo nas posições certas dentro do arquivo
/// da parte. Etapa sequencial (rápida — é só serialização), feita depois
/// que a leitura/hash em paralelo termina.
fn montar_e_escrever_hashtables(
    caminho_parte: &Path,
    hashes: &[[u8; 20]],
    shts_nesta_parte: u32,
) -> Resultado<()> {
    let mut arquivo = OpenOptions::new().write(true).open(caminho_parte)?;
    let mut mht = MasterHashTable::nova();

    for indice_sht in 0..shts_nesta_parte {
        let inicio = (indice_sht * BLOCOS_POR_SHT) as usize;
        let fim = hashes.len().min(inicio + BLOCOS_POR_SHT as usize);

        let mut sht = SubHashTable::nova();
        for hash in &hashes[inicio..fim] {
            sht.adicionar(*hash)?;
        }

        arquivo.seek(SeekFrom::Start(offset_sht_na_parte(indice_sht)))?;
        arquivo.write_all(&sht.para_bytes())?;

        mht.adicionar(hashtable::sha1(&sht.para_bytes()))?;
    }

    arquivo.seek(SeekFrom::Start(0))?;
    arquivo.write_all(&mht.para_bytes())?;

    Ok(())
}

/// Enche `buffer` (um ou mais blocos de 4096 bytes) lendo de `origem` (já
/// posicionado). Se o arquivo acabar antes de preencher o buffer todo — só
/// acontece nos últimos blocos físicos do arquivo —, o restante é preenchido
/// com zero, replicando o comportamento
/// do original (que sempre aloca um array novo, já zerado, e ignora quantos
/// bytes o `Read` efetivamente devolveu).
fn ler_bloco(origem: &mut File, buffer: &mut [u8]) -> Resultado<()> {
    let mut lidos = 0usize;
    loop {
        match origem.read(&mut buffer[lidos..])? {
            0 => break,
            n => lidos += n,
        }
        if lidos == buffer.len() {
            break;
        }
    }
    if lidos < buffer.len() {
        buffer[lidos..].fill(0);
    }
    Ok(())
}

/// Lê a Master Hash Table já gravada no início de uma "Part".
fn ler_mht_da_parte(caminho: &Path) -> Resultado<MasterHashTable> {
    let mut arquivo = File::open(caminho)?;
    let mut buffer = [0u8; hashtable::TAMANHO_TABELA];
    arquivo.read_exact(&mut buffer)?;
    Ok(MasterHashTable::ler(&buffer))
}

/// Sobrescreve a Master Hash Table no início de uma "Part" já existente.
fn escrever_mht_na_parte(caminho: &Path, mht: &MasterHashTable) -> Resultado<()> {
    let mut arquivo = OpenOptions::new().write(true).open(caminho)?;
    arquivo.write_all(&mht.para_bytes())?;
    Ok(())
}

/// Encadeia as Master Hash Tables de todas as partes: para cada parte, da
/// última até a segunda, calcula o hash da sua MHT e o adiciona como mais
/// uma entrada na MHT da parte anterior (reescrevendo-a em disco). O hash da
/// MHT final da parte 0 (já com a cadeia completa) é o que vai para o campo
/// `MhtHash` do cabeçalho LIVE. Também devolve o tamanho em bytes da última
/// parte, usado para calcular o tamanho total gravado no cabeçalho.
///
/// Equivalente a `Iso2God.calcMhtHashChain` — com uma correção: o original
/// só calcula `lastPartSize` dentro do laço de encadeamento, que nunca
/// executa quando há apenas uma única parte, deixando esse valor em 0 (um
/// bug real do original para ISOs pequenas o bastante para caber em uma
/// parte só). Aqui `lastPartSize` é sempre obtido diretamente do tamanho do
/// arquivo, independente de haver ou não encadeamento a fazer.
fn calcular_cadeia_mht(pasta_dados: &Path, partes_totais: u32) -> Resultado<(u64, [u8; 20])> {
    let caminho_ultima = caminho_parte(pasta_dados, partes_totais - 1);
    let tamanho_ultima_parte = fs::metadata(&caminho_ultima)?.len();

    if partes_totais == 1 {
        let mht = ler_mht_da_parte(&caminho_ultima)?;
        let hash_final = hashtable::sha1(&mht.para_bytes());
        return Ok((tamanho_ultima_parte, hash_final));
    }

    let mut hash_final = [0u8; 20];

    for indice in (1..partes_totais).rev() {
        let mht_atual = ler_mht_da_parte(&caminho_parte(pasta_dados, indice))?;
        let hash_mht_atual = hashtable::sha1(&mht_atual.para_bytes());

        let caminho_anterior = caminho_parte(pasta_dados, indice - 1);
        let mut mht_anterior = ler_mht_da_parte(&caminho_anterior)?;
        mht_anterior.adicionar(hash_mht_atual)?;
        escrever_mht_na_parte(&caminho_anterior, &mht_anterior)?;

        if indice - 1 == 0 {
            hash_final = hashtable::sha1(&mht_anterior.para_bytes());
        }
    }

    Ok((tamanho_ultima_parte, hash_final))
}

/// Monta e grava o cabeçalho LIVE final, reunindo os metadados fornecidos
/// pelo usuário com os valores calculados durante a conversão (contagem de
/// blocos, informação das partes e hash da cadeia de MHTs). Equivalente a
/// `Iso2God.createConHeader`.
#[allow(clippy::too_many_arguments)]
fn gravar_cabecalho(
    metadados: &MetadadosJogo,
    numero_disco: bool,
    caminho: &Path,
    blocos_alocados: u32,
    total_partes: u32,
    tamanho_partes: u64,
    hash_mht: &[u8; 20],
    reporter: &Reporter,
) -> Resultado<()> {
    let titulo = if numero_disco && metadados.total_discos > 1 {
        format!("{} - Disc {}", metadados.titulo, metadados.disco)
    } else {
        metadados.titulo.clone()
    };

    // O campo de título do cabeçalho tem tamanho fixo; `escrever_ids` corta o
    // que não cabe (senão o excesso sobrescreveria os campos seguintes), mas
    // o usuário merece saber que o nome que vai aparecer no console não é o
    // que ele informou.
    let (titulo, cortado) = cabecalho::ajustar_titulo(&titulo);
    if cortado {
        reporter.aviso(&format!(
            "título maior que os {} caracteres do campo do cabeçalho; gravando como \"{titulo}\"",
            cabecalho::MAX_TITULO_UTF16
        ));
    }

    let mut cabecalho = EscritorCabecalho::novo();
    cabecalho.escrever_ids(&metadados.title_id, &metadados.media_id, &titulo)?;
    cabecalho.escrever_detalhes_execucao(
        metadados.disco,
        metadados.total_discos,
        metadados.plataforma_byte,
        metadados.tipo_executavel_byte,
    )?;
    cabecalho.escrever_contagem_blocos(blocos_alocados, 0)?;
    cabecalho.escrever_info_partes(total_partes, tamanho_partes)?;
    cabecalho.escrever_icone(metadados.icone.as_deref())?;
    cabecalho.escrever_tipo_conteudo(metadados.tipo_conteudo)?;
    cabecalho.escrever_hash_mht(hash_mht)?;
    cabecalho.gravar(caminho)?;

    Ok(())
}

/// Gera o nome único da pasta ".data" a partir do Title ID/Media ID/disco,
/// via SHA1 sobre os mesmos bytes que o `BinaryWriter.Write(string)` do .NET
/// produziria (um prefixo de tamanho em "7-bit encoded int" seguido dos
/// bytes UTF-8) — usa só os primeiros 10 bytes do hash de 20, em
/// maiúsculas. Equivalente a `Iso2God.createUniqueName`.
fn nome_unico(title_id: &str, media_id: &str, disco: u8, total_discos: u8) -> String {
    let mut dados = Vec::new();
    escrever_string_estilo_dotnet(&mut dados, title_id);
    escrever_string_estilo_dotnet(&mut dados, media_id);
    dados.push(disco);
    dados.push(total_discos);

    let hash = hashtable::sha1(&dados);
    hash[..10].iter().map(|b| format!("{b:02X}")).collect()
}

/// Replica `System.IO.BinaryWriter.Write(string)`: um prefixo de tamanho
/// "7-bit encoded int" (little-endian, 7 bits de dado por byte, bit mais
/// alto indica continuação) seguido dos bytes UTF-8 da string.
fn escrever_string_estilo_dotnet(saida: &mut Vec<u8>, texto: &str) {
    let bytes = texto.as_bytes();
    let mut tamanho = bytes.len() as u32;
    loop {
        let mut b = (tamanho & 0x7F) as u8;
        tamanho >>= 7;
        if tamanho != 0 {
            b |= 0x80;
            saida.push(b);
        } else {
            saida.push(b);
            break;
        }
    }
    saida.extend_from_slice(bytes);
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Valores de referência calculados independentemente em Python
    /// (`hashlib.sha1`), replicando à mão o algoritmo do
    /// `BinaryWriter.Write(string)` do .NET, para não depender só do próprio
    /// código Rust se auto-validando.
    #[test]
    fn nome_unico_bate_com_valores_calculados_independentemente() {
        assert_eq!(
            nome_unico("4D5308BF", "AABBCCDD", 1, 1),
            "FE40C7D9CF2D599EB911"
        );
        assert_eq!(
            nome_unico("4D5308BF", "AABBCCDD", 2, 2),
            "87BFEED63C1A6ABD041C"
        );
    }

    #[test]
    fn nome_unico_muda_com_disco_e_total_discos() {
        let a = nome_unico("4D5308BF", "AABBCCDD", 1, 2);
        let b = nome_unico("4D5308BF", "AABBCCDD", 2, 2);
        assert_ne!(a, b);
    }

    #[test]
    fn prefixo_de_tamanho_curto_e_um_byte() {
        let mut saida = Vec::new();
        escrever_string_estilo_dotnet(&mut saida, "abc");
        assert_eq!(saida, vec![3, b'a', b'b', b'c']);
    }

    #[test]
    fn prefixo_de_tamanho_longo_usa_dois_bytes() {
        let texto = "x".repeat(200);
        let mut saida = Vec::new();
        escrever_string_estilo_dotnet(&mut saida, &texto);
        // 200 = 0b1100_1000 -> primeiro byte = (200 & 0x7F) | 0x80 = 0xC8,
        // segundo byte = 200 >> 7 = 1. Confirmado independentemente em Python.
        assert_eq!(&saida[0..2], &[0xC8, 0x01]);
        assert_eq!(saida.len(), 2 + 200);
    }

    #[test]
    fn caminho_parte_usa_quatro_digitos_com_zeros_a_esquerda() {
        let base = Path::new("/tmp/x");
        assert_eq!(caminho_parte(base, 0), base.join("Data0000"));
        assert_eq!(caminho_parte(base, 42), base.join("Data0042"));
        assert_eq!(caminho_parte(base, 12345), base.join("Data12345"));
    }

    #[test]
    fn ler_bloco_preenche_com_zero_apos_fim_do_arquivo() {
        let caminho = std::env::temp_dir().join("iso2god_teste_ler_bloco.bin");
        fs::write(&caminho, [0xAA; 10]).unwrap();

        let mut arquivo = File::open(&caminho).unwrap();
        let mut buffer = vec![0xFFu8; 16];
        ler_bloco(&mut arquivo, &mut buffer).unwrap();

        assert_eq!(&buffer[0..10], &[0xAA; 10]);
        assert_eq!(&buffer[10..16], &[0u8; 6]);

        fs::remove_file(&caminho).ok();
    }

    /// Testa `calcular_cadeia_mht` diretamente sobre "Parts" fabricadas à
    /// mão (sem passar pelos ~170MB que uma segunda parte real exigiria),
    /// cobrindo tanto o caso de uma única parte quanto o encadeamento entre
    /// duas partes.
    #[test]
    fn calcular_cadeia_mht_com_uma_unica_parte() {
        let pasta = std::env::temp_dir().join("iso2god_teste_cadeia_1parte");
        fs::create_dir_all(&pasta).unwrap();

        let mut mht = MasterHashTable::nova();
        mht.adicionar([7u8; 20]).unwrap();
        fs::write(caminho_parte(&pasta, 0), mht.para_bytes()).unwrap();
        // conteúdo extra após a MHT, simulando blocos de dados reais
        let mut arquivo = OpenOptions::new()
            .append(true)
            .open(caminho_parte(&pasta, 0))
            .unwrap();
        arquivo.write_all(&[1, 2, 3, 4]).unwrap();
        drop(arquivo);

        let (tamanho, hash) = calcular_cadeia_mht(&pasta, 1).unwrap();
        assert_eq!(tamanho, hashtable::TAMANHO_TABELA as u64 + 4);
        assert_eq!(hash, hashtable::sha1(&mht.para_bytes()));

        fs::remove_dir_all(&pasta).ok();
    }

    #[test]
    fn calcular_cadeia_mht_encadeia_duas_partes() {
        let pasta = std::env::temp_dir().join("iso2god_teste_cadeia_2partes");
        fs::create_dir_all(&pasta).unwrap();

        let mut mht_parte0 = MasterHashTable::nova();
        mht_parte0.adicionar([1u8; 20]).unwrap();
        fs::write(caminho_parte(&pasta, 0), mht_parte0.para_bytes()).unwrap();

        let mut mht_parte1 = MasterHashTable::nova();
        mht_parte1.adicionar([2u8; 20]).unwrap();
        fs::write(caminho_parte(&pasta, 1), mht_parte1.para_bytes()).unwrap();

        let (_tamanho, hash) = calcular_cadeia_mht(&pasta, 2).unwrap();

        // O hash final deve ser o da MHT da parte 0 DEPOIS de receber, como
        // entrada extra, o hash da MHT (original, não-modificada) da parte 1.
        let hash_mht1 = hashtable::sha1(&mht_parte1.para_bytes());
        let mut mht_parte0_esperada = mht_parte0.clone();
        mht_parte0_esperada.adicionar(hash_mht1).unwrap();
        let hash_esperado = hashtable::sha1(&mht_parte0_esperada.para_bytes());

        assert_eq!(hash, hash_esperado);

        // A parte 0 em disco deve realmente ter sido reescrita com a entrada extra.
        let mht_parte0_em_disco = ler_mht_da_parte(&caminho_parte(&pasta, 0)).unwrap();
        assert_eq!(mht_parte0_em_disco.len(), 2);

        fs::remove_dir_all(&pasta).ok();
    }
}
