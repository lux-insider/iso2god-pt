//! Nomes de arquivo no `--padding completa` (itens N-1 e N-2 do
//! AUDITORIA.md).
//!
//! A reconstrução regrava as tabelas de diretório numa GDF nova, que vai
//! inteira para dentro do pacote. Os testes convertem uma imagem, tiram a
//! GDF reconstruída de dentro das partes `DataNNNN` e a percorrem como o
//! console faz — seguindo os ponteiros da árvore de cada tabela —,
//! conferindo os bytes de cada nome e os dados de cada arquivo.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use iso2god::cli::{Plataforma, RemocaoPadding};
use iso2god::god::{self, OpcoesConversao};

const S: usize = 2048;
const BLOCO: usize = 4096;
const ARQ: u8 = 0x00;
const DIR: u8 = 0x10;

struct Temp(PathBuf);

impl Temp {
    fn nova(nome: &str) -> Self {
        let p = std::env::temp_dir().join(format!("iso2god-nomes-{nome}-{}", std::process::id()));
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

/// Bytes reproduzíveis, diferentes para cada semente.
fn bytes(semente: u64, n: usize) -> Vec<u8> {
    let mut x = semente.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

// ---------------------------------------------------------------------------
// montagem da imagem de origem (Xsf, deslocamento 0)
// ---------------------------------------------------------------------------

/// Uma entrada de uma tabela de diretório a montar.
struct Ent<'a> {
    nome: &'a [u8],
    setor: u32,
    tamanho: u32,
    attr: u8,
}

/// Tabela de diretório de um setor, como lista ligada pela direita.
fn tabela(ents: &[Ent]) -> Vec<u8> {
    let mut v = Vec::new();
    for (i, e) in ents.iter().enumerate() {
        let tamanho = (14 + e.nome.len()).div_ceil(4) * 4;
        let direita = if i + 1 < ents.len() {
            ((v.len() + tamanho) / 4) as u16
        } else {
            0
        };
        v.extend_from_slice(&0u16.to_le_bytes());
        v.extend_from_slice(&direita.to_le_bytes());
        v.extend_from_slice(&e.setor.to_le_bytes());
        v.extend_from_slice(&e.tamanho.to_le_bytes());
        v.push(e.attr);
        v.push(e.nome.len() as u8);
        v.extend_from_slice(e.nome);
        while !v.len().is_multiple_of(4) {
            v.push(0xFF);
        }
    }
    assert!(v.len() <= S, "a tabela de teste cabe num setor");
    v.resize(S, 0xFF);
    v
}

fn gravar_iso(caminho: &Path, partes: &[(u32, &[u8])], setores: u64) {
    let mut f = File::create(caminho).unwrap();
    f.set_len(setores * S as u64).unwrap();
    let mut d = vec![0u8; S];
    d[0..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    d[20..24].copy_from_slice(&33u32.to_le_bytes());
    d[24..28].copy_from_slice(&(S as u32).to_le_bytes());
    f.seek(SeekFrom::Start(32 * S as u64)).unwrap();
    f.write_all(&d).unwrap();
    for (setor, b) in partes {
        f.seek(SeekFrom::Start(*setor as u64 * S as u64)).unwrap();
        f.write_all(b).unwrap();
    }
}

/// Converte com `--padding completa` e devolve a GDF reconstruída, tirada
/// de dentro do pacote.
fn converter_completa(t: &Temp, iso: &Path) -> Vec<u8> {
    let destino = t.0.join("pacote");
    god::converter(&OpcoesConversao {
        origem: iso.to_path_buf(),
        destino: destino.clone(),
        padding: RemocaoPadding::Completa,
        numero_disco: false,
        plataforma: Some(Plataforma::Xbox360),
        title_id: Some("4D5308BF".into()),
        media_id: Some("AABBCCDD".into()),
        titulo: None,
        disco: None,
        total_discos: None,
        plataforma_byte: None,
        tipo_executavel_byte: None,
        icone: None,
        threads: 1,
        progresso_json: false,
    })
    .expect("a conversão deveria funcionar");

    let pasta = fs::read_dir(destino.join("4D5308BF/00007000"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.is_dir())
        .expect("pasta .data");
    assert!(
        !pasta.join("Data0001").exists(),
        "as imagens de teste cabem numa parte"
    );
    blocos_de_dados(&fs::read(pasta.join("Data0000")).unwrap())
}

/// Os blocos de dados de uma parte, em ordem, sem as tabelas de hash:
/// [MHT][SHT][204 blocos][SHT][204 blocos]...
fn blocos_de_dados(parte: &[u8]) -> Vec<u8> {
    let posicao = |i: usize| BLOCO + (i / 204) * (BLOCO + 204 * BLOCO) + BLOCO + (i % 204) * BLOCO;
    let mut imagem = Vec::new();
    let mut i = 0;
    while posicao(i) + BLOCO <= parte.len() {
        imagem.extend_from_slice(&parte[posicao(i)..posicao(i) + BLOCO]);
        i += 1;
    }
    imagem
}

// ---------------------------------------------------------------------------
// leitura da GDF reconstruída, como o console: seguindo a árvore
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Lida {
    nome: Vec<u8>,
    setor: u32,
    tamanho: u32,
    attr: u8,
}

/// As entradas de uma tabela, seguindo os ponteiros esquerdo e direito a
/// partir da raiz da árvore (deslocamento 0).
fn entradas(imagem: &[u8], setor: u32, tamanho: u32) -> Vec<Lida> {
    let inicio = setor as usize * S;
    let tabela = &imagem[inicio..inicio + tamanho as usize];
    let mut lidas = Vec::new();
    let mut pendentes = vec![0usize];
    let mut vistas = HashSet::new();
    while let Some(p) = pendentes.pop() {
        if !vistas.insert(p) {
            continue;
        }
        let u16_em = |q: usize| u16::from_le_bytes([tabela[q], tabela[q + 1]]);
        let u32_em = |q: usize| u32::from_le_bytes(tabela[q..q + 4].try_into().unwrap());
        let (esquerda, direita) = (u16_em(p), u16_em(p + 2));
        let n = tabela[p + 13] as usize;
        lidas.push(Lida {
            nome: tabela[p + 14..p + 14 + n].to_vec(),
            setor: u32_em(p + 4),
            tamanho: u32_em(p + 8),
            attr: tabela[p + 12],
        });
        for filho in [esquerda, direita] {
            if filho != 0 {
                pendentes.push(filho as usize * 4);
            }
        }
    }
    lidas
}

fn raiz(imagem: &[u8]) -> Vec<Lida> {
    let d = &imagem[32 * S..];
    assert_eq!(&d[0..20], b"MICROSOFT*XBOX*MEDIA");
    let setor = u32::from_le_bytes(d[20..24].try_into().unwrap());
    let tamanho = u32::from_le_bytes(d[24..28].try_into().unwrap());
    entradas(imagem, setor, tamanho)
}

/// A entrada com exatamente estes bytes de nome.
fn achar<'a>(lidas: &'a [Lida], nome: &[u8]) -> &'a Lida {
    lidas.iter().find(|e| e.nome == nome).unwrap_or_else(|| {
        let nomes: Vec<String> = lidas
            .iter()
            .map(|e| String::from_utf8_lossy(&e.nome).into_owned())
            .collect();
        panic!(
            "nenhuma entrada com o nome {:?} ({}); a tabela tem {nomes:?}",
            String::from_utf8_lossy(nome),
            nome.iter().map(|b| format!("{b:02x}")).collect::<String>()
        )
    })
}

fn dados<'a>(imagem: &'a [u8], e: &Lida) -> &'a [u8] {
    let inicio = e.setor as usize * S;
    &imagem[inicio..inicio + e.tamanho as usize]
}

