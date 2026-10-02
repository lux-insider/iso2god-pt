//! Testes do programa de verdade para os achados do AUDITORIA.md que só
//! aparecem fora da biblioteca: saída fechada, sinais, pânico.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const S: u64 = 2048;
const EXE: &str = env!("CARGO_BIN_EXE_iso2god");

#[cfg(windows)]
mod console_windows;

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

/// Converte em segundo plano e devolve o processo depois que a gravação
/// das partes começou (o evento da fase "convertendo" já saiu).
#[cfg(unix)]
fn conversao_em_andamento(iso: &Path, destino: &Path) -> std::process::Child {
    let mut filho = converter(iso, destino)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut leitor = BufReader::new(filho.stdout.take().unwrap());
    let mut linha = String::new();
    while leitor.read_line(&mut linha).unwrap() > 0 {
        if linha.contains("\"progresso\"") {
            break;
        }
        linha.clear();
    }
    // continua lendo em segundo plano para o pipe não encher
    std::thread::spawn(move || std::io::copy(&mut leitor, &mut std::io::sink()));
    filho
}

/// S-2: fechar o terminal (SIGHUP) no meio da conversão. Antes, o processo
/// morria na hora e deixava a parte pela metade; agora cancela como o
/// SIGTERM: apaga a saída e sai com 130.
#[cfg(unix)]
#[test]
fn s2_sighup_cancela_e_apaga_a_saida() {
    let t = Temp::nova("s2");
    let iso = iso_grande(&t.0, 2048);
    let destino = t.0.join("destino");
    let mut filho = conversao_em_andamento(&iso, &destino);
    unsafe { libc::kill(filho.id() as libc::pid_t, libc::SIGHUP) };
    let status = filho.wait().unwrap();
    assert_eq!(status.code(), Some(130), "{status:?}");
    assert!(
        arquivos(&destino).is_empty(),
        "sobrou saída: {:?}",
        arquivos(&destino).iter().map(|a| &a.0).collect::<Vec<_>>()
    );
}

/// S-2 no Windows: fechar a janela no meio da conversão cancela como o
/// Ctrl+C: apaga a saída e sai com 130. O programa roda num console
/// próprio, fechado como pelo X da janela.
#[cfg(windows)]
#[test]
fn s2_janela_fechada_cancela_e_apaga_a_saida() {
    let t = Temp::nova("s2-janela");
    let iso = iso_grande(&t.0, 2048);
    let destino = t.0.join("destino");
    let (janela, saida) = console_windows::Janela::abrir(
        Path::new(EXE),
        &[
            "converter".as_ref(),
            iso.as_os_str(),
            destino.as_os_str(),
            "--plataforma".as_ref(),
            "xbox360".as_ref(),
            "--title-id".as_ref(),
            "4D5308BF".as_ref(),
            "--media-id".as_ref(),
            "AABBCCDD".as_ref(),
            "--progresso-json".as_ref(),
        ],
    );
    // a gravação das partes começou (o primeiro evento de progresso)
    let mut leitor = BufReader::new(saida);
    let mut linha = String::new();
    while leitor.read_line(&mut linha).unwrap() > 0 {
        if linha.contains("\"progresso\"") {
            break;
        }
        linha.clear();
    }
    janela.fechar();
    // o resto da saída é lido até o programa sair, para o pipe não encher
    std::io::copy(&mut leitor, &mut std::io::sink()).unwrap();
    let codigo = janela.esperar(std::time::Duration::from_secs(30));
    assert_eq!(codigo, Some(130));
    assert!(
        arquivos(&destino).is_empty(),
        "sobrou saída: {:?}",
        arquivos(&destino).iter().map(|a| &a.0).collect::<Vec<_>>()
    );
}

/// S-3: converter de novo o mesmo jogo para o mesmo destino e o processo
/// morrer no meio. Antes, o cabeçalho da conversão anterior ficava ao lado
/// das partes novas pela metade: um pacote com cara de pronto, corrompido.
#[cfg(unix)]
#[test]
fn s3_conversao_interrompida_nao_deixa_cabecalho_antigo_com_dados_novos() {
    let t = Temp::nova("s3");
    let destino = t.0.join("destino");
    let cabecalho = destino.join("4D5308BF/00007000/FE40C7D9CF2D599EB911");

    let pequena = t.0.join("pequena");
    fs::create_dir_all(&pequena).unwrap();
    let ok = converter(&iso_grande(&pequena, 8), &destino)
        .output()
        .unwrap();
    assert_eq!(ok.status.code(), Some(0));
    assert!(
        cabecalho.is_file(),
        "a primeira conversão grava o cabeçalho"
    );

    let grande = t.0.join("grande");
    fs::create_dir_all(&grande).unwrap();
    let mut filho = conversao_em_andamento(&iso_grande(&grande, 2048), &destino);
    filho.kill().unwrap();
    filho.wait().unwrap();
    assert!(
        !cabecalho.exists(),
        "o cabeçalho antigo não pode ficar ao lado das partes novas pela metade"
    );
}

