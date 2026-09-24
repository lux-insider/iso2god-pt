//! Modo interativo: rodar o binário sem nenhum subcomando abre um menu, na
//! mesma linha visual do `xiso-manager.py` (referência de UX da casa, não de
//! código) — menu agrupado com faixas do gradiente azul→verde, navegador de
//! pastas, tabelas e assistente passo a passo.
//!
//! Tudo aqui é apresentação: a conversão em si é a mesma `god::converter`
//! usada pelo comando `converter` orientado a flags, com as mesmas opções.
//!
//! Decisões de forma que valem explicação:
//!
//! - **Não limpa a tela entre passos.** Um bloco de 66 colunas cercado de
//!   terminal vazio parece uma janelinha perdida; deixando o conteúdo fluir,
//!   o histórico do terminal continua servindo para conferir o que passou.
//! - **Máquina de etapas, não uma sequência de perguntas.** Cada passo pode
//!   devolver "voltar" ou "sair", então errar o destino não obriga a
//!   recomeçar nem a matar o processo.
//! - **Tudo que vale erro é checado antes de confirmar** (imagem legível,
//!   destino gravável, espaço livre), porque descobrir tarde aqui custa uma
//!   conversão de vários GB.

use std::io::{self, ErrorKind, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::analise::{self, InfoIso};
use crate::cli::RemocaoPadding;
use crate::erro::{Erro, Resultado};
use crate::gdf;
use crate::god::{self, OpcoesConversao};
use crate::sistema;
use crate::terminal::{c, cortar, emo, encurtar_home, fmt_bytes, fmt_tempo, Tema, ITA, LARGURA, SIM_SETA};

/// Tamanho de setor da GDF, usado para converter "último setor ocupado" em
/// bytes na estimativa de espaço (o mesmo valor fixo de `crate::gdf`).
const TAMANHO_SETOR_GDF: u64 = 2048;

/// Etapas do assistente depois que a origem já foi escolhida.
const TOTAL_ETAPAS: u32 = 4;

/// Máximo de subpastas listadas por vez no navegador. Uma pasta com centenas
/// de subpastas viraria uma parede de texto sem nenhuma ajuda a mais.
const MAX_SUBPASTAS: usize = 30;

/// Ponto de entrada do assistente (chamado quando o binário roda sem
/// subcomando). Requer um terminal interativo de verdade — é feito de
/// prompts, então sem TTY na entrada não há como funcionar.
///
/// Devolve quantas conversões **não** deram certo, para o processo poder sair
/// com código diferente de zero: quem chama o programa de um script precisa
/// saber disso mesmo que cada falha já tenha aparecido na tela.
pub fn executar() -> Resultado<u32> {
    if !io::stdin().is_terminal() {
        return Err(Erro::IsoInvalida(
            "modo interativo requer um terminal (stdin não é um TTY). Use `iso2god converter <ISO> <DESTINO>` ou `iso2god info <ISO>`.".to_string(),
        ));
    }

    let tema = Tema::detectar();
    let mut falhas = 0u32;

    loop {
        // Um Ctrl+C que sobrou de uma ação anterior não pode cancelar a
        // próxima antes mesmo de ela começar.
        sistema::limpar_cancelamento();
        mostrar_menu(&tema);

        let escolha = match ler_linha(&tema, "Escolha uma opção")? {
            Entrada::Texto(t) => t.to_lowercase(),
            Entrada::Voltar => continue,
            Entrada::Sair => break,
        };

        match escolha.as_str() {
            "0" => break,
            "1" => falhas += acao_converter(&tema, Selecao::Escolher)?,
            "2" => falhas += acao_converter(&tema, Selecao::PastaInteira)?,
            "3" => acao_analisar(&tema)?,
            "s" | "sobre" => {
                tela_sobre(&tema);
                pausar(&tema)?;
            }
            "" => {}
            _ => {
                tema.aviso("Opção inválida.");
                pausar(&tema)?;
            }
        }
    }

    println!("\n  {}  {}\n", emo::SAIR(), tema.texto_gradiente("Até a próxima!", 0.0));
    Ok(falhas)
}

// ==================================================================== menu

/// Uma opção do menu: tecla, emoji, rótulo e a descrição em cinza.
type ItemMenu = (&'static str, &'static str, &'static str, &'static str);

/// Um grupo do menu: título, nota à direita da divisória e seus itens.
type Grupo<'a> = (&'static str, &'static str, &'a [ItemMenu]);

fn mostrar_menu(tema: &Tema) {
    println!(
        "{}",
        tema.caixa_titulo_com(
            &format!("iso2god  ·  v{}", env!("CARGO_PKG_VERSION")),
            emo::APP(),
            LARGURA,
            true,
            true,
        )
    );

    // Duas informações que mudam a decisão de quem está na frente do menu:
    // quantas threads faz sentido pedir, e se há espaço para o pacote.
    let nucleos = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let mut estado = tema.ponto_status(true, &format!("{nucleos} núcleo(s)"));
    if let Some(casa) = crate::terminal::pasta_pessoal() {
        if let Some(livre) = sistema::espaco_livre(&casa) {
            estado.push_str(&tema.ponto_status(
                true,
                &format!("{} livres em {}", fmt_bytes(livre), encurtar_home(&casa)),
            ));
        }
    }
    println!("\n{estado}\n");
    println!("{}", tema.regua_gradiente(LARGURA));

    let converter = [
        ("1", emo::APP(), "ISO para GOD", "assistente guiado"),
        ("2", emo::VELOCIDADE(), "Lote: uma pasta inteira", "todas as ISOs de uma vez"),
    ];
    let inspecionar = [("3", emo::ANALISAR(), "Analisar ISO", "sem converter")];
    let sistema = [("s", emo::ESTRELA(), "Sobre / ajuda", "")];
    let grupos: [Grupo<'_>; 3] = [
        ("Converter", "GOD", &converter),
        ("Inspecionar", "", &inspecionar),
        ("Sistema", "", &sistema),
    ];

    let total = grupos.len() as f64;
    for (indice, (titulo, nota, itens)) in grupos.iter().enumerate() {
        let ini = indice as f64 / total;
        let fim = (indice + 1) as f64 / total;
        println!("\n{}", tema.cabeca_grupo(titulo, nota, (ini, fim)));
        // a chave de cada item fica no meio da faixa do seu grupo, então a
        // cor também diz a que seção a linha pertence
        let cor_chave = tema.cor_do_gradiente((ini + fim) / 2.0);
        for (tecla, emoji, rotulo, descricao) in itens.iter() {
            println!("{}", tema.opcao_com(tecla, rotulo, emoji, cor_chave.as_deref(), descricao));
        }
    }

    println!("\n{}", tema.opcao("0", "Sair", emo::SAIR(), Some(&c::vermelho())));
    println!("\n{}", tema.regua(LARGURA));
}