// ---------------------------------------------------------------------------
// os testes
// ---------------------------------------------------------------------------

/// a) Nome em Latin-1 com acento: "Canção.ogg" é `43 61 6E E7 E3 6F ...`
/// no disco. Antes, a reconstrução gravava "Can??o.ogg" e o jogo não achava
/// o arquivo.
#[test]
fn a_nome_latin1_com_acento_sai_com_os_mesmos_bytes() {
    let t = Temp::nova("latin1");
    let nome: &[u8] = b"Can\xe7\xe3o.ogg";
    let cancao = bytes(1, 5_000);
    let outro = bytes(2, 3_000);
    let raiz_origem = tabela(&[
        Ent {
            nome,
            setor: 60,
            tamanho: cancao.len() as u32,
            attr: ARQ,
        },
        Ent {
            nome: b"outro.bin",
            setor: 50,
            tamanho: outro.len() as u32,
            attr: ARQ,
        },
    ]);
    let iso = t.0.join("latin1.iso");
    gravar_iso(&iso, &[(33, &raiz_origem), (50, &outro), (60, &cancao)], 80);

    let imagem = converter_completa(&t, &iso);
    let lidas = raiz(&imagem);
    assert_eq!(lidas.len(), 2);
    let e = achar(&lidas, nome);
    assert_eq!(dados(&imagem, e), &cancao[..]);
    assert_eq!(dados(&imagem, achar(&lidas, b"outro.bin")), &outro[..]);
}

/// b) Nomes em UTF-8 com acento, num arquivo e numa pasta (com um arquivo
/// acentuado dentro). Antes, cada byte a partir de 0x80 virava "?":
/// "Canção.ogg" saía "Can????o.ogg" e a pasta "Músicas" saía "M??sicas".
#[test]
fn b_nome_utf8_com_acento_sai_com_os_mesmos_bytes() {
    let t = Temp::nova("utf8");
    let arquivo = "Canção.ogg".as_bytes();
    let pasta = "Músicas".as_bytes();
    let dentro = "Ação.wav".as_bytes();
    let cancao = bytes(3, 7_000);
    let acao = bytes(4, 9_000);
    let sub = tabela(&[Ent {
        nome: dentro,
        setor: 70,
        tamanho: acao.len() as u32,
        attr: ARQ,
    }]);
    let raiz_origem = tabela(&[
        Ent {
            nome: arquivo,
            setor: 50,
            tamanho: cancao.len() as u32,
            attr: ARQ,
        },
        Ent {
            nome: pasta,
            setor: 40,
            tamanho: S as u32,
            attr: DIR,
        },
    ]);
    let iso = t.0.join("utf8.iso");
    gravar_iso(
        &iso,
        &[(33, &raiz_origem), (40, &sub), (50, &cancao), (70, &acao)],
        90,
    );

    let imagem = converter_completa(&t, &iso);
    let lidas = raiz(&imagem);
    assert_eq!(dados(&imagem, achar(&lidas, arquivo)), &cancao[..]);
    let p = achar(&lidas, pasta);
    assert_eq!(p.attr, DIR);
    let sub_lidas = entradas(&imagem, p.setor, p.tamanho);
    assert_eq!(dados(&imagem, achar(&sub_lidas, dentro)), &acao[..]);
}
