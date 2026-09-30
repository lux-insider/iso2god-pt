//! Teste de integração: uma ISO sintética de Xbox original (com um
//! `default.xbe` de verdade dentro) deve ter todos os metadados detectados
//! automaticamente pela orquestração completa (`god::converter`), sem
//! nenhuma flag manual de identificação.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

use iso2god::cli::RemocaoPadding;
use iso2god::god::{self, OpcoesConversao};

const SETOR: u64 = 2048;
const BASE_ASSINATURA: u64 = 32 * SETOR;
const DESLOCAMENTO_XGD3: u64 = 34_078_720;
const ASSINATURA: &[u8] = b"MICROSOFT*XBOX*MEDIA";

const ASSINATURA_XBEH: u32 = 1_212_498_520;
const OFF_BASE_ADDRESS: usize = 260;
const OFF_CERTIFICATE_ADDRESS: usize = 280;
const TAMANHO_CERTIFICADO: usize = 172;

/// Monta os bytes de um XBE sintético mínimo, válido o suficiente para
/// `xbe::ler_info_certificado` extrair Title ID, título e número do disco.
fn montar_xbe_sintetico(title_id: u32, titulo: &str, disco_numero: u32) -> Vec<u8> {
    const BASE_ADDRESS: u32 = 0x0001_0000;
    const TAMANHO_CABECALHO: u32 = 284;

    let mut xbe = vec![0u8; TAMANHO_CABECALHO as usize];
    xbe[0..4].copy_from_slice(&ASSINATURA_XBEH.to_le_bytes());
    xbe[OFF_BASE_ADDRESS..OFF_BASE_ADDRESS + 4].copy_from_slice(&BASE_ADDRESS.to_le_bytes());

    let certificate_address = BASE_ADDRESS + TAMANHO_CABECALHO;
    xbe[OFF_CERTIFICATE_ADDRESS..OFF_CERTIFICATE_ADDRESS + 4]
        .copy_from_slice(&certificate_address.to_le_bytes());

    let mut cert = vec![0u8; TAMANHO_CERTIFICADO];
    cert[8..12].copy_from_slice(&title_id.to_le_bytes());
    let nome_utf16: Vec<u8> = titulo
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    cert[12..12 + nome_utf16.len()].copy_from_slice(&nome_utf16);
    cert[168..172].copy_from_slice(&disco_numero.to_le_bytes());

    xbe.extend_from_slice(&cert);
    xbe
}

fn montar_entrada(setor: u32, tamanho: u32, attrib: u8, nome: &str) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&0u16.to_le_bytes()); // subtree_l
    b.extend_from_slice(&0u16.to_le_bytes()); // subtree_r
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

/// Cria uma ISO sintética XGD3 cujo diretório raiz contém um único arquivo,
/// "default.xbe", com o conteúdo de `xbe_bytes`.
fn criar_iso_com_xbe(nome_arquivo: &str, xbe_bytes: &[u8]) -> PathBuf {
    const SETOR_RAIZ: u32 = 100;
    const SETOR_XBE: u32 = 200;

    let mut bloco_raiz = montar_entrada(SETOR_XBE, xbe_bytes.len() as u32, 0x00, "default.xbe");
    while !(bloco_raiz.len() as u64).is_multiple_of(SETOR) {
        bloco_raiz.push(0xFF);
    }
    let tamanho_raiz = bloco_raiz.len() as u32;

    let mut descritor = Vec::new();
    descritor.extend_from_slice(ASSINATURA);
    descritor.extend_from_slice(&SETOR_RAIZ.to_le_bytes());
    descritor.extend_from_slice(&tamanho_raiz.to_le_bytes());
    descritor.extend_from_slice(b"TESTDATA");

    let caminho = std::env::temp_dir().join(nome_arquivo);
    let mut f = File::create(&caminho).expect("criar arquivo de teste");

    let tamanho_total = DESLOCAMENTO_XGD3 + (SETOR_XBE as u64) * SETOR + xbe_bytes.len() as u64;
    f.set_len(tamanho_total).unwrap();

    f.seek(SeekFrom::Start(BASE_ASSINATURA + DESLOCAMENTO_XGD3))
        .unwrap();
    f.write_all(&descritor).unwrap();

    f.seek(SeekFrom::Start(
        DESLOCAMENTO_XGD3 + SETOR_RAIZ as u64 * SETOR,
    ))
    .unwrap();
    f.write_all(&bloco_raiz).unwrap();

    f.seek(SeekFrom::Start(
        DESLOCAMENTO_XGD3 + SETOR_XBE as u64 * SETOR,
    ))
    .unwrap();
    f.write_all(xbe_bytes).unwrap();

    caminho
}

