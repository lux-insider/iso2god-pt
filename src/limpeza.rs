//! O que uma conversão em andamento já criou no disco e precisa sumir se o
//! programa morrer por um pânico.
//!
//! Erro e cancelamento passam pela limpeza normal da conversão. Um pânico
//! não: o perfil de release aborta, sem desenrolar a pilha. Por isso a
//! conversão registra aqui a saída que está criando, e o gancho de pânico
//! (em `main.rs`) apaga o que estiver registrado antes do aborto.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

struct Pendente {
    id: u64,
    caminho: PathBuf,
    /// Pastas-pai que ficarem vazias também saem, até esta (exclusive).
    limite: PathBuf,
}

static PENDENTES: Mutex<Vec<Pendente>> = Mutex::new(Vec::new());
static PROXIMO: AtomicU64 = AtomicU64::new(0);

/// Enquanto vivo, `caminho` é apagado se o programa entrar em pânico. Sai
/// do registro quando é descartado (a conversão terminou, bem ou mal, e já
/// cuidou da própria saída).
pub struct Registro(u64);

impl Drop for Registro {
    fn drop(&mut self) {
        if let Ok(mut v) = PENDENTES.lock() {
            v.retain(|p| p.id != self.0);
        }
    }
}

/// Registra `caminho` (arquivo ou pasta) como saída em andamento; as
/// pastas-pai que ficarem vazias também saem, até `limite`.
pub fn registrar(caminho: &Path, limite: &Path) -> Registro {
    let id = PROXIMO.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut v) = PENDENTES.lock() {
        v.push(Pendente {
            id,
            caminho: caminho.to_path_buf(),
            limite: limite.to_path_buf(),
        });
    }
    Registro(id)
}

/// Apaga tudo que está registrado. Chamado pelo gancho de pânico: não
/// espera pela trava (o pânico pode ter acontecido com ela presa).
pub fn apagar_pendentes() {
    let Ok(mut v) = PENDENTES.try_lock() else {
        return;
    };
    for p in v.drain(..) {
        let apagado = match fs::symlink_metadata(&p.caminho) {
            Ok(m) if m.is_dir() => fs::remove_dir_all(&p.caminho).is_ok(),
            Ok(_) => fs::remove_file(&p.caminho).is_ok(),
            Err(_) => true,
        };
        if !apagado {
            continue;
        }
        let mut pasta = p.caminho.parent();
        while let Some(atual) = pasta {
            if atual == p.limite || fs::remove_dir(atual).is_err() {
                break;
            }
            pasta = atual.parent();
        }
    }
}