/// Cabeçalho de uma tela de ação: caixa de título e uma linha explicando o
/// que ela faz, para a tela não depender de o usuário lembrar do menu.
fn tela(tema: &Tema, titulo: &str, emoji: &str, descricao: &str) {
    println!("{}", tema.caixa_titulo(titulo, emoji, LARGURA, true));
    println!("{}", tema.c(&format!("  {descricao}"), &[&c::cinza(), ITA]));
}

fn tela_sobre(tema: &Tema) {
    tela(
        tema,
        "Sobre o iso2god",
        emo::ESTRELA(),
        "Converte imagens ISO de Xbox/Xbox 360 em containers GOD.",
    );
    println!();
    println!("{}", tema.campo("Versão", env!("CARGO_PKG_VERSION"), emo::APP(), 18));
    println!("{}", tema.campo("Formato de saída", "GOD (Games on Demand), cabeçalho LIVE", emo::DADOS(), 18));
    println!("{}", tema.campo("Detecção", "Title ID, Media ID e capa vêm da própria ISO", emo::ANALISAR(), 18));

    println!("{}", tema.etapa(None, "Mesmas operações pela linha de comando", None));
    for (comando, descricao) in [
        ("iso2god converter <ISO> <DESTINO>", "conversão direta"),
        ("iso2god info <ISO>", "só a análise"),
        ("iso2god converter ... --progresso-json", "progresso em JSON, para scripts"),
        ("iso2god converter --help", "todas as flags"),
    ] {
        println!("{}", tema.campo(comando, descricao, "", 42));
    }

    println!("{}", tema.etapa(None, "Estratégias de padding", None));
    println!("{}", tema.opcao_com("1", "Nenhuma ", "", None, "converte o volume inteiro, como está"));
    println!("{}", tema.opcao_com("2", "Parcial ", "", None, "remove o padding do final (recomendado)"));
    println!("{}", tema.opcao_com("3", "Completa", "", None, "reconstrói a GDF: menor arquivo, mais lento"));
}

/// Pausa entre telas: sem isso o resultado de uma ação sobe junto com o menu
/// seguinte e o usuário não tem chance de ler.
fn pausar(tema: &Tema) -> Resultado<()> {
    print!(
        "\n  {} {}",
        tema.c(SIM_SETA, &[&c::cinza()]),
        tema.c("Enter para continuar", &[&c::cinza(), ITA])
    );
    io::stdout().flush()?;
    let mut lixo = String::new();
    match io::stdin().read_line(&mut lixo) {
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::Interrupted => sistema::limpar_cancelamento(),
        Err(e) => return Err(e.into()),
    }
    println!();
    Ok(())
}

// ===================================================== entrada do usuário

/// O que o usuário digitou num prompt, já interpretado: além do texto, as
/// duas saídas de emergência que todo passo entende.
enum Entrada {
    Texto(String),
    Voltar,
    Sair,
}

/// Resultado de uma etapa do assistente.
enum Passo<T> {
    Ok(T),
    Voltar,
    Sair,
}

enum Resposta {
    Sim,
    Nao,
    Sair,
}

/// Lê uma linha tratando as três formas de o usuário pedir para parar:
/// digitar `q`/`sair`, apertar Ctrl+D (fim de entrada) ou Ctrl+C (que, com o
/// handler de `crate::sistema` instalado sem `SA_RESTART`, faz esta leitura
/// voltar com `EINTR` em vez de ficar pendurada).
fn ler_linha(tema: &Tema, pergunta: &str) -> Resultado<Entrada> {
    print!("{}", tema.prompt(pergunta));
    io::stdout().flush()?;

    let mut linha = String::new();
    match io::stdin().read_line(&mut linha) {
        Ok(0) => {
            println!();
            return Ok(Entrada::Sair);
        }
        Ok(_) => {}
        Err(e) if e.kind() == ErrorKind::Interrupted => {
            sistema::limpar_cancelamento();
            println!();
            return Ok(Entrada::Sair);
        }
        Err(e) => return Err(e.into()),
    }

    let texto = linha.trim().to_string();
    Ok(match texto.to_lowercase().as_str() {
        "q" | "sair" | "quit" => Entrada::Sair,
        "v" | "voltar" => Entrada::Voltar,
        _ => Entrada::Texto(texto),
    })
}

fn perguntar_sim_nao(tema: &Tema, pergunta: &str, padrao_sim: bool) -> Resultado<Resposta> {
    let sufixo = if padrao_sim { "[S/n]" } else { "[s/N]" };
    let entrada = ler_linha(tema, &format!("{pergunta} {sufixo}"))?;
    let texto = match entrada {
        Entrada::Sair => return Ok(Resposta::Sair),
        // "voltar" não tem sentido numa pergunta de sim/não: vale como "não".
        Entrada::Voltar => return Ok(Resposta::Nao),
        Entrada::Texto(t) => t,
    };

    let escolha = match texto.to_lowercase().as_str() {
        "" => padrao_sim,
        "s" | "sim" | "y" | "yes" => true,
        _ => false,
    };
    println!("{}", tema.resposta(if escolha { "sim" } else { "não" }));
    Ok(if escolha { Resposta::Sim } else { Resposta::Nao })
}

