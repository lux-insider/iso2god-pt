//! Reconstrução completa da estrutura GDF, removendo todo o padding (não só
//! o do final da imagem). Equivalente a `Iso2God.Iso2God_Full` +
//! `RemapSectors` + `WriteGDF` + `WriteFiles`.
//!
//! A ideia central: a árvore de diretórios já lida (`Gdf::analisar_diretorios`)
//! é clonada; nessa cópia, cada diretório e arquivo recebe um novo setor
//! sequencial (sem os buracos de padding do disco original); a árvore
//! *original* (com os setores antigos) continua servindo só para saber de
//! onde ler os bytes de cada arquivo. As duas árvores têm exatamente a
//! mesma forma — só os campos `setor`/`tamanho` da cópia mudam — então dá
//! para percorrer as duas em conjunto, casando entradas pelo nome.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::erro::{Contexto, Erro, Operacao, Resultado};
use crate::gdf::Gdf;
use crate::gdf::diretorio::TabelaDiretorio;

/// Tamanho de setor da GDF (fixo, não confundir com o "bloco" de 4096 bytes
/// do container GOD).
const TAMANHO_SETOR: u64 = 2048;

/// Primeiro setor livre para alocar diretórios/arquivos remapeados — os
/// setores anteriores ficam reservados para a área de segurança e o
/// cabeçalho da GDF reconstruída. Mesmo valor "mágico" do original
/// (`Iso2God.freeSector`).
const SETOR_INICIAL_LIVRE: u32 = 36;

/// Assinatura escrita no início de toda GDF reconstruída: "XSF" + 0x1A,
/// como `u32` little-endian. A reconstrução sempre produz uma imagem no
/// layout "Xsf" (deslocamento raiz 0) — faz sentido, já que o objetivo é
/// eliminar justamente o deslocamento de padding do XGD1/2/3 original.
const ASSINATURA_XSF: u32 = 0x1A46_5358;

/// Primary Volume Descriptor ISO9660 fixo, gravado sempre idêntico no
/// setor 16 (offset 32768) de toda GDF reconstruída. Não depende da imagem
/// de origem — extraído byte a byte do array `gdf_sector` do
/// `Iso2God.cs` original (não é usado pelo leitor de GDF/XDVDFS do Xbox
/// 360 em si; é só uma camada de compatibilidade com ferramentas genéricas
/// de CD/DVD que tentem ler a imagem como ISO9660 comum).
const PVD_ISO9660: &[u8; 2055] = include_bytes!("pvd_iso9660.bin");

/// Um arquivo aberto e o seu caminho: uma falha de E/S diz em qual arquivo
/// foi.
struct Aberto<'a> {
    arquivo: File,
    caminho: &'a Path,
}

impl Aberto<'_> {
    fn ir(&mut self, posicao: u64, operacao: Operacao) -> Resultado<()> {
        self.arquivo
            .seek(SeekFrom::Start(posicao))
            .ctx(operacao, self.caminho)?;
        Ok(())
    }

    fn gravar(&mut self, dados: &[u8]) -> Resultado<()> {
        self.arquivo
            .write_all(dados)
            .ctx(Operacao::Gravar, self.caminho)
    }

    fn ler_exato(&mut self, dados: &mut [u8]) -> Resultado<()> {
        self.arquivo
            .read_exact(dados)
            .ctx(Operacao::Ler, self.caminho)
    }
}