fn opcoes_sem_metadados(origem: PathBuf, destino: PathBuf) -> OpcoesConversao {
    OpcoesConversao {
        origem,
        destino,
        padding: RemocaoPadding::Nenhuma,
        numero_disco: false,
        plataforma: None,
        title_id: None,
        media_id: None,
        titulo: None,
        disco: None,
        total_discos: None,
        plataforma_byte: None,
        tipo_executavel_byte: None,
        icone: None,
        threads: 1,
        progresso_json: false,
    }
}

#[test]
fn converter_detecta_tudo_automaticamente_a_partir_do_default_xbe() {
    let xbe = montar_xbe_sintetico(0x0000_1234, "Jogo Detectado\0\0", 2);
    let origem = criar_iso_com_xbe("iso2god_teste_xbe_origem.iso", &xbe);
    let destino = std::env::temp_dir().join("iso2god_teste_xbe_destino");
    fs::remove_dir_all(&destino).ok();

    let opcoes = opcoes_sem_metadados(origem.clone(), destino.clone());
    god::converter(&opcoes).expect("conversão deveria ter sucesso sem nenhuma flag manual");

    // Xbox original -> tipo de conteúdo XboxOriginal (0x5000).
    // Title ID detectado do próprio default.xbe (0x00001234), em 8 dígitos
    let pasta_conteudo = destino.join("00001234").join("00005000");
    assert!(pasta_conteudo.is_dir());

    let entradas: Vec<_> = fs::read_dir(&pasta_conteudo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    let nome_cabecalho = entradas
        .iter()
        .find(|n| !n.ends_with(".data"))
        .expect("deveria haver um arquivo de cabeçalho");

    let cabecalho = fs::read(pasta_conteudo.join(nome_cabecalho)).unwrap();

    // Title ID detectado do certificado (0x00001234 -> 8 dígitos fixos).
    assert_eq!(&cabecalho[864..868], &0x0000_1234u32.to_be_bytes());

    // Media ID derivado do MD5 do XBE inteiro.
    let media_id_esperado = {
        use md5::{Digest, Md5};
        let mut h = Md5::new();
        h.update(&xbe);
        let hash = h.finalize();
        [hash[0], hash[1], hash[2], hash[3]]
    };
    assert_eq!(&cabecalho[852..856], &media_id_esperado);

    // Disco detectado (2), total sempre 1 para Xbox original.
    assert_eq!(cabecalho[870], 2);
    assert_eq!(cabecalho[871], 1);

    // Tipo de conteúdo = XboxOriginal (0x5000).
    assert_eq!(&cabecalho[836..840], &0x5000u32.to_be_bytes());

    // Título detectado do certificado (não caiu para o nome do arquivo).
    let titulo_bytes = &cabecalho[1041..1041 + 40];
    let titulo = titulo_bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .take_while(|&u| u != 0)
        .collect::<Vec<_>>();
    assert_eq!(String::from_utf16(&titulo).unwrap(), "Jogo Detectado");

    fs::remove_file(&origem).ok();
    fs::remove_dir_all(&destino).ok();
}