/// Limpa um caminho digitado ou colado no terminal. Cobre os três jeitos de
/// um caminho válido chegar aqui "quebrado":
///
/// - `~/Jogos/x.iso` — o shell expande `~`, mas quem digita no nosso prompt
///   não passa por shell nenhum;
/// - `'/mnt/HD Externo/x.iso'` — arrastar o arquivo para o terminal cola com
///   aspas em vários ambientes;
/// - `/mnt/HD\ Externo/x.iso` — e em outros, com os espaços escapados.
pub fn normalizar_caminho(entrada: &str) -> PathBuf {
    let texto = entrada.trim();

    // Aspas ao redor: o conteúdo é literal, sem desescapar nada.
    for aspa in ['"', '\''] {
        if texto.len() >= 2 && texto.starts_with(aspa) && texto.ends_with(aspa) {
            return expandir_til(&texto[1..texto.len() - 1]);
        }
    }

    // Sem aspas: desfaz os escapes de shell (`\ `, `\(`, `\'`...), mantendo a
    // barra invertida quando ela vem antes de letra/dígito — aí faz parte do
    // nome do arquivo, não é escape.
    let mut limpo = String::with_capacity(texto.len());
    let mut chars = texto.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some(&proximo) if !proximo.is_alphanumeric() => {
                    limpo.push(proximo);
                    chars.next();
                }
                _ => limpo.push(ch),
            }
        } else {
            limpo.push(ch);
        }
    }

    expandir_til(&limpo)
}

fn expandir_til(caminho: &str) -> PathBuf {
    let Some(resto) = caminho.strip_prefix('~') else {
        return PathBuf::from(caminho);
    };
    // `~outrousuario` não é expandido: sem o banco de usuários, chutar seria
    // pior que deixar como está.
    if !(resto.is_empty() || resto.starts_with('/')) {
        return PathBuf::from(caminho);
    }
    let Some(home) = crate::terminal::pasta_pessoal() else {
        return PathBuf::from(caminho);
    };
    let mut expandido = PathBuf::from(home);
    if let Some(relativo) = resto.strip_prefix('/')
        && !relativo.is_empty()
    {
        expandido.push(relativo);
    }
    expandido
}

/// Interpreta a seleção de ISOs de uma pasta: `todas`, `2`, `1,3,5`, `1-4`
/// ou qualquer combinação (`1,4-6,9`). Devolve índices 1-based, sem
/// repetição e em ordem, ou `None` se algo não fizer sentido — inclusive um
/// número fora da lista, que é o erro mais fácil de cometer.
pub fn interpretar_selecao(entrada: &str, total: usize) -> Option<Vec<usize>> {
    let texto = entrada.trim().to_lowercase();
    if texto.is_empty() || total == 0 {
        return None;
    }
    if matches!(texto.as_str(), "todas" | "todos" | "tudo" | "*") {
        return Some((1..=total).collect());
    }

    let mut escolhidos: Vec<usize> = Vec::new();
    for parte in texto.split(',') {
        let parte = parte.trim();
        if parte.is_empty() {
            return None;
        }
        let (inicio, fim) = match parte.split_once('-') {
            Some((a, b)) => (a.trim().parse::<usize>().ok()?, b.trim().parse::<usize>().ok()?),
            None => {
                let n = parte.parse::<usize>().ok()?;
                (n, n)
            }
        };
        if inicio == 0 || fim < inicio || fim > total {
            return None;
        }
        for n in inicio..=fim {
            if !escolhidos.contains(&n) {
                escolhidos.push(n);
            }
        }
    }

    escolhidos.sort_unstable();
    Some(escolhidos)
}

// =============================================================== navegador

/// Como a escolha de imagens vai acontecer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Selecao {
    /// Navegar e escolher uma ou mais ISOs.
    Escolher,
    /// Navegar até uma pasta e levar todas as ISOs dela.
    PastaInteira,
}

/// Navegador de pastas: mostra subpastas e ISOs da pasta atual numa tabela
/// (com tamanho e layout do disco detectado na hora), e deixa entrar,
/// subir, digitar um caminho ou selecionar por número.
///
/// O layout sai de `gdf::tipo_da_imagem`, que só confere a assinatura — dá
/// para ver de relance quais arquivos são mesmo imagens de Xbox antes de
/// escolher, sem abrir imagem nenhuma de verdade.
fn navegador(tema: &Tema, inicial: &Path, modo: Selecao) -> Resultado<Passo<Vec<PathBuf>>> {
    let mut atual = inicial
        .canonicalize()
        .unwrap_or_else(|_| pasta_inicial_segura());
    if !atual.is_dir() {
        atual = pasta_inicial_segura();
    }

    loop {
        println!("{}", tema.caixa_titulo("Escolher imagem", emo::ORIGEM(), LARGURA, true));
        println!("{}", tema.campo("Pasta atual", &encurtar_home(&atual), emo::DESTINO(), 14));

        let (subpastas, truncou) = listar_subpastas(&atual);
        let isos = listar_isos(&atual);

        if subpastas.is_empty() && isos.is_empty() {
            tema.marca_nd("Pasta vazia", "nenhuma subpasta ou .iso aqui");
        } else {
            println!("\n{}", tabela_de_itens(tema, &subpastas, &isos));
            if truncou {
                tema.dica(&format!("Mostrando as primeiras {MAX_SUBPASTAS} subpastas."));
            }
        }

        println!();
        if modo == Selecao::PastaInteira && !isos.is_empty() {
            println!(
                "{}",
                tema.opcao_com("t", "Usar todas as ISOs desta pasta", emo::VELOCIDADE(), Some(&c::verde()), &format!("{} imagem(ns)", isos.len()))
            );
        }
        println!("{}", tema.opcao("..", "Subir uma pasta", emo::SUBIR(), Some(&c::azul())));
        println!("{}", tema.opcao("c", "Digitar um caminho", emo::OPCOES(), Some(&c::azul())));
        if modo == Selecao::Escolher && !isos.is_empty() {
            println!("{}", tema.opcao("t", "Todas as ISOs desta pasta", emo::VELOCIDADE(), Some(&c::azul())));
        }
        println!("{}", tema.opcao("0", "Voltar ao menu", emo::SAIR(), Some(&c::vermelho())));

        if !isos.is_empty() && modo == Selecao::Escolher {
            tema.dica("Pelo número: uma (`2`), várias (`1,3-5`) ou `todas`.");
        }

        let escolha = match ler_linha(tema, "Escolha")? {
            Entrada::Texto(t) => t,
            Entrada::Voltar => return Ok(Passo::Voltar),
            Entrada::Sair => return Ok(Passo::Sair),
        };

        match escolha.to_lowercase().as_str() {
            "" if modo == Selecao::PastaInteira && !isos.is_empty() => return Ok(Passo::Ok(isos)),
            "" => continue,
            "0" => return Ok(Passo::Voltar),
            ".." => {
                atual = atual.parent().map(Path::to_path_buf).unwrap_or(atual);
                continue;
            }
            "t" if !isos.is_empty() => return Ok(Passo::Ok(isos)),
            "c" => {
                match ler_linha(tema, "Caminho da pasta ou da ISO")? {
                    Entrada::Texto(t) if !t.is_empty() => {
                        let alvo = normalizar_caminho(&t);
                        if alvo.is_dir() {
                            atual = alvo.canonicalize().unwrap_or(alvo);
                        } else if alvo.is_file() {
                            return Ok(Passo::Ok(vec![alvo]));
                        } else {
                            tema.erro(&format!("Caminho não encontrado: {}", alvo.display()));
                        }
                    }
                    Entrada::Texto(_) => {}
                    Entrada::Voltar => {}
                    Entrada::Sair => return Ok(Passo::Sair),
                }
                continue;
            }
            _ => {}
        }

        // Seleção por número: a lista mistura pastas e ISOs, então o primeiro
        // índice manda — pasta significa "entra nela", ISO significa "leva
        // estas".
        let total = subpastas.len() + isos.len();
        let Some(indices) = interpretar_selecao(&escolha, total) else {
            tema.erro(&format!("Escolha inválida. Use 1 a {total}, `..`, `c`, `t` ou `0`."));
            continue;
        };

        if indices[0] <= subpastas.len() {
            atual = subpastas[indices[0] - 1].clone();
            continue;
        }

        let escolhidas: Vec<PathBuf> = indices
            .iter()
            .filter(|n| **n > subpastas.len())
            .map(|n| isos[n - subpastas.len() - 1].clone())
            .collect();
        let escolhidas = if modo == Selecao::Escolher {
            escolhidas
        } else {
            isos
        };
        if escolhidas.is_empty() {
            tema.erro("Nenhuma imagem nessa seleção.");
            continue;
        }
        return Ok(Passo::Ok(escolhidas));
    }
}

