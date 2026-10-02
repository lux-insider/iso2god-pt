//! Windows: o programa num console só dele, como aberto numa janela, para
//! testar o que acontece quando a janela é fechada no meio do trabalho.
//!
//! O console é um pseudoconsole (o mesmo do Windows Terminal), e `fechar` é
//! o X da janela: o Windows manda `CTRL_CLOSE_EVENT` a todos os processos
//! presos a ele. A saída padrão do programa vem por um pipe; a entrada e o
//! erro vão para o `NUL`. Só a API do Windows, sem crate novo.

#![allow(dead_code)]

use std::cell::Cell;
use std::ffi::{OsStr, c_void};
use std::fs::File;
use std::io::{self, PipeReader, Read};
use std::iter::repeat_n;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, IntoRawHandle, RawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Duration;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Coord {
    x: i16,
    y: i16,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reservado: *mut u16,
    area_de_trabalho: *mut u16,
    titulo: *mut u16,
    x: u32,
    y: u32,
    largura: u32,
    altura: u32,
    colunas: u32,
    linhas: u32,
    cores: u32,
    flags: u32,
    mostrar: u16,
    reservado2_tamanho: u16,
    reservado2: *mut u8,
    entrada: Handle,
    saida: Handle,
    erro: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    info: StartupInfoW,
    atributos: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    processo: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}

const STARTF_USESTDHANDLES: u32 = 0x100;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
const HANDLE_FLAG_INHERIT: u32 = 1;
const WAIT_OBJECT_0: u32 = 0;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreatePipe(leitura: *mut Handle, escrita: *mut Handle, seg: *const c_void, tam: u32) -> i32;
    fn CreatePseudoConsole(
        tamanho: Coord,
        entrada: Handle,
        saida: Handle,
        flags: u32,
        console: *mut Handle,
    ) -> i32;
    fn ClosePseudoConsole(console: Handle);
    fn InitializeProcThreadAttributeList(
        lista: *mut c_void,
        quantos: u32,
        flags: u32,
        tamanho: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        lista: *mut c_void,
        flags: u32,
        atributo: usize,
        valor: *const c_void,
        tamanho: usize,
        anterior: *mut c_void,
        retorno: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(lista: *mut c_void);
    fn CreateProcessW(
        aplicativo: *const u16,
        linha: *mut u16,
        seg_processo: *const c_void,
        seg_thread: *const c_void,
        herdar: i32,
        flags: u32,
        ambiente: *const c_void,
        pasta: *const u16,
        inicio: *const StartupInfoExW,
        info: *mut ProcessInformation,
    ) -> i32;
    fn SetHandleInformation(h: Handle, mascara: u32, flags: u32) -> i32;
    fn WaitForSingleObject(h: Handle, milissegundos: u32) -> u32;
    fn GetExitCodeProcess(h: Handle, codigo: *mut u32) -> i32;
    fn TerminateProcess(h: Handle, codigo: u32) -> i32;
    fn CloseHandle(h: Handle) -> i32;
}

/// Um programa rodando no seu próprio console.
pub struct Janela {
    processo: Handle,
    console: Handle,
    fechada: Cell<bool>,
    // a ponta de escrita da entrada do console fica aberta até o fim
    _teclado: File,
}

