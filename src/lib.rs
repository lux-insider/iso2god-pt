//! Conversão de imagens ISO de Xbox e Xbox 360 para o formato Games on
//! Demand (GOD).

/// `println!` que não entra em pânico quando a saída padrão sumiu (pipe
/// fechado por quem lia o progresso, terminal fechado). O `println!` da
/// biblioteca padrão entra em pânico nesse caso, e o perfil de release
/// aborta no pânico: a conversão morria no meio e deixava a saída pela
/// metade. Aqui a falha de escrita é ignorada e a conversão segue até o
/// fim (ou até um cancelamento), com a limpeza de sempre.
#[macro_export]
macro_rules! saida {
    () => {
        $crate::terminal::escrever(false, format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::terminal::escrever(false, format_args!("{}\n", format_args!($($arg)*)))
    };
}

/// O mesmo, sem quebra de linha (`print!`).
#[macro_export]
macro_rules! saida_sem_linha {
    ($($arg:tt)*) => {
        $crate::terminal::escrever(false, format_args!($($arg)*))
    };
}

/// O mesmo para a saída de erros (`eprintln!`).
#[macro_export]
macro_rules! saida_erro {
    ($($arg:tt)*) => {
        $crate::terminal::escrever(true, format_args!("{}\n", format_args!($($arg)*)))
    };
}

pub mod analise;
pub mod arquivo;
pub mod assistente;
pub mod cli;
pub mod erro;
pub mod gdf;
pub mod god;
pub mod limpeza;
pub mod progresso;
pub mod sistema;
pub mod terminal;
pub mod xbe;
pub mod xex;

#[cfg(test)]
mod testes_auditoria;
