//! Criação de arquivos de saída sem seguir link simbólico.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

/// Cria `caminho` para escrita, vazio, sem seguir links: o que estiver no
/// caminho (um arquivo de uma conversão anterior, ou um link plantado no
/// destino) é apagado antes — `remove_file` apaga o link, nunca o arquivo
/// para onde ele aponta — e o novo é criado com `create_new`, que falha em
/// vez de seguir um link que apareça no meio. Assim a gravação nunca vai
/// parar fora do lugar pedido.
pub fn criar(caminho: &Path) -> io::Result<File> {
    match fs::symlink_metadata(caminho) {
        Ok(m) if m.is_dir() => {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "já existe uma pasta com esse nome",
            ));
        }
        Ok(_) => fs::remove_file(caminho)?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(caminho)
}
