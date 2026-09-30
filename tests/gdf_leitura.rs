//! Testes de integração para a leitura de estruturas GDF sintéticas,
//! cobrindo detecção de tipo de disco, parsing da árvore de diretórios e
//! navegação sob demanda (`existe`).

use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

const SETOR: u64 = 2048;
const BASE_ASSINATURA: u64 = 32 * SETOR;
const DESLOCAMENTO_XGD3: u64 = 34_078_720;
const ASSINATURA: &[u8] = b"MICROSOFT*XBOX*MEDIA";

/// Monta os bytes de uma entrada de diretório GDF já alinhada a 4 bytes.
fn montar_entrada(
    subtree_l: u16,
    subtree_r: u16,
    setor: u32,
    tamanho: u32,
    attrib: u8,
    nome: &str,
) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&subtree_l.to_le_bytes());
    b.extend_from_slice(&subtree_r.to_le_bytes());
    b.extend_from_slice(&setor.to_le_bytes());
    b.extend_from_slice(&tamanho.to_le_bytes());
    b.push(attrib);
    b.push(nome.len() as u8);
    b.extend_from_slice(nome.as_bytes());
    while b.len() % 4 != 0 {
        b.push(0xFF);
    }
    b
}

fn preencher_ate_setor(b: &mut Vec<u8>) {
    while !(b.len() as u64).is_multiple_of(SETOR) {
        b.push(0xFF);
    }
}

/// Cria, no diretório temporário do sistema, uma imagem GDF sintética do
/// tipo XGD3 com o seguinte layout:
///   / (raiz, setor 100)
///     default.xex          (arquivo, setor 1000, 400000 bytes)
///     SUBDIR/               (diretório, setor 2000)
///       readme.txt          (arquivo, setor 3000, 123 bytes)
fn criar_iso_teste(nome_arquivo: &str) -> PathBuf {
    criar_iso_teste_com_raiz(nome_arquivo, 100)
}

/// O mesmo layout de `criar_iso_teste`, com a tabela da raiz no setor
/// escolhido — inclusive depois de todos os arquivos, como em imagens
/// reconstruídas ou homebrew.
fn criar_iso_teste_com_raiz(nome_arquivo: &str, setor_raiz: u32) -> PathBuf {
    let entrada_xex = montar_entrada(0, 0, 1000, 400_000, 0x00, "default.xex");
    let entrada_subdir = montar_entrada(0, 0, 2000, SETOR as u32, 0x10, "SUBDIR");
    let mut bloco_raiz = Vec::new();
    bloco_raiz.extend(entrada_xex);
    bloco_raiz.extend(entrada_subdir);
    preencher_ate_setor(&mut bloco_raiz);

    let mut bloco_subdir = montar_entrada(0, 0, 3000, 123, 0x00, "readme.txt");
    preencher_ate_setor(&mut bloco_subdir);

    let tamanho_raiz = bloco_raiz.len() as u32;

    let mut descritor = Vec::new();
    descritor.extend_from_slice(ASSINATURA);
    descritor.extend_from_slice(&setor_raiz.to_le_bytes());
    descritor.extend_from_slice(&tamanho_raiz.to_le_bytes());
    descritor.extend_from_slice(b"TESTDATA");

    let caminho = std::env::temp_dir().join(nome_arquivo);
    let mut f = File::create(&caminho).expect("criar arquivo de teste");

    f.set_len(
        (DESLOCAMENTO_XGD3 + (setor_raiz as u64 + 1) * SETOR)
            .max(BASE_ASSINATURA + DESLOCAMENTO_XGD3 + 65536),
    )
    .expect("truncar arquivo de teste");

    f.seek(SeekFrom::Start(BASE_ASSINATURA + DESLOCAMENTO_XGD3))
        .unwrap();
    f.write_all(&descritor).unwrap();

    f.seek(SeekFrom::Start(
        DESLOCAMENTO_XGD3 + setor_raiz as u64 * SETOR,
    ))
    .unwrap();
    f.write_all(&bloco_raiz).unwrap();

    f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3 + 2000 * SETOR))
        .unwrap();
    f.write_all(&bloco_subdir).unwrap();

    caminho
}

use iso2god::gdf;

#[test]
fn detecta_tipo_e_le_descritor() {
    let caminho = criar_iso_teste("iso2god_teste_descritor.iso");
    let g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    assert_eq!(g.descritor.setor_dir_raiz, 100);
    assert_eq!(g.descritor.tamanho_dir_raiz, 2048);
    assert_eq!(g.descritor.deslocamento_raiz, DESLOCAMENTO_XGD3);

    std::fs::remove_file(&caminho).ok();
}

#[test]
fn le_entradas_da_raiz() {
    let caminho = criar_iso_teste("iso2god_teste_raiz.iso");
    let g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    let raiz = g
        .raiz
        .as_ref()
        .expect("diretório raiz deveria ter sido lido");
    assert_eq!(raiz.entradas.len(), 2);
    assert!(raiz.encontrar("default.xex").is_some());
    assert!(
        raiz.encontrar("DEFAULT.XEX").is_some(),
        "busca deve ignorar maiúsculas/minúsculas"
    );
    assert!(raiz.encontrar("subdir").unwrap().eh_diretorio());

    std::fs::remove_file(&caminho).ok();
}