/// Reconstrói a GDF inteira (sem nenhum padding) a partir de `gdf` (já
/// aberta) lendo os dados brutos de `origem`, e grava o resultado em
/// `destino`. Equivalente a `RemapSectors` + `WriteGDF` + `WriteFiles`.
pub fn reconstruir(gdf: &mut Gdf, origem: &Path, destino: &Path) -> Resultado<()> {
    gdf.analisar_diretorios()?;

    let raiz_original = gdf.raiz.clone().ok_or(Erro::GdfNaoEncontrada)?;
    let mut raiz_nova = raiz_original.clone();

    let mut proximo_setor = SETOR_INICIAL_LIVRE;
    remapear_raiz(&mut raiz_nova, &mut proximo_setor);

    let mut saida = Aberto {
        arquivo: File::create(destino).ctx(Operacao::Criar, destino)?,
        caminho: destino,
    };
    escrever_cabecalho(
        &mut saida,
        &raiz_nova,
        &gdf.descritor.identificador,
        &gdf.descritor.criacao_imagem,
    )?;
    escrever_tabelas(&mut saida, &raiz_nova)?;

    let mut arquivo_origem = Aberto {
        arquivo: File::open(origem).ctx(Operacao::Abrir, origem)?,
        caminho: origem,
    };
    escrever_arquivos(
        &mut saida,
        &mut arquivo_origem,
        gdf.descritor.deslocamento_raiz,
        &raiz_original,
        &raiz_nova,
    )?;

    escrever_tamanhos_finais(&mut saida)?;

    Ok(())
}

fn tamanho_para_setores(tamanho: u32) -> u32 {
    (tamanho as u64).div_ceil(TAMANHO_SETOR) as u32
}

/// Atribui um novo setor ao diretório raiz (arredondando seu tamanho para
/// um múltiplo de setor, igual ao original) e dispara o remapeamento do
/// resto da árvore: primeiro todos os diretórios, depois todos os arquivos.
/// Equivalente a `RemapSectors`.
fn remapear_raiz(raiz: &mut TabelaDiretorio, proximo_setor: &mut u32) {
    raiz.setor = *proximo_setor;
    let setores = tamanho_para_setores(raiz.tamanho);
    raiz.tamanho = setores * TAMANHO_SETOR as u32;
    *proximo_setor += setores;

    remapear_diretorios(raiz, proximo_setor);
    remapear_arquivos(raiz, proximo_setor);
}

/// Percorre a árvore em pré-ordem, atribuindo um novo setor sequencial a
/// cada *diretório* (não a arquivos). Equivalente a `Iso2God.remapDirs`.
fn remapear_diretorios(tabela: &mut TabelaDiretorio, proximo_setor: &mut u32) {
    for entrada in tabela.entradas.iter_mut() {
        if !entrada.eh_diretorio() {
            continue;
        }
        let Some(sub) = entrada.subdiretorio.as_mut() else {
            entrada.setor = 0;
            entrada.tamanho = 0;
            continue;
        };
        entrada.setor = *proximo_setor;
        sub.setor = *proximo_setor;
        *proximo_setor += tamanho_para_setores(entrada.tamanho);
        remapear_diretorios(sub, proximo_setor);
    }
}

/// Atribui um novo setor sequencial a cada *arquivo* — primeiro todos os
/// arquivos diretamente dentro de `tabela`, só depois descendo para as
/// subpastas (mesma ordem do original, embora a ordem em si não afete a
/// correção: cada entrada guarda seu próprio setor). Equivalente a
/// `Iso2God.remapFiles`.
fn remapear_arquivos(tabela: &mut TabelaDiretorio, proximo_setor: &mut u32) {
    for entrada in tabela.entradas.iter_mut() {
        if entrada.eh_diretorio() {
            continue;
        }
        entrada.setor = *proximo_setor;
        *proximo_setor += tamanho_para_setores(entrada.tamanho);
    }
    for entrada in tabela.entradas.iter_mut() {
        if entrada.eh_diretorio()
            && let Some(sub) = entrada.subdiretorio.as_mut()
        {
            remapear_arquivos(sub, proximo_setor);
        }
    }
}