impl Janela {
    /// Roda `programa args` num console novo. Devolve a janela e a saída
    /// padrão do programa.
    pub fn abrir(programa: &Path, args: &[&OsStr]) -> (Janela, PipeReader) {
        unsafe {
            // o console: o que ele "desenha" é lido e descartado numa thread
            let (mut entrada_console, mut teclado) = (null_mut(), null_mut());
            let (mut tela, mut saida_console) = (null_mut(), null_mut());
            conferir(
                CreatePipe(&mut entrada_console, &mut teclado, null(), 0),
                "CreatePipe",
            );
            conferir(
                CreatePipe(&mut tela, &mut saida_console, null(), 0),
                "CreatePipe",
            );
            let mut console = null_mut();
            let r = CreatePseudoConsole(
                Coord { x: 120, y: 40 },
                entrada_console,
                saida_console,
                0,
                &mut console,
            );
            assert_eq!(r, 0, "CreatePseudoConsole: {r:#x}");
            CloseHandle(entrada_console);
            CloseHandle(saida_console);
            let mut tela = File::from_raw_handle(tela as RawHandle);
            std::thread::spawn(move || {
                let mut lixo = [0u8; 4096];
                while matches!(tela.read(&mut lixo), Ok(n) if n > 0) {}
            });

            // a saída padrão do programa é um pipe; a entrada e o erro, NUL
            let (leitor, escritor) = io::pipe().unwrap();
            let escritor = escritor.into_raw_handle() as Handle;
            let nulo = File::options()
                .read(true)
                .write(true)
                .open("NUL")
                .unwrap()
                .into_raw_handle() as Handle;
            for h in [escritor, nulo] {
                conferir(
                    SetHandleInformation(h, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT),
                    "SetHandleInformation",
                );
            }

            let mut tamanho = 0usize;
            InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut tamanho);
            // alinhada como ponteiro, como a API espera
            let mut memoria = vec![0usize; tamanho.div_ceil(size_of::<usize>())];
            let lista = memoria.as_mut_ptr() as *mut c_void;
            conferir(
                InitializeProcThreadAttributeList(lista, 1, 0, &mut tamanho),
                "InitializeProcThreadAttributeList",
            );
            conferir(
                UpdateProcThreadAttribute(
                    lista,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    console,
                    size_of::<Handle>(),
                    null_mut(),
                    null_mut(),
                ),
                "UpdateProcThreadAttribute",
            );
            let mut inicio: StartupInfoExW = zeroed();
            inicio.info.cb = size_of::<StartupInfoExW>() as u32;
            inicio.info.flags = STARTF_USESTDHANDLES;
            inicio.info.entrada = nulo;
            inicio.info.saida = escritor;
            inicio.info.erro = nulo;
            inicio.atributos = lista;
            let mut linha = linha_de_comando(programa, args);
            let mut info: ProcessInformation = zeroed();
            let criado = CreateProcessW(
                null(),
                linha.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT,
                null(),
                null(),
                &inicio,
                &mut info,
            );
            let erro = io::Error::last_os_error();
            DeleteProcThreadAttributeList(lista);
            CloseHandle(escritor);
            CloseHandle(nulo);
            assert!(criado != 0, "CreateProcessW: {erro}");
            CloseHandle(info.thread);
            let janela = Janela {
                processo: info.processo,
                console,
                fechada: Cell::new(false),
                _teclado: File::from_raw_handle(teclado as RawHandle),
            };
            (janela, leitor)
        }
    }

    /// Fecha o console, como o X da janela. O `ClosePseudoConsole` pode
    /// esperar os processos saírem, então roda numa thread.
    pub fn fechar(&self) {
        if self.fechada.replace(true) {
            return;
        }
        let console = self.console as usize;
        std::thread::spawn(move || unsafe { ClosePseudoConsole(console as Handle) });
    }

    /// O código de saída, se o programa terminou dentro do prazo.
    pub fn esperar(&self, prazo: Duration) -> Option<u32> {
        unsafe {
            if WaitForSingleObject(self.processo, prazo.as_millis() as u32) != WAIT_OBJECT_0 {
                return None;
            }
            let mut codigo = 0;
            conferir(
                GetExitCodeProcess(self.processo, &mut codigo),
                "GetExitCodeProcess",
            );
            Some(codigo)
        }
    }
}

impl Drop for Janela {
    fn drop(&mut self) {
        unsafe {
            if WaitForSingleObject(self.processo, 0) != WAIT_OBJECT_0 {
                TerminateProcess(self.processo, 1);
            }
            CloseHandle(self.processo);
        }
        self.fechar();
    }
}

fn conferir(ok: i32, funcao: &str) {
    assert!(ok != 0, "{funcao}: {}", io::Error::last_os_error());
}

/// A linha de comando com cada parte entre aspas, nas regras do Windows: as
/// barras invertidas só se dobram antes de uma aspa.
fn linha_de_comando(programa: &Path, args: &[&OsStr]) -> Vec<u16> {
    let barra = u16::from(b'\\');
    let aspa = u16::from(b'"');
    let mut linha = Vec::new();
    for (i, parte) in std::iter::once(programa.as_os_str())
        .chain(args.iter().copied())
        .enumerate()
    {
        if i > 0 {
            linha.push(u16::from(b' '));
        }
        linha.push(aspa);
        let mut barras = 0;
        for c in parte.encode_wide() {
            if c == barra {
                barras += 1;
            } else {
                if c == aspa {
                    linha.extend(repeat_n(barra, barras + 1));
                }
                barras = 0;
            }
            linha.push(c);
        }
        linha.extend(repeat_n(barra, barras));
        linha.push(aspa);
    }
    linha.push(0);
    linha
}
