//! Testes do programa de verdade para os achados do AUDITORIA.md que só
//! aparecem fora da biblioteca: saída fechada, sinais, pânico.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const S: u64 = 2048;
const EXE: &str = env!("CARGO_BIN_EXE_iso2god");

struct Temp(PathBuf);

impl Temp {
    fn nova(nome: &str) -> Self {
        let p = std::env::temp_dir().join(format!("iso2god-cli-{nome}-{}", std::process::id()));
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

/// ISO Xsf com um arquivo `grande.bin` de `mib` MiB (esparso) e nenhum
/// executável: a conversão precisa de `--plataforma` e dos IDs, e avisa em
/// stderr que não achou o `default.xex`.
fn iso_grande(pasta: &Path, mib: u32) -> PathBuf {
    let tamanho = mib * 1024 * 1024;
    let mut raiz = vec![0xFFu8; S as usize];
    let nome = b"grande.bin";
    raiz[0..4].copy_from_slice(&[0, 0, 0, 0]);
    raiz[4..8].copy_from_slice(&40u32.to_le_bytes());
    raiz[8..12].copy_from_slice(&tamanho.to_le_bytes());
    raiz[12] = 0;
    raiz[13] = nome.len() as u8;
    raiz[14..14 + nome.len()].copy_from_slice(nome);
    let caminho = pasta.join("grande.iso");
    let mut f = File::create(&caminho).unwrap();
    f.set_len(40 * S + tamanho as u64).unwrap();
    let mut d = vec![0u8; S as usize];
    d[0..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    d[20..24].copy_from_slice(&33u32.to_le_bytes());
    d[24..28].copy_from_slice(&(S as u32).to_le_bytes());
    f.seek(SeekFrom::Start(32 * S)).unwrap();
    f.write_all(&d).unwrap();
    f.write_all(&raiz).unwrap();
    // algum conteúdo de verdade no começo e no fim do arquivo
    f.seek(SeekFrom::Start(40 * S)).unwrap();
    f.write_all(&[0x5A; 4096]).unwrap();
    f.seek(SeekFrom::Start(40 * S + tamanho as u64 - 4096))
        .unwrap();
    f.write_all(&[0xA5; 4096]).unwrap();
    caminho
}

fn converter(iso: &Path, destino: &Path) -> Command {
    let mut c = Command::new(EXE);
    c.arg("converter")
        .arg(iso)
        .arg(destino)
        .args(["--plataforma", "xbox360", "--title-id", "4D5308BF"])
        .args(["--media-id", "AABBCCDD", "--progresso-json"]);
    c
}

/// Tamanho e conteúdo de cada arquivo gerado, para comparar duas saídas.
fn arquivos(raiz: &Path) -> Vec<(String, Vec<u8>)> {
    fn rec(raiz: &Path, pasta: &Path, v: &mut Vec<(String, Vec<u8>)>) {
        for e in fs::read_dir(pasta).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                rec(raiz, &p, v);
            } else {
                let rel = p.strip_prefix(raiz).unwrap().to_string_lossy().into_owned();
                v.push((rel, fs::read(&p).unwrap()));
            }
        }
    }
    let mut v = Vec::new();
    if raiz.exists() {
        rec(raiz, raiz, &mut v);
    }
    v.sort();
    v
}

/// S-1: quem lê o progresso fecha o pipe (e o stderr também some). Antes,
/// o primeiro `println!` depois disso entrava em pânico: em release o
/// processo abortava e deixava a parte pela metade no destino.
#[test]
fn s1_saida_fechada_nao_interrompe_a_conversao() {
    let t = Temp::nova("s1");
    let iso = iso_grande(&t.0, 64);

    let referencia = t.0.join("referencia");
    let ok = converter(&iso, &referencia).output().unwrap();
    assert_eq!(ok.status.code(), Some(0));

    let destino = t.0.join("destino");
    let mut filho = converter(&iso, &destino)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // lê o primeiro evento e fecha as duas saídas
    let mut primeira = String::new();
    BufReader::new(filho.stdout.take().unwrap())
        .read_line(&mut primeira)
        .unwrap();
    assert!(primeira.contains("\"evento\""), "{primeira}");
    drop(filho.stderr.take());
    let status = filho.wait().unwrap();

    assert_eq!(status.code(), Some(0), "a conversão tem que ir até o fim");
    assert!(
        arquivos(&destino) == arquivos(&referencia),
        "o pacote tem que ficar igual ao de uma conversão normal"
    );
}