/// Grava os campos fixos do cabeçalho da GDF reconstruída: a assinatura
/// "Xsf", o Primary Volume Descriptor ISO9660 fixo, e o descritor de volume
/// da GDF propriamente dito (identificador e timestamp herdados do disco de
/// origem; setor/tamanho do diretório raiz apontando para o novo local).
/// Equivalente a `Iso2God.writeGDFheader`.
fn escrever_cabecalho(
    saida: &mut Aberto,
    raiz: &TabelaDiretorio,
    identificador: &[u8],
    criacao_imagem: &[u8],
) -> Resultado<()> {
    saida.ir(0, Operacao::Gravar)?;
    saida.gravar(&ASSINATURA_XSF.to_le_bytes())?;
    saida.gravar(&1024u32.to_le_bytes())?;

    saida.ir(32768, Operacao::Gravar)?;
    saida.gravar(PVD_ISO9660)?;

    saida.ir(65536, Operacao::Gravar)?;
    saida.gravar(identificador)?;
    saida.gravar(&raiz.setor.to_le_bytes())?;
    saida.gravar(&raiz.tamanho.to_le_bytes())?;
    saida.gravar(criacao_imagem)?;
    saida.gravar(&[1u8])?;

    saida.ir(67564, Operacao::Gravar)?;
    saida.gravar(identificador)?;

    Ok(())
}

/// Grava recursivamente cada tabela de diretório já remapeada em seu novo
/// setor. Equivalente a `Iso2God.writeGDFtable`.
fn escrever_tabelas(saida: &mut Aberto, tabela: &TabelaDiretorio) -> Resultado<()> {
    saida.ir(tabela.setor as u64 * TAMANHO_SETOR, Operacao::Gravar)?;
    saida.gravar(&tabela.para_bytes()?)?;

    for entrada in &tabela.entradas {
        if entrada.eh_diretorio()
            && let Some(sub) = &entrada.subdiretorio
        {
            escrever_tabelas(saida, sub)?;
        }
    }

    Ok(())
}

/// Copia os dados de cada arquivo do local original (via `tabela_original`,
/// com os setores antigos) para o novo local (via `tabela_nova`, com os
/// setores remapeados), casando as duas árvores por nome em cada nível —
/// elas têm exatamente a mesma forma, só os setores mudaram. Equivalente a
/// `Iso2God.writeFiles`.
fn escrever_arquivos(
    saida: &mut Aberto,
    origem: &mut Aberto,
    deslocamento_raiz_origem: u64,
    tabela_original: &TabelaDiretorio,
    tabela_nova: &TabelaDiretorio,
) -> Resultado<()> {
    for entrada_nova in &tabela_nova.entradas {
        if entrada_nova.eh_diretorio() {
            continue;
        }
        let entrada_original = tabela_original
            .encontrar(&entrada_nova.nome)
            .ok_or_else(|| {
                Erro::IsoInvalida(format!(
                    "entrada '{}' não encontrada na árvore original durante a reconstrução",
                    entrada_nova.nome
                ))
            })?;

        copiar_arquivo(
            saida,
            origem,
            deslocamento_raiz_origem,
            entrada_original.setor,
            entrada_nova.setor,
            entrada_original.tamanho,
        )?;
    }

    for entrada_nova in &tabela_nova.entradas {
        if !entrada_nova.eh_diretorio() {
            continue;
        }
        let Some(sub_nova) = &entrada_nova.subdiretorio else {
            continue;
        };
        let sub_original = tabela_original
            .encontrar(&entrada_nova.nome)
            .and_then(|e| e.subdiretorio.as_ref())
            .ok_or_else(|| {
                Erro::IsoInvalida(format!(
                    "subdiretório '{}' não encontrado na árvore original durante a reconstrução",
                    entrada_nova.nome
                ))
            })?;
        escrever_arquivos(
            saida,
            origem,
            deslocamento_raiz_origem,
            sub_original,
            sub_nova,
        )?;
    }

    Ok(())
}

