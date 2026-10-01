//! Ponte fina com o sistema operacional para duas coisas que a std ainda não
//! oferece e que a conversão precisa para não quebrar feio:
//!
//! - **Espaço livre no destino** (`statvfs`), para avisar *antes* de começar
//!   que não vai caber — em vez de falhar com `ENOSPC` a 90% de uma
//!   conversão de vários GB, deixando um GOD pela metade.
//! - **Cancelamento limpo no Ctrl+C** (`sigaction` sem `SA_RESTART`), para o
//!   laço de conversão perceber o pedido, parar num ponto conhecido e apagar
//!   a saída incompleta. Sem isso, o Ctrl+C mata o processo no meio de uma
//!   escrita e deixa lixo que parece um pacote válido.
//!
//! No Windows o espaço livre vem de `GetDiskFreeSpaceExW` e o cancelamento de
//! `SetConsoleCtrlHandler`. Em outros sistemas `espaco_livre` devolve `None`
//! (o chamador trata como "não sei dizer" e segue em frente).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Sinalizador global de "o usuário pediu para cancelar". Global porque um
/// handler de sinal não recebe contexto — é a única forma de o Ctrl+C falar
/// com o resto do programa.
static CANCELADO: AtomicBool = AtomicBool::new(false);

/// `true` depois que o usuário apertou Ctrl+C. Consultado entre blocos da
/// conversão (ver `crate::god`), nunca no meio de uma escrita.
pub fn cancelado() -> bool {
    #[cfg(test)]
    if CANCELADO_TESTE.get() {
        return true;
    }
    CANCELADO.load(Ordering::SeqCst)
}

/// Limpa o sinalizador. Usado pelo assistente ao voltar para o menu depois
/// de uma conversão cancelada — o próximo item começa do zero.
pub fn limpar_cancelamento() {
    #[cfg(test)]
    CANCELADO_TESTE.set(false);
    CANCELADO.store(false, Ordering::SeqCst);
}

// Nos testes, o cancelamento simulado vale só para a thread do teste que o
// pediu: os testes rodam em paralelo, e um pedido global faria outro teste
// (que só lê uma árvore, por exemplo) receber `Cancelado` no meio.
#[cfg(test)]
thread_local! {
    static CANCELADO_TESTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Marca cancelamento sem passar por sinal (usado em testes).
#[cfg(test)]
pub fn marcar_cancelamento() {
    CANCELADO_TESTE.set(true);
}

#[cfg(unix)]
extern "C" fn tratar_sigint(_sinal: libc::c_int) {
    // `swap` devolve o valor anterior: se já estava marcado, este é o
    // segundo Ctrl+C — o usuário quer sair agora, não esperar o cancelamento
    // chegar no próximo ponto seguro. `_exit` é uma das poucas chamadas
    // permitidas dentro de um handler de sinal (não roda destrutores nem
    // libera buffers, justamente por isso é segura aqui).
    if CANCELADO.swap(true, Ordering::SeqCst) {
        unsafe { libc::_exit(130) };
    }
}

/// SIGTERM é o pedido educado de outro programa para encerrar (ex.: o
/// xiso-manager, depois de um Ctrl+C, ou um `kill` sem opção). Ele cancela do
/// mesmo jeito limpo, mas **nunca** vira a saída imediata do segundo Ctrl+C:
/// quem manda SIGTERM logo depois de um SIGINT quer que a limpeza termine,
/// não que ela seja interrompida no meio e deixe um GOD pela metade.
#[cfg(unix)]
extern "C" fn tratar_sigterm(_sinal: libc::c_int) {
    CANCELADO.store(true, Ordering::SeqCst);
}

/// Instala os handlers de Ctrl+C (SIGINT), SIGTERM e SIGHUP. Deliberadamente
/// **sem** `SA_RESTART`: assim um `read` bloqueado num prompt do assistente
/// devolve `EINTR` em vez de ser reiniciado silenciosamente, e quem estiver
/// esperando entrada percebe o cancelamento na hora.
///
/// SIGHUP chega quando o terminal é fechado; sem tratador, ele matava o
/// processo no meio e deixava a saída pela metade. Ele cancela como o
/// SIGTERM — a não ser que já chegue ignorado (`nohup`): quem rodou assim
/// quer que a operação continue sem o terminal.
#[cfg(unix)]
pub fn instalar_cancelamento() {
    unsafe fn instalar(sinal: libc::c_int, tratador: extern "C" fn(libc::c_int)) {
        unsafe {
            let mut acao: libc::sigaction = std::mem::zeroed();
            acao.sa_sigaction = tratador as *const () as usize;
            acao.sa_flags = 0;
            libc::sigemptyset(&mut acao.sa_mask);
            libc::sigaction(sinal, &acao, std::ptr::null_mut());
        }
    }
    unsafe {
        instalar(libc::SIGINT, tratar_sigint);
        instalar(libc::SIGTERM, tratar_sigterm);
        let mut atual: libc::sigaction = std::mem::zeroed();
        let ignorado = libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut atual) == 0
            && atual.sa_sigaction == libc::SIG_IGN;
        if !ignorado {
            instalar(libc::SIGHUP, tratar_sigterm);
        }
    }
}