fn pasta_inicial_segura() -> PathBuf {
    crate::terminal::pasta_pessoal()
        .filter(|p| p.is_dir())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Subpastas visíveis da pasta, em ordem, limitadas a `MAX_SUBPASTAS`.
/// Devolve também se a lista foi cortada, para a tela poder avisar.
fn listar_subpastas(pasta: &Path) -> (Vec<PathBuf>, bool) {
    let mut pastas: Vec<PathBuf> = std::fs::read_dir(pasta)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !nome_de(p).starts_with('.'))
        .collect();
    pastas.sort_by_key(|p| nome_de(p).to_lowercase());
    let truncou = pastas.len() > MAX_SUBPASTAS;
    pastas.truncate(MAX_SUBPASTAS);
    (pastas, truncou)
}

/// Tabela do navegador: pastas primeiro, depois as ISOs com tamanho e
/// layout. A numeração é contínua entre os dois blocos.
fn tabela_de_itens(tema: &Tema, subpastas: &[PathBuf], isos: &[PathBuf]) -> String {
    let mut linhas: Vec<Vec<String>> = Vec::with_capacity(subpastas.len() + isos.len());

    for (i, pasta) in subpastas.iter().enumerate() {
        linhas.push(vec![
            tema.c(&(i + 1).to_string(), &[&c::verde()]),
            format!("{}  {}/", emo::DESTINO(), cortar(&nome_de(pasta), 34)),
            tema.c("—", &[&c::cinza()]),
            tema.c("—", &[&c::cinza()]),
        ]);
    }

    for (i, iso) in isos.iter().enumerate() {
        let tamanho = std::fs::metadata(iso).map(|m| m.len()).unwrap_or(0);
        let layout = match gdf::tipo_da_imagem(iso) {
            Some(tipo) => tema.c(tipo.rotulo(), &[&c::azul()]),
            None => tema.c("?", &[&c::cinza()]),
        };
        linhas.push(vec![
            tema.c(&(subpastas.len() + i + 1).to_string(), &[&c::verde()]),
            format!("{}  {}", emo::DISCO(), cortar(&nome_de(iso), 34)),
            tema.c(&fmt_bytes(tamanho), &[&c::branco()]),
            layout,
        ]);
    }

    tema.tabela(&["#", "ARQUIVO", "TAMANHO", "LAYOUT"], &linhas, &['<', '<', '>', '<'], None)
}

/// Lista arquivos `.iso` (case-insensitive) diretamente dentro de `pasta`,
/// em ordem alfabética.
fn listar_isos(pasta: &Path) -> Vec<PathBuf> {
    let mut isos: Vec<PathBuf> = std::fs::read_dir(pasta)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("iso")))
        .collect();
    isos.sort();
    isos
}

fn nome_de(caminho: &Path) -> String {
    caminho
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| caminho.display().to_string())
}

// ================================================================== ações

/// Ação de converter: escolhe as imagens, monta o plano e executa.
fn acao_converter(tema: &Tema, modo: Selecao) -> Resultado<u32> {
    let descricao = match modo {
        Selecao::Escolher => "Escolha uma ou mais imagens e responda quatro perguntas.",
        Selecao::PastaInteira => "Converte de uma vez todas as ISOs da pasta que você escolher.",
    };
    tela(tema, "Converter ISO para GOD", emo::APP(), descricao);

    loop {
        let caminhos = match navegador(tema, &pasta_inicial_segura(), modo)? {
            Passo::Ok(v) => v,
            Passo::Voltar | Passo::Sair => return Ok(0),
        };

        let itens = analisar_selecionadas(tema, &caminhos);
        if itens.is_empty() {
            tema.erro("Nenhuma imagem utilizável foi selecionada.");
            pausar(tema)?;
            continue;
        }
        mostrar_selecao(tema, &itens);

        match montar_plano(tema, itens)? {
            PlanoPronto::Pronto(plano) => {
                let falhas = executar_plano(tema, &plano);
                pausar(tema)?;
                return Ok(falhas);
            }
            PlanoPronto::TrocarOrigem => continue,
            PlanoPronto::Sair => return Ok(0),
        }
    }
}