/// Copia um arquivo do setor de origem para o setor de destino, em blocos
/// de um setor inteiro (2048 bytes) — o último, se parcial, é preenchido
/// com zero até completar o setor. Equivalente a `GDF.WriteFileToStream`.
fn copiar_arquivo(
    saida: &mut Aberto,
    origem: &mut Aberto,
    deslocamento_raiz_origem: u64,
    setor_origem: u32,
    setor_destino: u32,
    tamanho: u32,
) -> Resultado<()> {
    origem.ir(
        deslocamento_raiz_origem + setor_origem as u64 * TAMANHO_SETOR,
        Operacao::Ler,
    )?;
    saida.ir(setor_destino as u64 * TAMANHO_SETOR, Operacao::Gravar)?;

    let setores = tamanho_para_setores(tamanho);
    let mut restante = tamanho as u64;
    let mut buffer = [0u8; TAMANHO_SETOR as usize];

    for _ in 0..setores {
        // Mesma lógica de `god::processar_faixa`: o Ctrl+C precisa conseguir
        // parar uma reconstrução de vários GB num ponto conhecido.
        if crate::sistema::cancelado() {
            return Err(Erro::Cancelado);
        }
        let a_ler = (TAMANHO_SETOR as usize).min(restante as usize);
        origem.ler_exato(&mut buffer[..a_ler])?;
        if a_ler < buffer.len() {
            buffer[a_ler..].fill(0);
        }
        saida.gravar(&buffer)?;
        restante = restante.saturating_sub(a_ler as u64);
    }

    Ok(())
}

