//! Saída validada, byte a byte.
//!
//! A conversão já foi conferida em jogos reais no Xbox 360: o que ela grava
//! hoje para uma entrada válida é a referência. Estes testes montam ISOs
//! sintéticas de Xbox e de Xbox 360 (com `default.xex` cifrado em AES, com
//! compressão básica e com LZX, `default.xbe` com miniatura XPR, nomes com
//! acento), convertem e guardam o tamanho e o SHA-1 de **cada arquivo**
//! gerado: as partes `DataNNNN` (dados e tabelas de hash), o cabeçalho LIVE
//! (com nome, ícone e número do disco) e os nomes das pastas e arquivos.
//! Também guardam o JSON do `info`. Os valores foram gravados com a versão
//! 0.1.4, antes de qualquer correção da auditoria, e têm que continuar
//! idênticos em debug e em release.
//!
//! Tudo aqui é autocontido de propósito (montadores de ISO, XEX, XDBF e XBE
//! próprios): os testes não dependem de nenhum ajudante interno que possa
//! mudar junto com o código.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockEncrypt, KeyInit};
use sha1::{Digest, Sha1};

use iso2god::cli::{Plataforma, RemocaoPadding};
use iso2god::god::{self, OpcoesConversao};

const S: u64 = 2048;
const XGD2: u64 = 265_879_552;
const XGD3: u64 = 34_078_720;
const ARQ: u8 = 0x00;
const DIR: u8 = 0x10;

// ---------------------------------------------------------------------------
// utilidades
// ---------------------------------------------------------------------------

struct Temp(PathBuf);

