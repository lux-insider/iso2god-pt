//! Testes dos achados do AUDITORIA.md: cada um monta o cenário do item e
//! falha no código de antes da correção.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::gdf::Gdf;

const S: u64 = 2048;
const ARQ: u8 = 0x00;
const DIR: u8 = 0x10;

pub(crate) struct Temp(pub PathBuf);

impl Temp {
    pub(crate) fn nova(nome: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("iso2god-auditoria-{nome}-{}", std::process::id()));
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

fn entrada(dir: u16, setor: u32, tamanho: u32, attr: u8, nome: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&0u16.to_le_bytes());
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

/// Tabela de diretório de um setor, como lista ligada pela direita.
pub(crate) fn tabela(ents: &[(&[u8], u32, u32, u8)]) -> Vec<u8> {
    let mut v = Vec::new();
    for (i, &(nome, setor, tamanho, attr)) in ents.iter().enumerate() {
        let proximo = if i + 1 < ents.len() {
            ((v.len() + (14 + nome.len()).div_ceil(4) * 4) / 4) as u16
        } else {
            0
        };
        v.extend(entrada(proximo, setor, tamanho, attr, nome));
    }
    assert!(v.len() <= S as usize, "a tabela de teste cabe num setor");
    v.resize(S as usize, 0xFF);
    v
}

/// ISO Xsf (deslocamento 0) com a raiz em `raiz` e cada (setor, bytes) no
/// lugar.
pub(crate) fn gravar_iso(caminho: &Path, raiz: (u32, u32), partes: &[(u32, &[u8])], setores: u64) {
    let mut f = File::create(caminho).unwrap();
    f.set_len(setores * S).unwrap();
    let mut d = vec![0u8; S as usize];
    d[0..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    d[20..24].copy_from_slice(&raiz.0.to_le_bytes());
    d[24..28].copy_from_slice(&raiz.1.to_le_bytes());
    f.seek(SeekFrom::Start(32 * S)).unwrap();
    f.write_all(&d).unwrap();
    for (setor, b) in partes {
        f.seek(SeekFrom::Start(*setor as u64 * S)).unwrap();
        f.write_all(b).unwrap();
    }
}

/// B-1: cada tabela tem oito diretórios apontando para a mesma tabela
/// seguinte, 40 níveis. Antes, cada entrada carregava a sua própria cópia de
/// tudo abaixo dela: 8⁴⁰ tabelas, memória esgotada e aborto.
#[test]
fn b1_tabelas_compartilhadas_esgotam_o_orcamento_em_vez_da_memoria() {
    let t = Temp::nova("b1");
    let niveis = 40u32;
    let mut tabelas = Vec::new();
    for i in 0..niveis {
        let proxima = 100 + i + 1;
        let nomes: Vec<Vec<u8>> = (b'a'..=b'h').map(|c| vec![c]).collect();
        let ents: Vec<(&[u8], u32, u32, u8)> = nomes
            .iter()
            .map(|n| (n.as_slice(), proxima, S as u32, DIR))
            .collect();
        tabelas.push((100 + i, tabela(&ents)));
    }
    tabelas.push((100 + niveis, tabela(&[(b"fim.bin", 0, 0, ARQ)])));
    let partes: Vec<(u32, &[u8])> = tabelas.iter().map(|(s, b)| (*s, b.as_slice())).collect();
    let iso = t.0.join("dag.iso");
    gravar_iso(&iso, (100, S as u32), &partes, 200);

    let inicio = Instant::now();
    let mut gdf = Gdf::abrir(&iso).unwrap();
    let erro = gdf
        .analisar_diretorios()
        .expect_err("a imagem tem que ser recusada");
    assert!(
        erro.to_string().contains("entradas"),
        "mensagem pouco clara: {erro}"
    );
    assert!(
        inicio.elapsed() < Duration::from_secs(60),
        "levou {:?}",
        inicio.elapsed()
    );
}

/// B-2: XBE de 8 MB com dezenas de milhares de seções cujo nome aponta para
/// uma cauda sem zero. Antes, cada seção relia o arquivo até o fim: 239 s.
#[test]
fn b2_xbe_com_muitas_secoes_sem_nome_terminado_responde_rapido() {
    let n = 8 * 1024 * 1024;
    let mut b = vec![0x41u8; n];
    b[0..4].copy_from_slice(b"XBEH");
    let base = 0x1_0000u32;
    b[260..264].copy_from_slice(&base.to_le_bytes());
    b[284..288].copy_from_slice(&u32::MAX.to_le_bytes());
    b[288..292].copy_from_slice(&(base + 0x200).to_le_bytes());
    let mut p = 0x200;
    while p + 56 <= n / 2 {
        b[p + 20..p + 24].copy_from_slice(&(base + (n / 2) as u32).to_le_bytes());
        p += 56;
    }
    let inicio = Instant::now();
    assert!(crate::xbe::extrair_thumbnail(&b).is_err());
    assert!(
        inicio.elapsed() < Duration::from_secs(10),
        "levou {:?}",
        inicio.elapsed()
    );
}

/// B-3: XDBF de 88 bytes declarando 2³² entradas. Antes, 15 s de laço em
/// release a cada leitura.
#[test]
fn b3_xdbf_com_contagem_absurda_responde_rapido() {
    let mut x = vec![0u8; 88];
    x[0..4].copy_from_slice(b"XDBF");
    x[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
    x[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
    let inicio = Instant::now();
    let titulo = crate::xex::xdbf::ler(&x).unwrap();
    assert_eq!(titulo, Default::default());
    assert!(
        inicio.elapsed() < Duration::from_secs(5),
        "levou {:?}",
        inicio.elapsed()
    );
}
