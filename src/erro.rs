use thiserror::Error;

/// Erro unificado para todas as operações da ferramenta.
#[derive(Debug, Error)]
pub enum Erro {
    #[error("erro de E/S: {0}")]
    Io(#[from] std::io::Error),

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