impl Temp {
    fn nova(nome: &str) -> Self {
        let p = std::env::temp_dir().join(format!("iso2god-golden-{nome}-{}", std::process::id()));
        fs::remove_dir_all(&p).ok();
        fs::create_dir_all(&p).unwrap();
        Temp(p)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn sha1_hex(b: &[u8]) -> String {
    Sha1::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// Bytes pseudoaleatórios reproduzíveis (xorshift).
fn bytes(semente: u64, n: usize) -> Vec<u8> {
    let mut x = semente | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Uma linha por arquivo gerado: caminho relativo, tamanho e SHA-1.
fn manifesto(raiz: &Path) -> String {
    fn rec(raiz: &Path, pasta: &Path, linhas: &mut Vec<String>) {
        for e in fs::read_dir(pasta).unwrap() {
            let p = e.unwrap().path();
            let rel = p
                .strip_prefix(raiz)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                linhas.push(format!("{rel}/"));
                rec(raiz, &p, linhas);
            } else {
                let b = fs::read(&p).unwrap();
                linhas.push(format!("{rel} {} {}", b.len(), sha1_hex(&b)));
            }
        }
    }
    let mut linhas = Vec::new();
    rec(raiz, raiz, &mut linhas);
    linhas.sort();
    linhas.join("\n")
}

#[track_caller]
fn conferir(o_que: &str, obtido: &str, esperado: &str) {
    assert_eq!(
        obtido, esperado,
        "a saída de {o_que} mudou (deveria ser idêntica byte a byte)"
    );
}

/// Confere o que o cabeçalho LIVE diz do jogo: assim os golden cobrem de
/// fato a leitura do XEX/XBE, e não um caminho de reserva.
fn conferir_cabecalho(
    destino: &Path,
    titulo: &str,
    icone: Option<&[u8]>,
    disco: u8,
    total: u8,
) -> Vec<u8> {
    let mut cab = None;
    for t in fs::read_dir(destino).unwrap() {
        for tipo in fs::read_dir(t.unwrap().path()).unwrap() {
            for e in fs::read_dir(tipo.unwrap().path()).unwrap() {
                let p = e.unwrap().path();
                if p.is_file() {
                    cab = Some(fs::read(p).unwrap());
                }
            }
        }
    }
    let cab = cab.expect("cabeçalho LIVE não encontrado");
    let utf16: Vec<u8> = titulo.encode_utf16().flat_map(u16::to_be_bytes).collect();
    assert_eq!(
        &cab[1041..1041 + utf16.len()],
        &utf16[..],
        "título no cabeçalho"
    );
    assert_eq!(
        &cab[5777..5777 + utf16.len()],
        &utf16[..],
        "segunda cópia do título"
    );
    if let Some(png) = icone {
        let tam = u32::from_be_bytes(cab[5906..5910].try_into().unwrap()) as usize;
        assert_eq!(tam, png.len(), "tamanho do ícone");
        assert_eq!(&cab[5914..5914 + tam], png, "ícone no cabeçalho");
    }
    assert_eq!((cab[870], cab[871]), (disco, total), "número do disco");
    cab
}

// ---------------------------------------------------------------------------
// ISO (GDF)
// ---------------------------------------------------------------------------

fn entrada(esq: u16, dir: u16, setor: u32, tamanho: u32, attr: u8, nome: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&esq.to_le_bytes());
    v.extend_from_slice(&dir.to_le_bytes());
    v.extend_from_slice(&setor.to_le_bytes());
    v.extend_from_slice(&tamanho.to_le_bytes());
    v.push(attr);
    v.push(nome.len() as u8);
    v.extend_from_slice(nome);
    while !v.len().is_multiple_of(4) {
        v.push(0xFF);
    }
    v
}

/// Tabela de diretório como lista ligada pela direita, sem entrada
/// cruzando setor, preenchida com 0xFF até o fim do setor.
fn tabela(ents: &[(&[u8], u32, u32, u8)]) -> Vec<u8> {
    let mut pos = Vec::new();
    let mut p = 0usize;
    for (nome, ..) in ents {
        let t = (14 + nome.len()).div_ceil(4) * 4;
        if p % S as usize + t > S as usize {
            p = p.div_ceil(S as usize) * S as usize;
        }
        pos.push(p);
        p += t;
    }
    let mut v = vec![0xFFu8; p.div_ceil(S as usize).max(1) * S as usize];
    for (i, &(nome, setor, tamanho, attr)) in ents.iter().enumerate() {
        let dir = pos.get(i + 1).map_or(0, |&q| (q / 4) as u16);
        let e = entrada(0, dir, setor, tamanho, attr, nome);
        v[pos[i]..pos[i] + e.len()].copy_from_slice(&e);
    }
    v
}

/// Grava uma ISO no layout de `deslocamento`: o descritor de volume no setor
/// 32 da partição, cada (setor, bytes) no lugar, e o arquivo com
/// `setores_volume` setores depois do deslocamento (esparso).
fn gravar_iso(
    caminho: &Path,
    deslocamento: u64,
    raiz: (u32, &[u8]),
    partes: &[(u32, &[u8])],
    setores_volume: u64,
) {
    let mut f = File::create(caminho).unwrap();
    f.set_len(deslocamento + setores_volume * S).unwrap();
    // lixo na partição de vídeo, antes do volume
    if deslocamento > 0 {
        f.write_all(&bytes(77, 64 * 1024)).unwrap();
    }
    let mut d = vec![0u8; S as usize];
    d[0..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    d[20..24].copy_from_slice(&raiz.0.to_le_bytes());
    d[24..28].copy_from_slice(&(raiz.1.len() as u32).to_le_bytes());
    d[28..36].copy_from_slice(&0x01D4_5A1B_2C3D_4E5Fu64.to_le_bytes());
    d[0x7EC..0x800].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    let mut por = |setor: u32, b: &[u8]| {
        f.seek(SeekFrom::Start(deslocamento + setor as u64 * S))
            .unwrap();
        f.write_all(b).unwrap();
    };
    por(32, &d);
    por(raiz.0, raiz.1);
    for (setor, b) in partes {
        por(*setor, b);
    }
}

// ---------------------------------------------------------------------------
// XEX (AES + compressão básica ou LZX) com XDBF
// ---------------------------------------------------------------------------

const CHAVE_RETAIL: [u8; 16] = [
    0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91,
];

fn cifrar_bloco(chave: &[u8; 16], bloco: &[u8; 16]) -> [u8; 16] {
    let aes = Aes128::new(GenericArray::from_slice(chave));
    let mut b = GenericArray::clone_from_slice(bloco);
    aes.encrypt_block(&mut b);
    b.into()
}

fn cifrar_cbc(chave: &[u8; 16], dados: &[u8]) -> Vec<u8> {
    let aes = Aes128::new(GenericArray::from_slice(chave));
    let mut anterior = [0u8; 16];
    let mut saida = Vec::new();
    for bloco in dados.chunks_exact(16) {
        let mut b = GenericArray::clone_from_slice(bloco);
        for (x, p) in b.iter_mut().zip(anterior) {
            *x ^= p;
        }
        aes.encrypt_block(&mut b);
        saida.extend_from_slice(&b);
        anterior.copy_from_slice(&b);
    }
    saida
}

/// Informações de execução do XEX: Title ID, Media ID, plataforma, tipo de
/// executável, disco e total de discos.
struct Execucao {
    title_id: [u8; 4],
    media_id: [u8; 4],
    plataforma: u8,
    tipo: u8,
    disco: u8,
    total: u8,
}

/// XEX2 com o recurso XDBF (nome = Title ID em hex) dentro da imagem PE,
/// cifrada com a chave retail e comprimida (1 = básica, 2 = LZX).
fn montar_xex(exec: &Execucao, recurso: &[u8], compressao: u16) -> Vec<u8> {
    const BASE: u32 = 0x8200_0000;
    const OFF_RECURSO: usize = 0x3000;
    let mut imagem = vec![0u8; OFF_RECURSO];
    imagem[0..2].copy_from_slice(b"MZ");
    imagem.extend_from_slice(recurso);
    imagem.extend_from_slice(&bytes(0xC0DE, 70_000));
    while !imagem.len().is_multiple_of(16) {
        imagem.push(0);
    }

    let (formato, dados): (Vec<u8>, Vec<u8>) = match compressao {
        1 => {
            let mut f = Vec::new();
            f.extend_from_slice(&16u32.to_be_bytes());
            f.extend_from_slice(&1u16.to_be_bytes()); // AES
            f.extend_from_slice(&1u16.to_be_bytes()); // básica
            f.extend_from_slice(&(imagem.len() as u32).to_be_bytes());
            f.extend_from_slice(&0x2000u32.to_be_bytes());
            (f, imagem.clone())
        }
        _ => {
            let mut enc = lzxc::Encoder::new(lzxc::WindowSize::KB64);
            let mut bloco = vec![0u8; 24];
            for trecho in imagem.chunks(0x8000) {
                let c = enc.encode_chunk(trecho);
                bloco.extend_from_slice(&(c.len() as u16).to_be_bytes());
                bloco.extend_from_slice(&c);
            }
            bloco.extend_from_slice(&0u16.to_be_bytes());
            while !bloco.len().is_multiple_of(16) {
                bloco.push(0);
            }
            let mut f = Vec::new();
            f.extend_from_slice(&36u32.to_be_bytes());
            f.extend_from_slice(&1u16.to_be_bytes()); // AES
            f.extend_from_slice(&2u16.to_be_bytes()); // LZX
            f.extend_from_slice(&0x1_0000u32.to_be_bytes()); // janela 64 KB
            f.extend_from_slice(&(bloco.len() as u32).to_be_bytes());
            f.extend_from_slice(&[0u8; 20]);
            (f, bloco)
        }
    };

    let tamanho_imagem = imagem.len() as u32 + if compressao == 1 { 0x2000 } else { 0 };
    let chave_clara = [0x5Au8; 16];
    let chave_cifrada = cifrar_bloco(&CHAVE_RETAIL, &chave_clara);
    let nome: String = exec.title_id.iter().map(|b| format!("{b:02X}")).collect();

    // cabeçalho, 4 opcionais, segurança em 0x100, recursos em 0x300,
    // formato em 0x340, execução em 0x3C0, dados em 0x1000
    let mut xex = vec![0u8; 0x1000];
    xex[0..4].copy_from_slice(b"XEX2");
    xex[8..12].copy_from_slice(&0x1000u32.to_be_bytes());
    xex[16..20].copy_from_slice(&0x100u32.to_be_bytes());
    xex[20..24].copy_from_slice(&4u32.to_be_bytes());
    for (i, (k, v)) in [
        (0x0000_02FFu32, 0x300u32),
        (0x0000_03FF, 0x340),
        (0x0001_0201, BASE),
        (0x0004_0006, 0x3C0),
    ]
    .iter()
    .enumerate()
    {
        xex[24 + i * 8..28 + i * 8].copy_from_slice(&k.to_be_bytes());
        xex[28 + i * 8..32 + i * 8].copy_from_slice(&v.to_be_bytes());
    }
    xex[0x104..0x108].copy_from_slice(&tamanho_imagem.to_be_bytes());
    xex[0x210..0x214].copy_from_slice(&BASE.to_be_bytes());
    xex[0x250..0x260].copy_from_slice(&chave_cifrada);
    xex[0x300..0x304].copy_from_slice(&20u32.to_be_bytes());
    xex[0x304..0x30C].copy_from_slice(nome.as_bytes());
    xex[0x30C..0x310].copy_from_slice(&(BASE + OFF_RECURSO as u32).to_be_bytes());
    xex[0x310..0x314].copy_from_slice(&(recurso.len() as u32).to_be_bytes());
    xex[0x340..0x340 + formato.len()].copy_from_slice(&formato);
    xex[0x3C0..0x3C4].copy_from_slice(&exec.media_id);
    xex[0x3CC..0x3D0].copy_from_slice(&exec.title_id);
    xex[0x3D0] = exec.plataforma;
    xex[0x3D1] = exec.tipo;
    xex[0x3D2] = exec.disco;
    xex[0x3D3] = exec.total;
    xex.extend_from_slice(&cifrar_cbc(&chave_clara, &dados));
    xex
}

fn xstr(textos: &[(u16, &str)]) -> Vec<u8> {
    let mut corpo = Vec::new();
    for (id, t) in textos {
        corpo.extend_from_slice(&id.to_be_bytes());
        corpo.extend_from_slice(&(t.len() as u16).to_be_bytes());
        corpo.extend_from_slice(t.as_bytes());
    }
    let mut b = b"XSTR".to_vec();
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&(corpo.len() as u32 + 2).to_be_bytes());
    b.extend_from_slice(&(textos.len() as u16).to_be_bytes());
    b.extend(corpo);
    b
}

/// XDBF com o nome em inglês e em japonês, idioma padrão `padrao` e ícone.
fn montar_xdbf(ingles: &str, japones: &str, padrao: u32, icone: &[u8]) -> Vec<u8> {
    let mut xstc = b"XSTC".to_vec();
    xstc.extend_from_slice(&1u32.to_be_bytes());
    xstc.extend_from_slice(&4u32.to_be_bytes());
    xstc.extend_from_slice(&padrao.to_be_bytes());
    let blobs: Vec<(u16, u64, Vec<u8>)> = vec![
        (1, 0x5853_5443, xstc),
        (3, 1, xstr(&[(0x0001, "outro texto"), (0x8000, ingles)])),
        (3, 2, xstr(&[(0x8000, japones)])),
        (2, 0x8000, icone.to_vec()),
    ];
    let n = blobs.len() as u32;
    let mut cab = b"XDBF".to_vec();
    cab.extend_from_slice(&0x1_0000u32.to_be_bytes());
    cab.extend_from_slice(&n.to_be_bytes());
    cab.extend_from_slice(&n.to_be_bytes());
    cab.extend_from_slice(&1u32.to_be_bytes());
    cab.extend_from_slice(&0u32.to_be_bytes());
    let mut dados = Vec::new();
    for (ns, id, b) in &blobs {
        cab.extend_from_slice(&ns.to_be_bytes());
        cab.extend_from_slice(&id.to_be_bytes());
        cab.extend_from_slice(&(dados.len() as u32).to_be_bytes());
        cab.extend_from_slice(&(b.len() as u32).to_be_bytes());
        dados.extend_from_slice(b);
    }
    cab.extend_from_slice(&[0u8; 8]);
    cab.extend(dados);
    cab
}

/// "PNG" de ícone: a assinatura e bytes reproduzíveis (o conversor só
/// confere a assinatura e copia os bytes para o cabeçalho).
fn icone(semente: u64, n: usize) -> Vec<u8> {
    let mut p = b"\x89PNG\r\n\x1a\n".to_vec();
    p.extend_from_slice(&bytes(semente, n));
    p
}

// ---------------------------------------------------------------------------
// XBE com miniatura XPR (DXT1)
// ---------------------------------------------------------------------------

fn montar_xbe(title_id: u32, titulo: &str, disco: u32) -> Vec<u8> {
    const BASE: u32 = 0x0001_0000;
    let mut xbe = vec![0u8; 0x600];
    xbe[0..4].copy_from_slice(b"XBEH");
    xbe[260..264].copy_from_slice(&BASE.to_le_bytes());
    xbe[280..284].copy_from_slice(&(BASE + 0x200).to_le_bytes()); // certificado
    xbe[284..288].copy_from_slice(&1u32.to_le_bytes()); // uma seção
    xbe[288..292].copy_from_slice(&(BASE + 0x400).to_le_bytes()); // tabela de seções
    // certificado
    xbe[0x208..0x20C].copy_from_slice(&title_id.to_le_bytes());
    let nome: Vec<u8> = titulo.encode_utf16().flat_map(u16::to_le_bytes).collect();
    xbe[0x20C..0x20C + nome.len()].copy_from_slice(&nome);
    xbe[0x200 + 168..0x200 + 172].copy_from_slice(&disco.to_le_bytes());
    // seção $$XTIMAGE: dados em 0x500, nome em 0x480
    let mut xpr = Vec::new();
    xpr.extend_from_slice(b"XPR0");
    let textura = bytes(0xD7, 32); // 8x8 em DXT1: 4 blocos de 8 bytes
    xpr.extend_from_slice(&(28 + textura.len() as u32).to_le_bytes());
    xpr.extend_from_slice(&28u32.to_le_bytes());
    xpr.extend_from_slice(&[0u8; 12]);
    xpr.extend_from_slice(&[0, 12, 0, 3]); // DXT1, 2^3 = 8
    xpr.extend_from_slice(&textura);
    xbe[0x400 + 12..0x400 + 16].copy_from_slice(&0x500u32.to_le_bytes());
    xbe[0x400 + 16..0x400 + 20].copy_from_slice(&(xpr.len() as u32).to_le_bytes());
    xbe[0x400 + 20..0x400 + 24].copy_from_slice(&(BASE + 0x480).to_le_bytes());
    xbe[0x480..0x48A].copy_from_slice(b"$$XTIMAGE\0");
    xbe[0x500..0x500 + xpr.len()].copy_from_slice(&xpr);
    xbe
}

// ---------------------------------------------------------------------------
// imagens de teste
// ---------------------------------------------------------------------------

/// Disco de Xbox 360: `default.xex`, uma pasta com um arquivo grande, um
/// arquivo com nome Latin-1 acentuado, um arquivo vazio e uma pasta vazia.
fn iso_xbox360(t: &Temp, nome: &str, deslocamento: u64, xex: &[u8]) -> PathBuf {
    let fundo = bytes(1, 300_000);
    let cancao = bytes(2, 5_000);
    let sub = tabela(&[(b"fundo.bin", 300, fundo.len() as u32, ARQ)]);
    let raiz = tabela(&[
        (b"Can\xe7\xe3o.ogg", 500, cancao.len() as u32, ARQ),
        (b"default.xex", 100, xex.len() as u32, ARQ),
        (b"media", 40, sub.len() as u32, DIR),
        (b"vazia", 41, S as u32, DIR),
        (b"zero.bin", 0, 0, ARQ),
    ]);
    let caminho = t.0.join(nome);
    gravar_iso(
        &caminho,
        deslocamento,
        (33, &raiz),
        &[
            (40, &sub),
            (41, &[0xFF; S as usize]),
            (100, xex),
            (300, &fundo),
            (500, &cancao),
        ],
        // volume com folga depois do último arquivo (padding no fim)
        600,
    );
    caminho
}

fn iso_xbox(t: &Temp, xbe: &[u8]) -> PathBuf {
    let dados = bytes(3, 40_000);
    let raiz = tabela(&[
        (b"dados.bin", 120, dados.len() as u32, ARQ),
        (b"default.xbe", 100, xbe.len() as u32, ARQ),
    ]);
    let caminho = t.0.join("xbox.iso");
    gravar_iso(&caminho, 0, (33, &raiz), &[(100, xbe), (120, &dados)], 200);
    caminho
}

fn opcoes(origem: PathBuf, destino: PathBuf) -> OpcoesConversao {
    OpcoesConversao {
        origem,
        destino,
        padding: RemocaoPadding::Parcial,
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

fn converter(o: OpcoesConversao) -> String {
    let destino = o.destino.clone();
    god::converter(&o).expect("a conversão deveria funcionar");
    manifesto(&destino)
}

fn xex_basica() -> Vec<u8> {
    montar_xex(
        &Execucao {
            title_id: [0x4D, 0x53, 0x08, 0xBF],
            media_id: [0xAA, 0xBB, 0xCC, 0xDD],
            plataforma: 2,
            tipo: 0,
            disco: 1,
            total: 1,
        },
        &montar_xdbf("Jogo de Ação", "ゲーム", 1, &icone(10, 600)),
        1,
    )
}

fn xex_lzx() -> Vec<u8> {
    montar_xex(
        &Execucao {
            title_id: [0x41, 0x56, 0x07, 0xD4],
            media_id: [0x12, 0x34, 0x56, 0x78],
            plataforma: 2,
            tipo: 1,
            disco: 2,
            total: 2,
        },
        &montar_xdbf("Coração Valente", "勇敢な心", 2, &icone(11, 900)),
        2,
    )
}

// ---------------------------------------------------------------------------
// os testes
// ---------------------------------------------------------------------------

#[test]
fn xbox360_xex_basica_padding_parcial_nenhuma_e_completa() {
    let t = Temp::nova("basica");
    let iso = iso_xbox360(&t, "basica.iso", XGD3, &xex_basica());

    let m = converter(opcoes(iso.clone(), t.0.join("parcial")));
    conferir_cabecalho(
        &t.0.join("parcial"),
        "Jogo de Ação",
        Some(&icone(10, 600)),
        1,
        1,
    );
    conferir(
        "Xbox 360, XEX com compressão básica, padding parcial",
        &m,
        "4D5308BF/\n4D5308BF/00007000/\n4D5308BF/00007000/FE40C7D9CF2D599EB911 45056 e6b0102cc9f130e6861378739dea1d1030df8c07\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/Data0000 1044480 46d46d77934ff1333c41cebe0c599b1665d6ed25",
    );

    let m = converter(OpcoesConversao {
        padding: RemocaoPadding::Nenhuma,
        ..opcoes(iso.clone(), t.0.join("nenhuma"))
    });
    conferir(
        "Xbox 360, XEX com compressão básica, padding nenhuma",
        &m,
        "4D5308BF/\n4D5308BF/00007000/\n4D5308BF/00007000/FE40C7D9CF2D599EB911 45056 37c25a4ce703ba8630d2d27d2ae6b88e7144f748\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/Data0000 1241088 62756344f0041c3da3251746b7d71624d24bf162",
    );

    let m = converter(OpcoesConversao {
        padding: RemocaoPadding::Completa,
        ..opcoes(iso.clone(), t.0.join("completa"))
    });
    conferir(
        "Xbox 360, XEX com compressão básica, padding completa",
        &m,
        "4D5308BF/\n4D5308BF/00007000/\n4D5308BF/00007000/FE40C7D9CF2D599EB911 45056 c5f99c4ed18945740f8b15542fa49ebd2b2efd59\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/Data0000 483328 2087478f550857b570c37831630b42ed342cf895",
    );

    let info = serde_json::to_string(&iso2god::analise::analisar(&iso).unwrap()).unwrap();
    conferir(
        "info --json (Xbox 360, básica)",
        &sha1_hex(info.as_bytes()),
        "99435da59694093b885c5d004cedeb012661c0e9",
    );
}

#[test]
fn xbox360_xex_lzx_disco_2_de_2_com_numero_e_quatro_threads() {
    let t = Temp::nova("lzx");
    let iso = iso_xbox360(&t, "lzx.iso", XGD2, &xex_lzx());

    let m = converter(OpcoesConversao {
        numero_disco: true,
        threads: 4,
        ..opcoes(iso.clone(), t.0.join("saida"))
    });
    conferir_cabecalho(
        &t.0.join("saida"),
        "勇敢な心 - Disc 2",
        Some(&icone(11, 900)),
        2,
        2,
    );
    conferir(
        "Xbox 360, XEX com LZX, disco 2/2, -j 4",
        &m,
        "415607D4/\n415607D4/00007000/\n415607D4/00007000/B7D7BA05ECAEB8FFD5E8 45056 b642c163027a4e5f3f7acd2608783e2eb6989304\n415607D4/00007000/B7D7BA05ECAEB8FFD5E8.data/\n415607D4/00007000/B7D7BA05ECAEB8FFD5E8.data/Data0000 1044480 aaf1d58d343a6c7c804d13c4b57cdd346f280514",
    );

    let info = serde_json::to_string(&iso2god::analise::analisar(&iso).unwrap()).unwrap();
    conferir(
        "info --json (Xbox 360, LZX)",
        &sha1_hex(info.as_bytes()),
        "d939af5915f43eec67fab9c1aae72886c7bd1293",
    );
}

#[test]
fn xbox_original_xbe_com_miniatura() {
    let t = Temp::nova("xbe");
    let iso = iso_xbox(&t, &montar_xbe(0x4D53_0064, "Jogo Clássico ç", 0));

    let m = converter(opcoes(iso.clone(), t.0.join("saida")));
    let cab = conferir_cabecalho(&t.0.join("saida"), "Jogo Clássico ç", None, 1, 1);
    let tam = u32::from_be_bytes(cab[5906..5910].try_into().unwrap()) as usize;
    assert!(
        tam > 100 && cab[5914..].starts_with(b"\x89PNG"),
        "a miniatura do XBE deveria virar o ícone (PNG)"
    );
    conferir(
        "Xbox original, XBE com miniatura DXT1",
        &m,
        "4D530064/\n4D530064/00005000/\n4D530064/00005000/2CC07C752C92C76AA78B 45056 59f1eb23b09da769c14d0ab79111f0306f911b5c\n4D530064/00005000/2CC07C752C92C76AA78B.data/\n4D530064/00005000/2CC07C752C92C76AA78B.data/Data0000 294912 f7384ab072b57c7f860483c9fc76f3c27474c788",
    );

    let info = serde_json::to_string(&iso2god::analise::analisar(&iso).unwrap()).unwrap();
    conferir(
        "info --json (Xbox original)",
        &sha1_hex(info.as_bytes()),
        "1dd20caf29b4db75764356a0fb285163f62a6f44",
    );
}

#[test]
fn opcoes_manuais_titulo_icone_e_disco() {
    let t = Temp::nova("manual");
    let iso = iso_xbox(&t, &montar_xbe(0x4D53_0064, "Jogo Clássico ç", 0));
    let png = t.0.join("icone.png");
    fs::write(&png, icone(12, 1500)).unwrap();

    let m = converter(OpcoesConversao {
        numero_disco: true,
        plataforma: Some(Plataforma::Xbox360),
        title_id: Some("4d5308bf".into()),
        media_id: Some("0BADF00D".into()),
        titulo: Some("Título Manual — Edição Ç".into()),
        disco: Some(3),
        total_discos: Some(4),
        plataforma_byte: Some(7),
        tipo_executavel_byte: Some(1),
        icone: Some(png),
        ..opcoes(iso, t.0.join("saida"))
    });
    conferir_cabecalho(
        &t.0.join("saida"),
        "Título Manual — Edição Ç",
        Some(&icone(12, 1500)),
        3,
        4,
    );
    conferir(
        "opções manuais (título, ícone, disco 3/4)",
        &m,
        "4D5308BF/\n4D5308BF/00007000/\n4D5308BF/00007000/836B91F43E284E85A3EA 45056 138b59e6c0e2756386232e43ef5ea868bef7533c\n4D5308BF/00007000/836B91F43E284E85A3EA.data/\n4D5308BF/00007000/836B91F43E284E85A3EA.data/Data0000 294912 f7384ab072b57c7f860483c9fc76f3c27474c788",
    );
}

/// Mais de 41.412 blocos: duas partes e a cadeia de hash entre as Master
/// Hash Tables. Com 1 e com 4 threads, a saída tem que ser a mesma.
#[test]
fn duas_partes_com_cadeia_de_hash() {
    let t = Temp::nova("partes");
    let xex = xex_basica();
    let marcador = bytes(4, 3 * 4096 + 123);
    let grande = 42_000u32 * 4096 + 777; // passa de uma parte
    let raiz = tabela(&[
        (b"default.xex", 100, xex.len() as u32, ARQ),
        (b"enorme.bin", 200, grande, ARQ),
    ]);
    let iso = t.0.join("partes.iso");
    let setores = 200 + (grande as u64).div_ceil(S) + 4;
    gravar_iso(
        &iso,
        XGD3,
        (33, &raiz),
        &[
            (100, &xex),
            (200, &marcador),
            (200 + grande / S as u32 - 2, &marcador),
        ],
        setores,
    );

    let m1 = converter(opcoes(iso.clone(), t.0.join("j1")));
    conferir(
        "duas partes, -j 1",
        &m1,
        "4D5308BF/\n4D5308BF/00007000/\n4D5308BF/00007000/FE40C7D9CF2D599EB911 45056 cf69264d1bc50c3855fe3debe62a0d1cb3719bdc\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/Data0000 170459136 5caa43ff2dee592481950becfde211874d1a894c\n4D5308BF/00007000/FE40C7D9CF2D599EB911.data/Data0001 2842624 1e52e23c6560519ecaacc7266f566042490bbde9",
    );
    let m4 = converter(OpcoesConversao {
        threads: 4,
        ..opcoes(iso, t.0.join("j4"))
    });
    assert_eq!(
        m4, m1,
        "com 4 threads a saída tem que ser a mesma de 1 thread"
    );
}

/// O programa de verdade, como o xiso-manager o chama: o contrato do
/// `--progresso-json` (eventos e campos), o `info --json` e os códigos de
/// saída.
#[test]
fn linha_de_comando_progresso_json_info_e_codigos_de_saida() {
    use std::process::Command;
    let t = Temp::nova("cli");
    let iso = iso_xbox360(&t, "basica.iso", XGD3, &xex_basica());
    let exe = env!("CARGO_BIN_EXE_iso2god");

    let saida = Command::new(exe)
        .arg("converter")
        .arg(&iso)
        .arg(t.0.join("saida"))
        .arg("--progresso-json")
        .output()
        .unwrap();
    assert_eq!(saida.status.code(), Some(0));
    let mut sequencia: Vec<String> = Vec::new();
    for linha in String::from_utf8(saida.stdout).unwrap().lines() {
        let v: serde_json::Value = serde_json::from_str(linha).expect("cada linha é um JSON");
        let o = v.as_object().unwrap();
        let mut chaves: Vec<&str> = o.keys().map(String::as_str).collect();
        chaves.sort();
        let item = match o["evento"].as_str().unwrap() {
            "fase" => format!("fase:{} {chaves:?}", o["fase"].as_str().unwrap()),
            outro => format!("{outro} {chaves:?}"),
        };
        // a quantidade de eventos de progresso depende do relógio
        if sequencia.last() != Some(&item) {
            sequencia.push(item);
        }
    }
    conferir(
        "--progresso-json (sequência de eventos e campos)",
        &sequencia.join("\n"),
        "fase:iniciando [\"evento\", \"fase\", \"mensagem\"]\nfase:convertendo [\"evento\", \"fase\", \"mensagem\"]\nprogresso [\"blocos\", \"bytes\", \"eta_segundos\", \"evento\", \"total_blocos\", \"total_bytes\", \"velocidade_bps\"]\nfase:calculando_hash [\"evento\", \"fase\", \"mensagem\"]\nfase:gravando_cabecalho [\"evento\", \"fase\", \"mensagem\"]\nconcluido [\"duracao_segundos\", \"evento\", \"mensagem\", \"pasta\"]",
    );
    assert_eq!(
        manifesto(&t.0.join("saida")),
        converter(opcoes(iso.clone(), t.0.join("biblioteca"))),
        "a linha de comando grava o mesmo que a biblioteca"
    );

    let info = Command::new(exe)
        .arg("info")
        .arg(&iso)
        .arg("--json")
        .output()
        .unwrap();
    assert_eq!(info.status.code(), Some(0));
    conferir(
        "info --json (programa)",
        &sha1_hex(&info.stdout),
        "dbf92690d2aeff1b2035dfa2eb8fb2e9256cc5c2",
    );

    // erro: uma linha `erro` e código 1
    let erro = Command::new(exe)
        .arg("converter")
        .arg(t.0.join("nao-existe.iso"))
        .arg(t.0.join("saida2"))
        .arg("--progresso-json")
        .output()
        .unwrap();
    assert_eq!(erro.status.code(), Some(1));
    let ultima = String::from_utf8(erro.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(ultima.lines().last().unwrap()).unwrap();
    assert_eq!(v["evento"], "erro");
    assert!(v["mensagem"].as_str().is_some_and(|m| !m.is_empty()));

    // opção inválida: código 2 (o do clap)
    let uso = Command::new(exe)
        .arg("converter")
        .arg("--nao-existe")
        .output()
        .unwrap();
    assert_eq!(uso.status.code(), Some(2));
}