/// Ação de analisar: mostra tudo que dá para saber da imagem sem converter.
fn acao_analisar(tema: &Tema) -> Resultado<()> {
    tela(
        tema,
        "Analisar ISO",
        emo::ANALISAR(),
        "Plataforma, Title ID, Media ID, capa e último setor ocupado — sem escrever nada.",
    );

    let caminhos = match navegador(tema, &pasta_inicial_segura(), Selecao::Escolher)? {
        Passo::Ok(v) => v,
        Passo::Voltar | Passo::Sair => return Ok(()),
    };

    for caminho in &caminhos {
        match analise::analisar(caminho) {
            Ok(info) => {
                println!("{}", tema.campo("Arquivo", &nome_de(caminho), emo::DISCO(), 16));
                info.imprimir(tema);
            }
            Err(e) => tema.erro(&format!("{}: {e}", nome_de(caminho))),
        }
    }

    pausar(tema)
}

fn analisar_selecionadas(tema: &Tema, caminhos: &[PathBuf]) -> Vec<Item> {
    let mut itens = Vec::new();
    for caminho in caminhos {
        if caminhos.len() > 1 {
            tema.info_linha(emo::ANALISAR(), &format!("Analisando {}...", nome_de(caminho)));
        }
        match analise::analisar(caminho) {
            Ok(info) => itens.push(Item { caminho: caminho.clone(), info }),
            Err(e) => tema.erro(&format!("{}: {e}", nome_de(caminho))),
        }
    }
    itens
}

/// Uma ISO só: mostra a análise completa (é a tela mais informativa que
/// temos). Várias: uma tabela, senão a tela vira um paredão.
fn mostrar_selecao(tema: &Tema, itens: &[Item]) {
    if let [unico] = itens {
        unico.info.imprimir(tema);
        return;
    }

    println!("{}", tema.etapa(None, &format!("{} imagens prontas", itens.len()), None));
    let linhas: Vec<Vec<String>> = itens
        .iter()
        .map(|item| {
            let tamanho = std::fs::metadata(&item.caminho).map(|m| m.len()).unwrap_or(0);
            vec![
                format!("{}  {}", emo::DISCO(), cortar(&nome_de(&item.caminho), 34)),
                item.info.nome_plataforma().unwrap_or("?").to_string(),
                item.info.title_id.clone().unwrap_or_else(|| "—".to_string()),
                fmt_bytes(tamanho),
            ]
        })
        .collect();
    println!(
        "{}",
        tema.tabela(&["ARQUIVO", "CONSOLE", "TITLE ID", "TAMANHO"], &linhas, &['<', '<', '<', '>'], None)
    );
}

// ============================================================ plano/etapas

/// Uma ISO já escolhida e analisada.
struct Item {
    caminho: PathBuf,
    info: InfoIso,
}

impl Item {
    /// Quantos bytes de dados esta conversão vai processar, de acordo com a
    /// estratégia de padding — a mesma conta de
    /// `god::calcular_bytes_a_converter`, aqui só para estimar espaço antes
    /// de o usuário confirmar.
    fn bytes_a_converter(&self, padding: RemocaoPadding) -> u64 {
        match padding {
            RemocaoPadding::Nenhuma => self.info.tamanho_volume,
            RemocaoPadding::Parcial | RemocaoPadding::Completa => {
                self.info.ultimo_setor as u64 * TAMANHO_SETOR_GDF
            }
        }
    }
}

/// Tudo que o assistente precisa saber para converter — o equivalente
/// interativo da linha de comando que o usuário teria digitado.
struct Plano {
    itens: Vec<Item>,
    destino: PathBuf,
    padding: RemocaoPadding,
    threads: usize,
    titulo: Option<String>,
    icone: Option<PathBuf>,
    numero_disco: bool,
}

impl Plano {
    fn espaco_necessario(&self) -> u64 {
        // As conversões acontecem em sequência, mas os pacotes ficam todos no
        // destino: o que importa é a soma.
        self.itens
            .iter()
            .map(|item| god::tamanho_estimado_do_pacote(item.bytes_a_converter(self.padding)))
            .sum()
    }
}

/// Como a montagem do plano terminou.
enum PlanoPronto {
    Pronto(Plano),
    /// O usuário voltou da primeira etapa: quer escolher outras imagens.
    TrocarOrigem,
    Sair,
}

#[derive(Clone, Copy)]
enum Etapa {
    Destino,
    Padding,
    Threads,
    Avancado,
    Confirmar,
}

/// Conduz as etapas até o usuário confirmar, voltar para a escolha de
/// imagens ou desistir.
fn montar_plano(tema: &Tema, itens: Vec<Item>) -> Resultado<PlanoPronto> {
    let mut destino = PathBuf::new();
    let mut padding = RemocaoPadding::Parcial;
    let mut threads = 1usize;
    let mut titulo: Option<String> = None;
    let mut icone: Option<PathBuf> = None;
    let mut numero_disco = false;

    let mut etapa = Etapa::Destino;
    loop {
        match etapa {
            Etapa::Destino => {
                let padrao = itens
                    .first()
                    .and_then(|i| i.caminho.parent())
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(pasta_inicial_segura);
                match escolher_destino(tema, &padrao)? {
                    Passo::Ok(escolhido) => {
                        destino = escolhido;
                        etapa = Etapa::Padding;
                    }
                    Passo::Voltar => return Ok(PlanoPronto::TrocarOrigem),
                    Passo::Sair => return Ok(PlanoPronto::Sair),
                }
            }
            Etapa::Padding => match escolher_padding(tema)? {
                Passo::Ok(escolhido) => {
                    padding = escolhido;
                    etapa = Etapa::Threads;
                }
                Passo::Voltar => etapa = Etapa::Destino,
                Passo::Sair => return Ok(PlanoPronto::Sair),
            },
            Etapa::Threads => match escolher_threads(tema)? {
                Passo::Ok(escolhido) => {
                    threads = escolhido;
                    etapa = Etapa::Avancado;
                }
                Passo::Voltar => etapa = Etapa::Padding,
                Passo::Sair => return Ok(PlanoPronto::Sair),
            },
            Etapa::Avancado => match escolher_avancado(tema, &itens)? {
                Passo::Ok((t, i, nd)) => {
                    titulo = t;
                    icone = i;
                    numero_disco = nd;
                    etapa = Etapa::Confirmar;
                }
                Passo::Voltar => etapa = Etapa::Threads,
                Passo::Sair => return Ok(PlanoPronto::Sair),
            },
            Etapa::Confirmar => {
                let plano = Plano {
                    itens,
                    destino,
                    padding,
                    threads,
                    titulo,
                    icone,
                    numero_disco,
                };
                match confirmar(tema, &plano)? {
                    Passo::Ok(()) => return Ok(PlanoPronto::Pronto(plano)),
                    Passo::Voltar => {
                        // devolve tudo para as variáveis e volta uma etapa
                        return montar_plano_de(tema, plano, Etapa::Avancado);
                    }
                    Passo::Sair => return Ok(PlanoPronto::Sair),
                }
            }
        }
    }
}