/// No Windows, Ctrl+C e Ctrl+Break chegam por `SetConsoleCtrlHandler`. Mesma
/// regra do Linux: o primeiro marca o cancelamento (a conversão para no
/// próximo bloco e apaga a saída incompleta); o segundo devolve `FALSE` e o
/// sistema encerra o processo na hora, para quem não quer esperar.
///
/// Fechar a janela do console, sair da sessão ou desligar chegam como
/// `CTRL_CLOSE_EVENT`, `CTRL_LOGOFF_EVENT` e `CTRL_SHUTDOWN_EVENT`. Para
/// eles o Windows encerra o processo assim que o tratador volta (e, de
/// qualquer jeito, uns 5 s depois). Então o tratador marca o cancelamento e
/// espera: nesse tempo a thread principal vê o pedido, apaga o que a
/// operação criou e sai pelo `process::exit`, que encerra o processo (e
/// esta espera) antes do prazo.
#[cfg(windows)]
unsafe extern "system" fn tratar_console(tipo: u32) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };
    match tipo {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => {
            if CANCELADO.swap(true, Ordering::SeqCst) {
                0
            } else {
                1
            }
        }
        CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT | CTRL_SHUTDOWN_EVENT => {
            CANCELADO.store(true, Ordering::SeqCst);
            let inicio = std::time::Instant::now();
            while inicio.elapsed() < std::time::Duration::from_millis(4500) {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            1
        }
        _ => 0,
    }
}

#[cfg(windows)]
pub fn instalar_cancelamento() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    unsafe {
        SetConsoleCtrlHandler(Some(tratar_console), 1);
    }
}

#[cfg(not(any(unix, windows)))]
pub fn instalar_cancelamento() {}

/// Espaço livre, em bytes, disponível para um usuário comum no sistema de
/// arquivos que contém `caminho`. Sobe pelos diretórios-pai até achar um que
/// exista (o destino normalmente ainda não foi criado quando perguntamos).
///
/// `None` significa "não foi possível descobrir" — nunca "não há espaço": o
/// chamador segue com a conversão em vez de bloquear por falta de
/// informação.
#[cfg(unix)]
pub fn espaco_livre(caminho: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let existente = ancestral_existente(caminho)?;
    let c_caminho = CString::new(existente.as_os_str().as_bytes()).ok()?;

    unsafe {
        let mut info: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_caminho.as_ptr(), &mut info) != 0 {
            return None;
        }
        // f_bavail (e não f_bfree) é o que sobra para quem não é root: em
        // ext4 uma fatia do disco fica reservada e só o root pode usar.
        Some(info.f_bavail as u64 * info.f_frsize as u64)
    }
}

/// No Windows, `GetDiskFreeSpaceExW` já devolve o espaço disponível para o
/// usuário atual (respeitando cotas), o equivalente ao `f_bavail`.
#[cfg(windows)]
pub fn espaco_livre(caminho: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let existente = ancestral_existente(caminho)?;
    let largo: Vec<u16> = existente
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut livre: u64 = 0;
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            largo.as_ptr(),
            &mut livre,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(livre)
}

#[cfg(not(any(unix, windows)))]
pub fn espaco_livre(_caminho: &Path) -> Option<u64> {
    None
}

/// Primeiro caminho existente na cadeia `caminho`, pai, avô... Necessário
/// porque `statvfs` falha em caminho inexistente, e a pasta de destino
/// costuma ser justamente a que ainda vai ser criada.
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
fn ancestral_existente(caminho: &Path) -> Option<std::path::PathBuf> {
    let absoluto = if caminho.is_absolute() {
        caminho.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(caminho)
    };
    absoluto
        .ancestors()
        .find(|p| p.exists())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn espaco_livre_de_um_caminho_que_existe_e_plausivel() {
        let livre = espaco_livre(Path::new("/")).expect("statvfs de / deveria funcionar");
        assert!(
            livre > 0,
            "um sistema de arquivos raiz sem nenhum byte livre é improvável"
        );
    }

    #[test]
    fn espaco_livre_sobe_ate_um_ancestral_existente() {
        // Nada disso existe, mas a raiz existe — a resposta deve vir de lá.
        let inventado = std::env::temp_dir().join("iso2god_nao_existe/nem_isto/nem_aquilo");
        assert!(espaco_livre(&inventado).is_some());
    }

    #[test]
    fn cancelamento_comeca_desligado_e_pode_ser_limpo() {
        limpar_cancelamento();
        assert!(!cancelado());
        marcar_cancelamento();
        assert!(cancelado());
        limpar_cancelamento();
        assert!(!cancelado());
    }

    /// S-2: SIGHUP (terminal fechado) cancela como o SIGTERM, mas não quando
    /// já chega ignorado (`nohup`).
    #[cfg(unix)]
    #[test]
    fn s2_sighup_cancela_mas_respeita_nohup() {
        // Confere o tratador instalado, sem disparar o sinal: o
        // cancelamento é global e outros testes rodam em paralelo. O sinal
        // de verdade é testado com o programa em tests/cli_auditoria.rs.
        fn tratador_do_sighup() -> libc::sighandler_t {
            unsafe {
                let mut atual: libc::sigaction = std::mem::zeroed();
                libc::sigaction(libc::SIGHUP, std::ptr::null(), &mut atual);
                atual.sa_sigaction
            }
        }
        unsafe {
            // como o nohup deixa
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
            instalar_cancelamento();
        }
        assert_eq!(
            tratador_do_sighup(),
            libc::SIG_IGN,
            "com nohup, fica ignorado"
        );
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_DFL);
            instalar_cancelamento();
        }
        let esperado = tratar_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
        assert_eq!(tratador_do_sighup(), esperado, "SIGHUP deveria cancelar");
    }
}