/// Grava, ao final, o tamanho total da imagem reconstruída em três lugares
/// do cabeçalho: o tamanho em bytes (menos 1024) logo após a assinatura, e o
/// tamanho em setores duas vezes (little-endian e big-endian). Só dá para
/// calcular depois que todo o resto já foi escrito, por isso é o último
/// passo. Equivalente a `Iso2God.writeGDFsizes`.
fn escrever_tamanhos_finais(saida: &mut Aberto) -> Resultado<()> {
    let tamanho = saida
        .arquivo
        .seek(SeekFrom::End(0))
        .ctx(Operacao::Consultar, saida.caminho)?;

    saida.ir(8, Operacao::Gravar)?;
    saida.gravar(&(tamanho as i64 - 1024).to_le_bytes())?;

    let tamanho_em_setores = (tamanho / TAMANHO_SETOR) as u32;
    saida.ir(32848, Operacao::Gravar)?;
    saida.gravar(&tamanho_em_setores.to_le_bytes())?;
    saida.ir(32852, Operacao::Gravar)?;
    saida.gravar(&tamanho_em_setores.to_be_bytes())?;

    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::gdf::Gdf;
    use crate::gdf::volume::TipoIso;

    const SETOR: u64 = 2048;
    const BASE_ASSINATURA: u64 = 32 * SETOR;
    const DESLOCAMENTO_XGD3: u64 = 34_078_720;
    const ASSINATURA: &[u8] = b"MICROSOFT*XBOX*MEDIA";

    fn montar_entrada(setor: u32, tamanho: u32, attrib: u8, nome: &str) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&setor.to_le_bytes());
        b.extend_from_slice(&tamanho.to_le_bytes());
        b.push(attrib);
        b.push(nome.len() as u8);
        b.extend_from_slice(nome.as_bytes());
        while !(b.len()).is_multiple_of(4) {
            b.push(0xFF);
        }
        b
    }

    fn preencher_ate_setor(b: &mut Vec<u8>) {
        while !(b.len() as u64).is_multiple_of(SETOR) {
            b.push(0xFF);
        }
    }

    /// Cria, no diretório temporário, uma ISO sintética XGD3 com:
    ///   / (raiz, setor 100)
    ///     FILE1.TXT   (arquivo, setor 1000, conteúdo `conteudo_file1`)
    ///     SUBDIR/      (diretório, setor 2000)
    ///       FILE2.TXT (arquivo, setor 3000, conteúdo `conteudo_file2`)
    fn criar_iso_teste(
        nome_arquivo: &str,
        conteudo_file1: &[u8],
        conteudo_file2: &[u8],
    ) -> std::path::PathBuf {
        let entrada_file1 = montar_entrada(1000, conteudo_file1.len() as u32, 0x00, "FILE1.TXT");
        let entrada_subdir = montar_entrada(2000, SETOR as u32, 0x10, "SUBDIR");
        let mut bloco_raiz = Vec::new();
        bloco_raiz.extend(entrada_file1);
        bloco_raiz.extend(entrada_subdir);
        preencher_ate_setor(&mut bloco_raiz);

        let entrada_file2 = montar_entrada(3000, conteudo_file2.len() as u32, 0x00, "FILE2.TXT");
        let mut bloco_subdir = entrada_file2;
        preencher_ate_setor(&mut bloco_subdir);

        let setor_raiz: u32 = 100;
        let tamanho_raiz = bloco_raiz.len() as u32;

        let mut descritor = Vec::new();
        descritor.extend_from_slice(ASSINATURA);
        descritor.extend_from_slice(&setor_raiz.to_le_bytes());
        descritor.extend_from_slice(&tamanho_raiz.to_le_bytes());
        descritor.extend_from_slice(b"TESTDATA");

        let caminho = std::env::temp_dir().join(nome_arquivo);
        let mut f = File::create(&caminho).expect("criar arquivo de teste");
        f.set_len(BASE_ASSINATURA + DESLOCAMENTO_XGD3 + 65536)
            .unwrap();

        f.seek(SeekFrom::Start(BASE_ASSINATURA + DESLOCAMENTO_XGD3))
            .unwrap();
        f.write_all(&descritor).unwrap();

        f.seek(SeekFrom::Start(
            DESLOCAMENTO_XGD3 + setor_raiz as u64 * SETOR,
        ))
        .unwrap();
        f.write_all(&bloco_raiz).unwrap();

        f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3 + 1000 * SETOR))
            .unwrap();
        f.write_all(conteudo_file1).unwrap();

        f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3 + 2000 * SETOR))
            .unwrap();
        f.write_all(&bloco_subdir).unwrap();

        f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3 + 3000 * SETOR))
            .unwrap();
        f.write_all(conteudo_file2).unwrap();

        caminho
    }

    #[test]
    fn reconstruir_produz_gdf_valida_e_legivel_pelo_nosso_proprio_leitor() {
        let conteudo_file1 = b"conteudo do arquivo na raiz".to_vec();
        // conteudo maior que um setor, pra exercitar copiar_arquivo em varios blocos
        let conteudo_file2: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();

        let origem = criar_iso_teste(
            "iso2god_teste_reconstrucao_origem.iso",
            &conteudo_file1,
            &conteudo_file2,
        );
        let destino = std::env::temp_dir().join("iso2god_teste_reconstrucao_destino.iso");

        let mut gdf = Gdf::abrir(&origem).expect("abrir ISO de teste");
        reconstruir(&mut gdf, &origem, &destino).expect("reconstrução deveria ter sucesso");

        let mut gdf_reconstruida = Gdf::abrir(&destino).expect("abrir ISO reconstruída");
        assert_eq!(
            gdf_reconstruida.tipo,
            TipoIso::Xsf,
            "reconstrução deveria sempre usar o layout Xsf"
        );
        assert_eq!(gdf_reconstruida.descritor.deslocamento_raiz, 0);

        assert!(gdf_reconstruida.existe("FILE1.TXT"));
        assert!(gdf_reconstruida.existe("SUBDIR\\FILE2.TXT"));
        assert!(!gdf_reconstruida.existe("NAOEXISTE.TXT"));

        let lido_file1 = gdf_reconstruida.ler_arquivo("FILE1.TXT").unwrap();
        assert_eq!(lido_file1, conteudo_file1);

        let lido_file2 = gdf_reconstruida.ler_arquivo("SUBDIR\\FILE2.TXT").unwrap();
        assert_eq!(lido_file2, conteudo_file2);

        std::fs::remove_file(&origem).ok();
        std::fs::remove_file(&destino).ok();
    }
}