/// Retoma a montagem a partir de uma etapa, reaproveitando o que já foi
/// respondido. Existe para o "voltar" da tela de confirmação não jogar fora
/// as respostas anteriores.
fn montar_plano_de(tema: &Tema, plano: Plano, _etapa: Etapa) -> Resultado<PlanoPronto> {
    let Plano { itens, destino, padding, threads, .. } = plano;
    match escolher_avancado(tema, &itens)? {
        Passo::Ok((titulo, icone, numero_disco)) => {
            let novo = Plano { itens, destino, padding, threads, titulo, icone, numero_disco };
            match confirmar(tema, &novo)? {
                Passo::Ok(()) => Ok(PlanoPronto::Pronto(novo)),
                Passo::Voltar => montar_plano(tema, novo.itens),
                Passo::Sair => Ok(PlanoPronto::Sair),
            }
        }
        Passo::Voltar => montar_plano(tema, itens),
        Passo::Sair => Ok(PlanoPronto::Sair),
    }
}

/// Etapa 1: pasta de destino, realmente verificada (existe, é pasta, aceita
/// escrita) antes de seguir — em vez de o erro aparecer no meio da conversão.
fn escolher_destino(tema: &Tema, padrao: &Path) -> Resultado<Passo<PathBuf>> {
    println!("{}", tema.etapa(Some(1), "Destino", Some(TOTAL_ETAPAS)));
    loop {
        let entrada = ler_linha(
            tema,
            &format!("{}  Pasta de destino [{}]", emo::DESTINO(), encurtar_home(padrao)),
        )?;
        let destino = match entrada {
            Entrada::Texto(t) if t.is_empty() => padrao.to_path_buf(),
            Entrada::Texto(t) => normalizar_caminho(&t),
            Entrada::Voltar => return Ok(Passo::Voltar),
            Entrada::Sair => return Ok(Passo::Sair),
        };
        println!("{}", tema.resposta(&encurtar_home(&destino)));

        if destino.exists() && !destino.is_dir() {
            tema.erro("Esse caminho existe e não é uma pasta.");
            continue;
        }

        if !destino.exists() {
            match perguntar_sim_nao(tema, "Pasta não existe. Criar?", true)? {
                Resposta::Sim => {
                    if let Err(e) = std::fs::create_dir_all(&destino) {
                        tema.erro(&format!("Não foi possível criar a pasta: {e}"));
                        continue;
                    }
                }
                Resposta::Nao => continue,
                Resposta::Sair => return Ok(Passo::Sair),
            }
        }

        if let Err(e) = testar_escrita(&destino) {
            tema.erro(&format!("Sem permissão de escrita nessa pasta: {e}"));
            continue;
        }

        if let Some(livre) = sistema::espaco_livre(&destino) {
            tema.dica(&format!("Espaço livre no destino: {}", fmt_bytes(livre)));
        }

        return Ok(Passo::Ok(destino));
    }
}

/// Confirma que dá para escrever na pasta criando e apagando um arquivo
/// vazio — permissão de diretório sozinha mente em montagem só-leitura, em
/// sistema de arquivos cheio e em pasta de rede.
fn testar_escrita(pasta: &Path) -> io::Result<()> {
    let sonda = pasta.join(format!(".iso2god-escrita-{}", std::process::id()));
    std::fs::File::create(&sonda)?;
    std::fs::remove_file(&sonda)
}

/// Etapa 2: estratégia de remoção de padding (padrão: Parcial, igual à CLI).
fn escolher_padding(tema: &Tema) -> Resultado<Passo<RemocaoPadding>> {
    println!("{}", tema.etapa(Some(2), "Padding", Some(TOTAL_ETAPAS)));
    println!("{}", tema.opcao_com("1", "Nenhuma ", "", None, "converte o volume inteiro, como está"));
    println!(
        "{}",
        tema.opcao_com("2", "Parcial ", "", Some(&c::verde()), "remove o padding do final (recomendado)")
    );
    println!("{}", tema.opcao_com("3", "Completa", "", None, "reconstrói a GDF: menor, porém mais lento"));
    loop {
        let entrada = ler_linha(tema, "Escolha [2]")?;
        let texto = match entrada {
            Entrada::Texto(t) => t,
            Entrada::Voltar => return Ok(Passo::Voltar),
            Entrada::Sair => return Ok(Passo::Sair),
        };
        let padding = match texto.as_str() {
            "" | "2" => RemocaoPadding::Parcial,
            "1" => RemocaoPadding::Nenhuma,
            "3" => RemocaoPadding::Completa,
            _ => {
                tema.erro("Escolha inválida.");
                continue;
            }
        };
        println!("{}", tema.resposta(nome_padding(padding)));
        if matches!(padding, RemocaoPadding::Completa) {
            tema.dica(
                "A reconstrução grava uma ISO temporária no destino antes de converter — \
                 conte com espaço para as duas coisas.",
            );
        }
        return Ok(Passo::Ok(padding));
    }
}

