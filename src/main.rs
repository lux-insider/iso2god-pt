use clap::Parser;

use iso2god::saida;

use iso2god::analise;
use iso2god::assistente;
use iso2god::cli::{self, Cli, Comando};
use iso2god::erro::{Erro, Resultado};
use iso2god::god;
use iso2god::progresso;
use iso2god::sistema;
use iso2god::terminal::{self, Tema};

/// Código de saída para "interrompido pelo usuário", pela convenção de shell
/// de 128 + número do sinal (SIGINT = 2).
const SAIDA_CANCELADO: i32 = 130;

fn main() {
    // Antes de qualquer coisa: a partir daqui o Ctrl+C vira um pedido de
    // cancelamento tratável (ver `iso2god::sistema`), e não a morte imediata
    // do processo no meio de uma escrita.
    sistema::instalar_cancelamento();
    // No Windows: UTF-8 e cores ANSI no console (ver `terminal::preparar_console`).
    terminal::preparar_console();

    let cli = Cli::parse();
    let progresso_json = matches!(&cli.comando, Some(Comando::Converter(a)) if a.progresso_json);
    instalar_gancho_de_panico(progresso_json);

    match executar(cli) {
        Ok(codigo) => std::process::exit(codigo),
        Err(Erro::Cancelado) => {
            progresso::relatar_erro_final("Conversão cancelada.", progresso_json);
            std::process::exit(SAIDA_CANCELADO);
        }
        Err(erro) => {
            progresso::relatar_erro_final(&erro.to_string(), progresso_json);
            std::process::exit(1);
        }
    }
}

/// Um pânico é um defeito: em vez da mensagem padrão em inglês ("thread
/// 'main' panicked at..."), uma mensagem em português com o local, o evento
/// `erro` para quem lê o `--progresso-json`, e a saída incompleta apagada
/// (o perfil de release aborta no pânico, sem passar pela limpeza normal da
/// conversão). O código de saída continua o de um pânico.
fn instalar_gancho_de_panico(json: bool) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RELATADO: AtomicBool = AtomicBool::new(false);
    std::panic::set_hook(Box::new(move |info| {
        // Um pânico numa thread de conversão vira, em depuração, um
        // segundo pânico na thread principal: basta relatar o primeiro.
        if !RELATADO.swap(true, Ordering::SeqCst) {
            let onde = info
                .location()
                .map(|l| format!(" ({}:{})", l.file(), l.line()))
                .unwrap_or_default();
            let texto = format!(
                "erro interno: {}{onde}. Isto é um defeito do iso2god-pt; por favor, relate em \
                 https://github.com/lux-insider/iso2god-pt/issues",
                mensagem_do_panico(info.payload())
            );
            progresso::relatar_erro_final(&texto, json);
            if json {
                Tema::detectar().erro(&texto);
            }
        }
        iso2god::limpeza::apagar_pendentes();
    }));
}

fn mensagem_do_panico(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("pânico sem mensagem")
}

/// Devolve o código de saída do processo: 0 quando tudo deu certo. O
/// assistente pode devolver 1 mesmo sem erro "de cima" — ele trata a falha
/// de cada ISO individualmente (mostrando o motivo e seguindo para a
/// próxima), então quem chama o programa por script ainda precisa saber que
/// algo não converteu.
fn executar(cli: Cli) -> Resultado<i32> {
    match cli.comando {
        Some(Comando::Converter(args)) => {
            exibir_banner(args.progresso_json);
            comando_converter(args)?;
            Ok(0)
        }
        Some(Comando::Info(args)) => {
            exibir_banner(args.json);
            comando_info(args)?;
            Ok(0)
        }
        None => {
            let falhas = assistente::executar()?;
            Ok(if falhas > 0 { 1 } else { 0 })
        }
    }
}

/// Banner do programa, mostrado uma vez por execução — pulado quando a saída
/// é destinada a outro programa (`--json`/`--progresso-json`), para não
/// misturar decoração com dados.
fn exibir_banner(modo_maquina: bool) {
    if modo_maquina {
        return;
    }
    let tema = Tema::detectar();
    let titulo = format!("iso2god v{}", env!("CARGO_PKG_VERSION"));
    saida!(
        "{}",
        tema.caixa_titulo(&titulo, terminal::emo::APP(), terminal::LARGURA, true)
    );
}

fn comando_converter(args: cli::ArgsConverter) -> Resultado<()> {
    progresso::anunciar_fase(
        "iniciando",
        &format!(
            "Convertendo {} -> {}",
            args.origem.display(),
            args.destino.display()
        ),
        args.progresso_json,
    );

    let opcoes = god::OpcoesConversao {
        origem: args.origem,
        destino: args.destino,
        padding: args.padding,
        numero_disco: args.numero_disco,
        plataforma: args.plataforma,
        title_id: args.title_id,
        media_id: args.media_id,
        titulo: args.titulo,
        disco: args.disco,
        total_discos: args.total_discos,
        plataforma_byte: args.plataforma_byte,
        tipo_executavel_byte: args.tipo_executavel_byte,
        icone: args.icone,
        threads: args.threads,
        progresso_json: args.progresso_json,
    };

    god::converter(&opcoes)
}

fn comando_info(args: cli::ArgsInfo) -> Resultado<()> {
    let tema = Tema::detectar();
    if !args.json {
        tema.info_linha(
            terminal::emo::ANALISAR(),
            &format!("Analisando {}", args.origem.display()),
        );
    }

    let info = analise::analisar(&args.origem)?;

    if args.json {
        saida!("{}", serde_json::to_string(&info).unwrap_or_default());
        return Ok(());
    }

    info.imprimir(&tema);
    Ok(())
}
