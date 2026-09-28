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
    CANCELADO.load(Ordering::SeqCst)
}

/// Limpa o sinalizador. Usado pelo assistente ao voltar para o menu depois
/// de uma conversão cancelada — o próximo item começa do zero.
pub fn limpar_cancelamento() {
    CANCELADO.store(false, Ordering::SeqCst);
}

/// Marca cancelamento sem passar por sinal (usado em testes).
#[cfg(test)]
pub fn marcar_cancelamento() {
    CANCELADO.store(true, Ordering::SeqCst);
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

/// Instala os handlers de Ctrl+C (SIGINT) e SIGTERM. Deliberadamente **sem**
/// `SA_RESTART`: assim um `read` bloqueado num prompt do assistente devolve
/// `EINTR` em vez de ser reiniciado silenciosamente, e quem estiver esperando
/// entrada percebe o cancelamento na hora.
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
    }
}

/// No Windows, Ctrl+C e Ctrl+Break chegam por `SetConsoleCtrlHandler`. Mesma
/// regra do Linux: o primeiro marca o cancelamento (a conversão para no
/// próximo bloco e apaga a saída incompleta); o segundo devolve `FALSE` e o
/// sistema encerra o processo na hora, para quem não quer esperar.
#[cfg(windows)]
unsafe extern "system" fn tratar_console(tipo: u32) -> windows_sys::Win32::Foundation::BOOL {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    if tipo != CTRL_C_EVENT && tipo != CTRL_BREAK_EVENT {
        return 0;
    }
    if CANCELADO.swap(true, Ordering::SeqCst) { 0 } else { 1 }
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
    let largo: Vec<u16> = existente.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut livre: u64 = 0;
    let ok = unsafe { GetDiskFreeSpaceExW(largo.as_ptr(), &mut livre, std::ptr::null_mut(), std::ptr::null_mut()) };
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
    absoluto.ancestors().find(|p| p.exists()).map(Path::to_path_buf)
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn espaco_livre_de_um_caminho_que_existe_e_plausivel() {
        let livre = espaco_livre(Path::new("/")).expect("statvfs de / deveria funcionar");
        assert!(livre > 0, "um sistema de arquivos raiz sem nenhum byte livre é improvável");
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
}
