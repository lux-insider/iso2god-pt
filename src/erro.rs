//! Erros do programa, com mensagens em português que dizem o que houve.
//!
//! Uma falha de E/S num arquivo sempre diz **qual arquivo** e **qual
//! operação**: não há conversão automática de `io::Error` para `Erro`, então
//! cada chamada tem que dar o contexto (`.ctx(Operacao::Ler, caminho)`).

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// O que se tentava fazer com o arquivo quando a E/S falhou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operacao {
    Abrir,
    Ler,
    Gravar,
    Criar,
    CriarPasta,
    Apagar,
    Consultar,
}

impl fmt::Display for Operacao {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Operacao::Abrir => "abrir",
            Operacao::Ler => "ler",
            Operacao::Gravar => "gravar",
            Operacao::Criar => "criar",
            Operacao::CriarPasta => "criar a pasta",
            Operacao::Apagar => "apagar",
            Operacao::Consultar => "consultar",
        })
    }
}

/// Erro unificado para todas as operações da ferramenta.
#[derive(Debug, Error)]
pub enum Erro {
    /// Falha de E/S fora de um arquivo (a entrada do teclado no assistente).
    #[error("erro de E/S: {}", Causa(.0))]
    Io(io::Error),

    /// Uma operação de E/S que falhou num arquivo ou pasta conhecida.
    #[error("não foi possível {operacao} {}: {}", caminho.display(), Causa(fonte))]
    Arquivo {
        operacao: Operacao,
        caminho: PathBuf,
        #[source]
        fonte: io::Error,
    },

    #[error("imagem ISO inválida: {0}")]
    IsoInvalida(String),

    #[error("estrutura GDF não encontrada ou corrompida")]
    GdfNaoEncontrada,

    #[error("tabela de hash já contém o máximo de {0} entradas")]
    LimiteHashExcedido(usize),

    #[error("operação ainda não implementada: {0}")]
    NaoImplementado(&'static str),

    /// Espaço insuficiente no destino, detectado ANTES de escrever qualquer
    /// coisa (ver `crate::sistema::espaco_livre`) — um erro claro no início
    /// vale mais que um `ENOSPC` no meio de vários GB já gravados.
    #[error(
        "espaço insuficiente em {destino}: são necessários ~{necessario} e há {disponivel} livres"
    )]
    EspacoInsuficiente {
        destino: String,
        necessario: String,
        disponivel: String,
    },

    /// O usuário pediu cancelamento (Ctrl+C). Não é uma falha da conversão:
    /// quem trata esse erro apaga a saída incompleta e volta ao normal.
    #[error("conversão cancelada pelo usuário")]
    Cancelado,
}

pub type Resultado<T> = std::result::Result<T, Erro>;

/// Dá a uma falha de E/S o arquivo e a operação.
pub trait Contexto<T> {
    fn ctx(self, operacao: Operacao, caminho: &Path) -> Resultado<T>;
}

impl<T> Contexto<T> for io::Result<T> {
    #[inline]
    fn ctx(self, operacao: Operacao, caminho: &Path) -> Resultado<T> {
        match self {
            Ok(v) => Ok(v),
            Err(fonte) => Err(erro_arquivo(operacao, caminho, fonte)),
        }
    }
}

/// Fora da parte genérica: uma cópia só, em vez de uma por tipo de `T`.
#[inline(never)]
pub fn erro_arquivo(operacao: Operacao, caminho: &Path, fonte: io::Error) -> Erro {
    Erro::Arquivo {
        operacao,
        caminho: caminho.to_path_buf(),
        fonte,
    }
}

/// A causa de uma falha de E/S em português, para os casos comuns, com o
/// código do sistema (útil para quem for ajudar); nos outros, a mensagem
/// do próprio sistema.
struct Causa<'a>(&'a io::Error);

impl fmt::Display for Causa<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use io::ErrorKind as K;
        let e = self.0;
        let texto = match e.kind() {
            K::NotFound => "não existe",
            K::PermissionDenied => "sem permissão",
            K::AlreadyExists => "já existe",
            K::StorageFull => "não há espaço no disco",
            K::QuotaExceeded => "a cota de disco do usuário acabou",
            K::FileTooLarge => {
                "arquivo grande demais para este sistema de arquivos (FAT32 aceita até 4 GiB)"
            }
            K::ReadOnlyFilesystem => "o disco é só de leitura",
            K::IsADirectory => "é uma pasta",
            K::NotADirectory => "parte do caminho não é uma pasta",
            K::DirectoryNotEmpty => "a pasta não está vazia",
            K::ResourceBusy | K::ExecutableFileBusy => "o arquivo está em uso por outro programa",
            K::InvalidFilename => "nome de arquivo inválido para este sistema",
            K::UnexpectedEof => "o arquivo terminou antes do esperado (truncado?)",
            K::OutOfMemory => "falta de memória",
            K::Interrupted => "a operação foi interrompida",
            _ => return write!(f, "{e}"),
        };
        match e.raw_os_error() {
            Some(c) => write!(f, "{texto} (erro {c} do sistema)"),
            None => f.write_str(texto),
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn mensagem_diz_arquivo_operacao_e_causa() {
        let e: Resultado<()> = Err(io::Error::from_raw_os_error(2))
            .ctx(Operacao::Abrir, Path::new("/jogos/Halo 3.iso"));
        let m = e.unwrap_err().to_string();
        assert!(
            m.starts_with("não foi possível abrir /jogos/Halo 3.iso: não existe"),
            "{m}"
        );

        // causa sem tradução: a mensagem do sistema
        let e: Resultado<()> =
            Err(io::Error::other("algo estranho")).ctx(Operacao::Ler, Path::new("x"));
        assert_eq!(
            e.unwrap_err().to_string(),
            "não foi possível ler x: algo estranho"
        );
    }
}
