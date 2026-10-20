// Módulo compartilhado com o extract-xiso-pt.

//! Ajuda e erros de uso em português.
//!
//! O clap só fala inglês ("Usage", "Print help", "unexpected argument"...).
//! Em vez de reescrever a ajuda à mão, o texto que ele monta passa por esta
//! tabela de trocas: o formato e as cores continuam os do clap, e uma frase
//! que a tabela não conhece aparece como antes, em inglês, sem sumir.

use std::io::IsTerminal;

/// Trocas aplicadas em ordem: as frases mais longas vêm antes das que são
/// pedaço delas (`' found; no more...` antes de `' found`).
const TROCAS: &[(&str, &str)] = &[
    // ajuda
    ("Usage:", "Uso:"),
    ("Commands:", "Comandos:"),
    ("Arguments:", "Argumentos:"),
    ("Options:", "Opções:"),
    ("[OPTIONS]", "[OPÇÕES]"),
    ("<COMMAND>", "<COMANDO>"),
    ("[COMMAND]", "[COMANDO]"),
    ("Print help (see more with '--help')", "Mostra a ajuda (mais detalhes com '--help')"),
    ("Print help (see a summary with '-h')", "Mostra a ajuda (resumo com '-h')"),
    (
        "Print this message or the help of the given subcommand(s)",
        "Mostra esta ajuda ou a de um comando",
    ),
    ("Print help", "Mostra a ajuda"),
    ("Print version", "Mostra a versão"),
    ("[default: ", "[padrão: "),
    ("[possible values: ", "[valores possíveis: "),
    ("[aliases: ", "[apelidos: "),
    ("[subcommands: ", "[comandos: "),
    // erros de uso
    ("error:", "erro:"),
    ("tip:", "dica:"),
    ("For more information, try '", "Para mais informações, use '"),
    ("the following required arguments were not provided:", "faltam argumentos obrigatórios:"),
    ("' found; no more were expected", "' (não se esperava mais nenhum)"),
    ("unexpected value '", "valor inesperado '"),
    ("unexpected argument '", "argumento inesperado '"),
    ("' found", "'"),
    ("to pass '", "para passar '"),
    ("' as a value, use '", "' como valor, use '"),
    ("invalid value '", "valor inválido '"),
    ("a value is required for '", "falta o valor de '"),
    ("' but none was supplied", "'"),
    ("' cannot be used multiple times", "' não pode ser repetida"),
    ("' cannot be used with", "' não pode ser usada junto com"),
    ("the argument '", "a opção '"),
    ("the subcommand '", "o comando '"),
    ("unrecognized subcommand '", "comando desconhecido '"),
    ("' requires a subcommand but one was not provided", "' precisa de um comando"),
    ("a similar argument exists: '", "existe uma opção parecida: '"),
    ("some similar arguments exist: ", "existem opções parecidas: "),
    ("a similar subcommand exists: '", "existe um comando parecido: '"),
    ("some similar subcommands exist: ", "existem comandos parecidos: "),
    ("equal sign is needed when assigning values to '", "use o sinal de igual para dar valor a '"),
    (
        "invalid UTF-8 was detected in one or more arguments",
        "um dos argumentos não é texto UTF-8 válido",
    ),
    ("' for '", "' para '"),
];

/// O texto do clap, com as frases conhecidas em português.
pub fn traduzir(texto: &str) -> String {
    let mut saida = texto.to_string();
    for (de, para) in TROCAS {
        if saida.contains(de) {
            saida = saida.replace(de, para);
        }
    }
    saida
}

/// Mostra a ajuda, a versão ou o erro de uso do clap em português e sai.
///
/// `--help` e `--version` saem com 0. Um erro de uso sai com 1, como os
/// outros erros: o clap usaria 2, que no `verificar` do extract-xiso-pt é o
/// "não confere" (um xiso-manager mais novo passando uma opção que esta
/// versão não conhece leria "imagem modificada").
pub fn sair(erro: clap::Error) -> ! {
    use clap::error::ErrorKind;
    let pedido = matches!(erro.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
    let no_erro = erro.use_stderr();
    let terminal = if no_erro {
        std::io::stderr().is_terminal()
    } else {
        std::io::stdout().is_terminal()
    };
    let sem_cor = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let montado = erro.render();
    let texto = if terminal && !sem_cor {
        montado.ansi().to_string()
    } else {
        montado.to_string()
    };
    crate::terminal::escrever(no_erro, format_args!("{}", traduzir(&texto)));
    std::process::exit(if pedido { 0 } else { 1 });
}

#[cfg(test)]
mod testes {
    use super::traduzir;

    #[test]
    fn erros_de_uso_em_portugues() {
        let texto = "error: unexpected argument '--xyz' found\n\n  tip: to pass '--xyz' as a \
                     value, use '-- --xyz'\n\nUsage: prog listar [OPTIONS] <IMAGEM>\n\nFor more \
                     information, try '--help'.\n";
        assert_eq!(
            traduzir(texto),
            "erro: argumento inesperado '--xyz'\n\n  dica: para passar '--xyz' como valor, use \
             '-- --xyz'\n\nUso: prog listar [OPÇÕES] <IMAGEM>\n\nPara mais informações, use \
             '--help'.\n"
        );
        assert_eq!(
            traduzir("error: invalid value 'x' for '--padding <PADDING>'"),
            "erro: valor inválido 'x' para '--padding <PADDING>'"
        );
        assert_eq!(
            traduzir("error: unexpected value 'b' for 'a' found; no more were expected"),
            "erro: valor inesperado 'b' para 'a' (não se esperava mais nenhum)"
        );
    }

    #[test]
    fn cores_entre_as_aspas_nao_atrapalham() {
        let texto = "\x1b[1m\x1b[31merror:\x1b[0m unexpected argument '\x1b[33m--xyz\x1b[0m' found";
        assert_eq!(
            traduzir(texto),
            "\x1b[1m\x1b[31merro:\x1b[0m argumento inesperado '\x1b[33m--xyz\x1b[0m'"
        );
    }
}