#[test]
fn existe_encontra_arquivo_na_raiz_e_nao_encontra_inexistente() {
    let caminho = criar_iso_teste("iso2god_teste_existe_raiz.iso");
    let mut g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    assert!(g.existe("default.xex"));
    assert!(g.existe("DEFAULT.XEX"));
    assert!(!g.existe("default.xbe"));

    std::fs::remove_file(&caminho).ok();
}

#[test]
fn existe_navega_ate_arquivo_em_subdiretorio() {
    let caminho = criar_iso_teste("iso2god_teste_existe_sub.iso");
    let mut g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    assert!(g.existe("SUBDIR\\readme.txt"));
    assert!(g.existe("subdir\\README.TXT"));
    assert!(!g.existe("SUBDIR\\naoexiste.txt"));
    assert!(!g.existe("NAOEXISTE\\readme.txt"));

    std::fs::remove_file(&caminho).ok();
}

#[test]
fn analisar_diretorios_encontra_ultimo_setor_recursivamente() {
    let caminho = criar_iso_teste("iso2god_teste_ultimo_setor.iso");
    let mut g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    // readme.txt está no setor 3000 com 123 bytes -> ocupa 1 setor -> 3001.
    // É o maior entre todas as entradas (raiz, xex, subdir, readme).
    let ultimo_setor = g.analisar_diretorios().expect("analisar diretórios");
    assert_eq!(ultimo_setor, 3001);

    std::fs::remove_file(&caminho).ok();
}

/// Regressão: a tabela da raiz não é entrada de diretório nenhum, então a
/// varredura não a contava. Com a raiz depois dos arquivos (imagem
/// reconstruída), o corte "parcial" deixava a raiz fora do pacote GOD e o
/// jogo não abria no console — sem erro nenhum na conversão.
#[test]
fn analisar_diretorios_conta_a_tabela_da_raiz_depois_dos_arquivos() {
    let caminho = criar_iso_teste_com_raiz("iso2god_teste_raiz_no_fim.iso", 5000);
    let mut g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    // readme.txt termina no setor 3001; a raiz ocupa o setor 5000 inteiro.
    let ultimo_setor = g.analisar_diretorios().expect("analisar diretórios");
    assert_eq!(ultimo_setor, 5001);
    g.validar_ultimo_setor(ultimo_setor)
        .expect("a raiz cabe no volume");

    std::fs::remove_file(&caminho).ok();
}

/// Regressão: uma ISO real e incomum pode declarar um subdiretório cujo
/// tamanho ultrapassa o fim do arquivo (imagem truncada/corrompida, ou
/// detecção incorreta de XGD1/2/3). Antes desta correção, isso fazia
/// `analisar_diretorios` propagar um erro de E/S cru ("failed to fill whole
/// buffer") e abortar a conversão inteira. Agora deve só pular aquela
/// subárvore (registrando um aviso) e seguir com o que já foi possível
/// calcular — mesma resiliência do `GDF.ParseDirectory` original.
#[test]
fn analisar_diretorios_tolera_subdiretorio_com_tamanho_alem_do_fim_do_arquivo() {
    let entrada_xex = montar_entrada(0, 0, 1000, 400_000, 0x00, "default.xex");
    // Declara um tamanho bem maior do que o arquivo realmente vai ter.
    let entrada_subdir_corrompida = montar_entrada(0, 0, 2000, 50 * SETOR as u32, 0x10, "SUBDIR");
    let mut bloco_raiz = Vec::new();
    bloco_raiz.extend(entrada_xex);
    bloco_raiz.extend(entrada_subdir_corrompida);
    preencher_ate_setor(&mut bloco_raiz);

    let setor_raiz: u32 = 100;
    let tamanho_raiz = bloco_raiz.len() as u32;

    let mut descritor = Vec::new();
    descritor.extend_from_slice(ASSINATURA);
    descritor.extend_from_slice(&setor_raiz.to_le_bytes());
    descritor.extend_from_slice(&tamanho_raiz.to_le_bytes());
    descritor.extend_from_slice(b"TESTDATA");

    let caminho = std::env::temp_dir().join("iso2god_teste_subdir_corrompido.iso");
    let mut f = File::create(&caminho).expect("criar arquivo de teste");

    // Termina o arquivo só 100 bytes depois de onde o SUBDIR começa — bem
    // menos que os 50 setores (102400 bytes) que ele declara ter.
    f.set_len(DESLOCAMENTO_XGD3 + 2000 * SETOR + 100).unwrap();

    f.seek(SeekFrom::Start(BASE_ASSINATURA + DESLOCAMENTO_XGD3))
        .unwrap();
    f.write_all(&descritor).unwrap();

    f.seek(SeekFrom::Start(
        DESLOCAMENTO_XGD3 + setor_raiz as u64 * SETOR,
    ))
    .unwrap();
    f.write_all(&bloco_raiz).unwrap();

    let mut g = gdf::Gdf::abrir(&caminho).expect("abrir GDF de teste");

    // O footprint declarado do SUBDIR (setor 2000 + 50 setores = 2050) ainda
    // conta, mesmo sem conseguir ler o que tem dentro dele — maior que o do
    // default.xex (1000 + 196 = 1196).
    let ultimo_setor = g
        .analisar_diretorios()
        .expect("não deveria propagar erro por causa de um subdiretório corrompido");
    assert_eq!(ultimo_setor, 2050);

    // A subárvore realmente não foi carregada.
    let raiz = g.raiz.as_ref().unwrap();
    assert!(raiz.encontrar("SUBDIR").unwrap().subdiretorio.is_none());

    std::fs::remove_file(&caminho).ok();
}