/// Espera o processo terminar em até `limite`; se não terminar, mata e
/// falha o teste.
#[cfg(unix)]
fn esperar(
    filho: &mut std::process::Child,
    limite: std::time::Duration,
) -> std::process::ExitStatus {
    let inicio = std::time::Instant::now();
    loop {
        if let Some(status) = filho.try_wait().unwrap() {
            return status;
        }
        if inicio.elapsed() > limite {
            filho.kill().ok();
            filho.wait().ok();
            panic!("o processo não terminou em {limite:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// B-7: `--icone /dev/zero`. Antes, o arquivo era lido inteiro antes de
/// conferir o tamanho: a leitura nunca terminava e esgotava a memória.
#[cfg(unix)]
#[test]
fn b7_icone_sem_fim_e_recusado_sem_ler_tudo() {
    let t = Temp::nova("b7");
    let iso = iso_grande(&t.0, 1);
    let mut filho = converter(&iso, &t.0.join("destino"))
        .args(["--icone", "/dev/zero"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = esperar(&mut filho, std::time::Duration::from_secs(30));
    assert_eq!(status.code(), Some(1));
    let mut saida = String::new();
    std::io::Read::read_to_string(&mut filho.stdout.take().unwrap(), &mut saida).unwrap();
    assert!(
        saida.contains("ícone") && saida.contains("mais de 16"),
        "{saida}"
    );
}

/// E-1: a mensagem de uma falha de E/S diz o arquivo e a operação, em
/// português. Antes: "erro de E/S: No such file or directory (os error 2)".
#[test]
fn e1_erro_de_es_diz_arquivo_e_operacao() {
    let t = Temp::nova("e1");
    let inexistente = t.0.join("nao-existe.iso");
    let saida = converter(&inexistente, &t.0.join("destino"))
        .output()
        .unwrap();
    assert_eq!(saida.status.code(), Some(1));
    let texto = String::from_utf8(saida.stdout).unwrap();
    let esperado = format!(
        "não foi possível abrir {}: não existe",
        inexistente.display()
    );
    // a saída é o --progresso-json: a mensagem vem dentro do JSON, onde as
    // barras invertidas de um caminho do Windows chegam escapadas
    let mensagens: Vec<String> = texto
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| v["mensagem"].as_str().map(str::to_owned))
        .collect();
    assert!(mensagens.iter().any(|m| m.contains(&esperado)), "{texto}");

    // destino que é um arquivo: a pasta do pacote não pode ser criada
    let iso = iso_grande(&t.0, 1);
    let arquivo = t.0.join("destino-arquivo");
    fs::write(&arquivo, b"x").unwrap();
    let saida = converter(&iso, &arquivo).output().unwrap();
    assert_eq!(saida.status.code(), Some(1));
    let texto = String::from_utf8(saida.stdout).unwrap();
    assert!(
        texto.contains("não foi possível criar a pasta") && texto.contains("destino-arquivo"),
        "{texto}"
    );
}

/// E-2: um pânico no meio da gravação. Antes: mensagem padrão do Rust em
/// inglês, nenhum evento `erro` para quem lê o progresso e a parte pela
/// metade no destino. O pânico é provocado por uma variável que só existe
/// na compilação de depuração (a dos testes).
#[cfg(debug_assertions)]
#[test]
fn e2_panico_vira_evento_erro_em_portugues_e_apaga_a_saida() {
    let t = Temp::nova("e2");
    let iso = iso_grande(&t.0, 64);
    let destino = t.0.join("destino");
    let saida = converter(&iso, &destino)
        .env("ISO2GOD_PANICO_DE_TESTE", "1")
        .output()
        .unwrap();
    assert!(!saida.status.success());
    assert_ne!(saida.status.code(), Some(1), "pânico não é um erro comum");

    let stdout = String::from_utf8(saida.stdout).unwrap();
    let erros: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["evento"] == "erro")
        .collect();
    assert_eq!(erros.len(), 1, "um evento erro: {stdout}");
    let mensagem = erros[0]["mensagem"].as_str().unwrap();
    assert!(
        mensagem.starts_with("erro interno: pânico de teste") && mensagem.contains("relate"),
        "{mensagem}"
    );
    let stderr = String::from_utf8(saida.stderr).unwrap();
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(
        fs::read_dir(&destino).map_or(true, |mut d| d.next().is_none()),
        "as pastas do pacote também saem"
    );
    assert!(
        arquivos(&destino).is_empty(),
        "sobrou saída: {:?}",
        arquivos(&destino).iter().map(|a| &a.0).collect::<Vec<_>>()
    );
}

/// P-1: os pacotes de hash compilados para velocidade no perfil de
/// release (o resto continua otimizado para tamanho).
#[test]
fn p1_hashes_otimizados_para_velocidade_no_release() {
    // no Windows o git pode entregar o arquivo com \r\n
    let cargo = include_str!("../Cargo.toml").replace("\r\n", "\n");
    assert!(cargo.contains("[profile.release]\nopt-level = \"z\""));
    for pacote in ["sha1", "digest", "block-buffer", "md-5"] {
        let secao = format!("[profile.release.package.{pacote}]\nopt-level = 3");
        assert!(cargo.contains(&secao), "falta {secao}");
    }
}

/// ISO Xsf com um `default.xbe` mínimo (só o certificado) com `titulo`.
fn iso_com_xbe(pasta: &Path, titulo: &str) -> PathBuf {
    let mut xbe = vec![0u8; 0x400];
    xbe[0..4].copy_from_slice(b"XBEH");
    xbe[260..264].copy_from_slice(&0x1_0000u32.to_le_bytes());
    xbe[280..284].copy_from_slice(&0x1_0200u32.to_le_bytes());
    xbe[0x208..0x20C].copy_from_slice(&0x4D53_0064u32.to_le_bytes());
    let nome: Vec<u8> = titulo.encode_utf16().flat_map(u16::to_le_bytes).collect();
    xbe[0x20C..0x20C + nome.len()].copy_from_slice(&nome);

    let mut raiz = vec![0xFFu8; S as usize];
    raiz[0..4].copy_from_slice(&[0, 0, 0, 0]);
    raiz[4..8].copy_from_slice(&40u32.to_le_bytes());
    raiz[8..12].copy_from_slice(&(xbe.len() as u32).to_le_bytes());
    raiz[12] = 0;
    raiz[13] = 11;
    raiz[14..25].copy_from_slice(b"default.xbe");
    let caminho = pasta.join("xbe.iso");
    let mut f = File::create(&caminho).unwrap();
    f.set_len(60 * S).unwrap();
    let mut d = vec![0u8; S as usize];
    d[0..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    d[20..24].copy_from_slice(&33u32.to_le_bytes());
    d[24..28].copy_from_slice(&(S as u32).to_le_bytes());
    f.seek(SeekFrom::Start(32 * S)).unwrap();
    f.write_all(&d).unwrap();
    f.write_all(&raiz).unwrap();
    f.seek(SeekFrom::Start(40 * S)).unwrap();
    f.write_all(&xbe).unwrap();
    caminho
}

/// T-1: o nome do jogo vem do XBE (ou do XDBF) e ia cru para o terminal:
/// um título com sequências de escape limpava a tela, trocava o título da
/// janela ou disfarçava o texto. O JSON continua com o texto como ele é.
#[test]
fn t1_texto_da_imagem_nao_vai_cru_para_o_terminal() {
    let t = Temp::nova("t1");
    let titulo = "Jogo\u{1b}]0;janela\u{7}\u{1b}[2J\u{202E}fim";
    let iso = iso_com_xbe(&t.0, titulo);

    let saida = Command::new(EXE).arg("info").arg(&iso).output().unwrap();
    assert_eq!(saida.status.code(), Some(0));
    let texto = String::from_utf8(saida.stdout).unwrap();
    assert!(texto.contains("Jogo"), "{texto}");
    for c in ['\u{1b}', '\u{7}', '\u{202E}'] {
        assert!(!texto.contains(c), "{c:?} cru na saída: {texto:?}");
    }
    assert!(texto.contains("\\u{1b}]0;janela"), "{texto}");

    let json = Command::new(EXE)
        .arg("info")
        .arg(&iso)
        .arg("--json")
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(v["titulo"], titulo);
}

/// L-1 e L-2: dependências trocadas por poucas linhas de código.
#[test]
fn l_dependencias_removidas() {
    let cargo = include_str!("../Cargo.toml");
    let dependencias = &cargo[cargo.find("[dependencies]").unwrap()..];
    assert!(!dependencias.contains("terminal_size"), "L-1");
    assert!(!dependencias.contains("base64"), "L-2");
}