fn nome_padding(padding: RemocaoPadding) -> &'static str {
    match padding {
        RemocaoPadding::Nenhuma => "Nenhuma",
        RemocaoPadding::Parcial => "Parcial",
        RemocaoPadding::Completa => "Completa",
    }
}

/// Etapa 3: número de threads para leitura/hash (padrão: 1, igual à CLI).
fn escolher_threads(tema: &Tema) -> Resultado<Passo<usize>> {
    println!("{}", tema.etapa(Some(3), "Threads", Some(TOTAL_ETAPAS)));
    let nucleos = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    tema.dica(&format!(
        "0 detecta automaticamente ({nucleos} núcleo(s) aqui). Em HD mecânico, mais threads \
         pode piorar."
    ));
    loop {
        let entrada = ler_linha(tema, &format!("{}  Quantas threads [1]", emo::THREADS()))?;
        let texto = match entrada {
            Entrada::Texto(t) => t,
            Entrada::Voltar => return Ok(Passo::Voltar),
            Entrada::Sair => return Ok(Passo::Sair),
        };
        if texto.is_empty() {
            println!("{}", tema.resposta("1"));
            return Ok(Passo::Ok(1));
        }
        match texto.parse::<usize>() {
            Ok(n) => {
                if n > god::MAX_THREADS {
                    tema.aviso(&format!(
                        "o máximo é {}; a conversão vai usar esse valor.",
                        god::MAX_THREADS
                    ));
                }
                println!("{}", tema.resposta(&texto));
                return Ok(Passo::Ok(n));
            }
            Err(_) => tema.erro("Informe um número inteiro."),
        }
    }
}

/// Etapa 4: opções que antes só existiam por flag na CLI e ficavam
/// invisíveis para quem usa o assistente. Pulável — quem só quer converter
/// aperta Enter.
#[allow(clippy::type_complexity)]
fn escolher_avancado(
    tema: &Tema,
    itens: &[Item],
) -> Resultado<Passo<(Option<String>, Option<PathBuf>, bool)>> {
    println!("{}", tema.etapa(Some(TOTAL_ETAPAS), "Opções avançadas", Some(TOTAL_ETAPAS)));
    match perguntar_sim_nao(tema, "Ajustar título, ícone ou numeração de disco?", false)? {
        Resposta::Nao => return Ok(Passo::Ok((None, None, false))),
        Resposta::Sair => return Ok(Passo::Sair),
        Resposta::Sim => {}
    }

    let mut titulo = None;
    let mut icone = None;

    // Título e ícone são de um jogo específico: com várias ISOs na fila, o
    // mesmo valor nas duas seria errado nas duas.
    if let [unico] = itens {
        let detectado = unico.info.titulo.clone().unwrap_or_else(|| {
            unico
                .caminho
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });

        match ler_linha(tema, &format!("{}  Título [{detectado}]", emo::ESTRELA()))? {
            Entrada::Texto(t) if !t.is_empty() => {
                println!("{}", tema.resposta(&t));
                titulo = Some(t);
            }
            Entrada::Texto(_) => println!("{}", tema.resposta(&detectado)),
            Entrada::Voltar => return Ok(Passo::Voltar),
            Entrada::Sair => return Ok(Passo::Sair),
        }

        loop {
            match ler_linha(tema, &format!("{}  Ícone PNG [automático]", emo::JOGO()))? {
                Entrada::Texto(t) if t.is_empty() => {
                    println!("{}", tema.resposta("automático"));
                    break;
                }
                Entrada::Texto(t) => {
                    let caminho = normalizar_caminho(&t);
                    if !caminho.is_file() {
                        tema.erro("Arquivo de ícone não encontrado.");
                        continue;
                    }
                    println!("{}", tema.resposta(&encurtar_home(&caminho)));
                    icone = Some(caminho);
                    break;
                }
                Entrada::Voltar => return Ok(Passo::Voltar),
                Entrada::Sair => return Ok(Passo::Sair),
            }
        }
    } else {
        tema.dica("Título e ícone só se aplicam a uma imagem por vez — pulando esses dois.");
    }

    let numero_disco = match perguntar_sim_nao(tema, "Acrescentar \"- Disc N\" ao título?", false)? {
        Resposta::Sim => true,
        Resposta::Nao => false,
        Resposta::Sair => return Ok(Passo::Sair),
    };

    Ok(Passo::Ok((titulo, icone, numero_disco)))
}

fn confirmar(tema: &Tema, plano: &Plano) -> Resultado<Passo<()>> {
    println!("{}", tema.etapa(None, "Resumo", None));

    if let [unico] = plano.itens.as_slice() {
        println!("{}", tema.campo("Origem", &encurtar_home(&unico.caminho), emo::ORIGEM(), 16));
        if let Some(nome_plataforma) = unico.info.nome_plataforma() {
            println!("{}", tema.campo("Plataforma", nome_plataforma, emo::JOGO(), 16));
        }
        if let Some(t) = &unico.info.titulo {
            println!("{}", tema.campo("Título", t, emo::ESTRELA(), 16));
        }
    } else {
        println!(
            "{}",
            tema.campo("Origem", &format!("{} imagens", plano.itens.len()), emo::ORIGEM(), 16)
        );
    }

    println!("{}", tema.campo("Destino", &encurtar_home(&plano.destino), emo::DESTINO(), 16));
    println!("{}", tema.campo("Padding", nome_padding(plano.padding), emo::OPCOES(), 16));
    println!("{}", tema.campo("Threads", &plano.threads.to_string(), emo::THREADS(), 16));
    if let Some(t) = &plano.titulo {
        println!("{}", tema.campo("Título manual", t, emo::ESTRELA(), 16));
    }
    if let Some(i) = &plano.icone {
        println!("{}", tema.campo("Ícone", &encurtar_home(i), emo::JOGO(), 16));
    }
    if plano.numero_disco {
        println!("{}", tema.campo("Numerar disco", "sim", emo::DISCO(), 16));
    }

    let necessario = plano.espaco_necessario();
    println!("{}", tema.campo("Saída estimada", &fmt_bytes(necessario), emo::DADOS(), 16));

    if let Some(livre) = sistema::espaco_livre(&plano.destino) {
        println!("{}", tema.campo("Espaço livre", &fmt_bytes(livre), emo::DADOS(), 16));
        if livre < necessario {
            tema.aviso(&format!(
                "faltam ~{} no destino — a conversão vai ser recusada antes de começar.",
                fmt_bytes(necessario - livre)
            ));
        }
    }

    tema.dica("`n` ou `v` voltam às opções, `q` cancela tudo.");
    match perguntar_sim_nao(tema, "Iniciar conversão?", true)? {
        Resposta::Sim => Ok(Passo::Ok(())),
        Resposta::Nao => Ok(Passo::Voltar),
        Resposta::Sair => Ok(Passo::Sair),
    }
}

// ================================================================ execução

/// Converte cada item do plano, devolvendo quantos não deram certo. Uma
/// falha isolada não interrompe o lote (a próxima imagem pode estar
/// perfeita); um cancelamento, sim — foi o usuário quem pediu.
fn executar_plano(tema: &Tema, plano: &Plano) -> u32 {
    let total = plano.itens.len();
    let inicio = Instant::now();
    let mut convertidas = 0u32;
    let mut falhas = 0u32;

    for (indice, item) in plano.itens.iter().enumerate() {
        if total > 1 {
            println!(
                "{}",
                tema.etapa(None, &format!("[{}/{total}] {}", indice + 1, nome_de(&item.caminho)), None)
            );
        }

        let opcoes = OpcoesConversao {
            origem: item.caminho.clone(),
            destino: plano.destino.clone(),
            padding: plano.padding,
            numero_disco: plano.numero_disco,
            plataforma: None,
            title_id: None,
            media_id: None,
            titulo: plano.titulo.clone(),
            disco: None,
            total_discos: None,
            plataforma_byte: None,
            tipo_executavel_byte: None,
            icone: plano.icone.clone(),
            threads: plano.threads,
            progresso_json: false,
        };

        match god::converter(&opcoes) {
            Ok(()) => convertidas += 1,
            Err(Erro::Cancelado) => {
                sistema::limpar_cancelamento();
                let restantes = (total - indice) as u32;
                falhas += restantes;
                tema.aviso(&format!(
                    "Cancelado. {restantes} imagem(ns) não convertida(s); a saída incompleta foi descartada."
                ));
                break;
            }
            Err(e) => {
                falhas += 1;
                tema.erro(&format!("Falha ao converter {}: {e}", nome_de(&item.caminho)));
            }
        }
    }

    if total > 1 {
        println!("{}", tema.etapa(None, "Resumo do lote", None));
        println!("{}", tema.campo("Convertidas", &format!("{convertidas} de {total}"), emo::OK(), 16));
        if falhas > 0 {
            println!("{}", tema.campo("Não convertidas", &falhas.to_string(), emo::ERRO(), 16));
        }
        println!(
            "{}",
            tema.campo("Tempo total", &fmt_tempo(inicio.elapsed().as_secs_f64()), emo::TEMPO(), 16)
        );
    }

    falhas
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn normalizar_caminho_expande_til() {
        let home = std::env::var("HOME").expect("HOME definido no ambiente de teste");
        assert_eq!(normalizar_caminho("~/Jogos/x.iso"), PathBuf::from(&home).join("Jogos/x.iso"));
        assert_eq!(normalizar_caminho("~"), PathBuf::from(&home));
        // `~outro` não é nosso para expandir
        assert_eq!(normalizar_caminho("~alguem/x.iso"), PathBuf::from("~alguem/x.iso"));
    }

    #[test]
    fn normalizar_caminho_tira_aspas_de_arrastar_e_soltar() {
        assert_eq!(
            normalizar_caminho("'/mnt/HD Externo/Jogo (USA).iso'"),
            PathBuf::from("/mnt/HD Externo/Jogo (USA).iso")
        );
        assert_eq!(
            normalizar_caminho("\"/mnt/HD Externo/Jogo.iso\""),
            PathBuf::from("/mnt/HD Externo/Jogo.iso")
        );
    }

    #[test]
    fn normalizar_caminho_desfaz_escapes_de_shell() {
        assert_eq!(
            normalizar_caminho("/mnt/HD\\ Externo/Jogo\\ \\(USA\\).iso"),
            PathBuf::from("/mnt/HD Externo/Jogo (USA).iso")
        );
    }

    #[test]
    fn normalizar_caminho_preserva_barra_invertida_que_faz_parte_do_nome() {
        // `\d` não é escape de shell: a barra é parte do nome do arquivo.
        assert_eq!(normalizar_caminho("/tmp/a\\disco.iso"), PathBuf::from("/tmp/a\\disco.iso"));
    }

    #[test]
    fn interpretar_selecao_entende_numero_lista_intervalo_e_todas() {
        assert_eq!(interpretar_selecao("2", 5), Some(vec![2]));
        assert_eq!(interpretar_selecao("1,3,5", 5), Some(vec![1, 3, 5]));
        assert_eq!(interpretar_selecao("2-4", 5), Some(vec![2, 3, 4]));
        assert_eq!(interpretar_selecao("1,3-5", 5), Some(vec![1, 3, 4, 5]));
        assert_eq!(interpretar_selecao("todas", 3), Some(vec![1, 2, 3]));
        assert_eq!(interpretar_selecao(" TODAS ", 2), Some(vec![1, 2]));
    }

    #[test]
    fn interpretar_selecao_remove_repetidos_e_ordena() {
        assert_eq!(interpretar_selecao("3,1,2-3", 3), Some(vec![1, 2, 3]));
    }

    #[test]
    fn interpretar_selecao_recusa_entrada_sem_sentido() {
        assert_eq!(interpretar_selecao("", 3), None);
        assert_eq!(interpretar_selecao("0", 3), None);
        assert_eq!(interpretar_selecao("4", 3), None, "número fora da lista");
        assert_eq!(interpretar_selecao("3-1", 3), None, "intervalo invertido");
        assert_eq!(interpretar_selecao("1,,2", 3), None);
        assert_eq!(interpretar_selecao("abc", 3), None);
        assert_eq!(interpretar_selecao("1-", 3), None);
    }
}
