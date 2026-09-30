//! Camada de apresentação do terminal: cores 24-bit, gradiente azul→verde,
//! caixas com bordas arredondadas, alinhamento consciente de largura visível
//! (emoji contam 2 colunas), barra de progresso animada, e detecção de
//! TTY/`NO_COLOR`. Inspirado na experiência visual do `xiso-manager.py`
//! (referência de UX, não de código) — tudo aqui é só apresentação, nada
//! decide como a conversão funciona.

use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use unicode_width::UnicodeWidthChar;

/// Largura padrão das caixas/réguas, em colunas.
pub const LARGURA: usize = 66;

// ================================================================ console

/// O console interpreta sequências ANSI (cor, apagar linha)? No Unix sempre;
/// no Windows só depois que `preparar_console` consegue ligar o modo VT.
static ANSI_OK: AtomicBool = AtomicBool::new(cfg!(unix));
/// O console desenha emoji? No Windows, só o Windows Terminal (e terminais
/// modernos como o do VS Code); o conhost clássico e o Wine mostram `��`.
static EMOJI_OK: AtomicBool = AtomicBool::new(cfg!(unix));

/// Prepara o console antes da primeira escrita. No Unix não há nada a fazer.
/// No Windows: UTF-8 na entrada e na saída, e liga o processamento de
/// sequências ANSI. Se o console recusar (Windows antigo, Wine), o programa
/// segue sem cor e sem sequências de controle — em vez de imprimir os códigos
/// como texto, que o conhost conta como letras e quebra a linha no meio deles.
pub fn preparar_console() {
    #[cfg(windows)]
    console_windows::preparar();
}

#[cfg(windows)]
mod console_windows {
    use super::{ANSI_OK, EMOJI_OK};
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::System::Console::{
        ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
        STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCP, SetConsoleMode,
        SetConsoleOutputCP,
    };
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};

    const UTF8: u32 = 65001;

    pub fn preparar() {
        unsafe {
            SetConsoleOutputCP(UTF8);
            SetConsoleCP(UTF8);
        }
        let wine = rodando_no_wine();
        // O conhost do Wine aceita o modo VT mas não interpreta as sequências:
        // continua contando cada byte como uma coluna e quebra as linhas.
        let vt = !wine && ligar_vt(STD_OUTPUT_HANDLE);
        if vt {
            ligar_vt(STD_ERROR_HANDLE);
        }
        ANSI_OK.store(vt, Ordering::Relaxed);

        let terminal_moderno =
            std::env::var_os("WT_SESSION").is_some() || std::env::var_os("TERM_PROGRAM").is_some();
        EMOJI_OK.store(vt && terminal_moderno, Ordering::Relaxed);
    }

    /// Liga o processamento de sequências ANSI no console de `qual`. `false`
    /// quando não é um console (saída redirecionada) ou o Windows recusa.
    fn ligar_vt(qual: STD_HANDLE) -> bool {
        unsafe {
            let handle = GetStdHandle(qual);
            let mut modo = 0;
            if handle.is_null() || GetConsoleMode(handle, &mut modo) == 0 {
                return false;
            }
            SetConsoleMode(
                handle,
                modo | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING,
            ) != 0
        }
    }

    /// O Wine exporta `wine_get_version` da ntdll; o Windows não.
    fn rodando_no_wine() -> bool {
        unsafe {
            let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
            !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
        }
    }
}

/// Emoji só onde o terminal desenha; nos outros, um símbolo simples.
pub fn emoji_ativo() -> bool {
    EMOJI_OK.load(Ordering::Relaxed)
}

/// Volta ao início da linha e apaga o que havia nela (a barra de progresso).
/// Sem ANSI, apaga cobrindo com espaços.
pub fn limpar_linha() -> String {
    if ANSI_OK.load(Ordering::Relaxed) {
        "\r\x1b[2K".to_string()
    } else {
        format!("\r{}\r", " ".repeat(colunas_terminal().saturating_sub(1)))
    }
}

/// Pasta pessoal do usuário: `HOME` no Unix, `USERPROFILE` no Windows.
pub fn pasta_pessoal() -> Option<std::path::PathBuf> {
    ["HOME", "USERPROFILE"]
        .iter()
        .filter_map(std::env::var_os)
        .find(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

// ============================================================ cores/estilo

const RESET: &str = "\x1b[0m";
const NEGRITO: &str = "\x1b[1m";
const FRACO: &str = "\x1b[2m";
const ITALICO: &str = "\x1b[3m";

/// Ponta inicial (azul) e final (verde) do gradiente, em RGB.
const GRADIENTE_INICIO: (f64, f64, f64) = (56.0, 150.0, 255.0);
const GRADIENTE_FIM: (f64, f64, f64) = (46.0, 214.0, 130.0);

fn rgb(r: f64, g: f64, b: f64) -> String {
    format!("\x1b[38;2;{};{};{}m", r as u8, g as u8, b as u8)
}

/// Cores nomeadas fixas (equivalente à classe `C` do xiso-manager).
pub mod c {
    pub fn azul() -> String {
        super::rgb(56.0, 150.0, 255.0)
    }
    pub fn verde() -> String {
        super::rgb(46.0, 214.0, 130.0)
    }
    pub fn ciano() -> String {
        super::rgb(92.0, 200.0, 220.0)
    }
    pub fn amarelo() -> String {
        super::rgb(255.0, 186.0, 72.0)
    }
    pub fn vermelho() -> String {
        super::rgb(255.0, 95.0, 110.0)
    }
    pub fn roxo() -> String {
        super::rgb(170.0, 130.0, 255.0)
    }
    pub fn cinza() -> String {
        super::rgb(128.0, 138.0, 150.0)
    }
    pub fn branco() -> String {
        super::rgb(235.0, 240.0, 245.0)
    }
}

pub const NEG: &str = NEGRITO;
pub const FRA: &str = FRACO;
pub const ITA: &str = ITALICO;

/// Cor interpolada azul → verde. `pos` de 0.0 a 1.0.
pub fn gradiente(pos: f64) -> String {
    let pos = pos.clamp(0.0, 1.0);
    rgb(
        GRADIENTE_INICIO.0 + (GRADIENTE_FIM.0 - GRADIENTE_INICIO.0) * pos,
        GRADIENTE_INICIO.1 + (GRADIENTE_FIM.1 - GRADIENTE_INICIO.1) * pos,
        GRADIENTE_INICIO.2 + (GRADIENTE_FIM.2 - GRADIENTE_INICIO.2) * pos,
    )
}

/// Detecta se o terminal suporta cor: respeita `NO_COLOR`, exige TTY, e
/// rejeita `TERM=dumb`/vazio. Calculado uma vez (o terminal não muda de
/// suporte a cor no meio da execução).
pub fn suporta_cor() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if !std::io::stdout().is_terminal() {
        return false;
    }
    // No Windows `TERM` normalmente nem existe: o que decide é o console ter
    // aceitado o modo ANSI (ver `preparar_console`).
    if cfg!(windows) {
        return ANSI_OK.load(Ordering::Relaxed);
    }
    matches!(std::env::var("TERM"), Ok(term) if !term.is_empty() && term != "dumb")
}

/// Aplica cor/estilo ao texto, só quando o terminal suporta (senão devolve
/// o texto puro, sem nenhum código ANSI).
pub fn cor(texto: &str, estilos: &[&str], usar_cor: bool) -> String {
    if !usar_cor || estilos.is_empty() {
        return texto.to_string();
    }
    format!("{}{}{}", estilos.concat(), texto, RESET)
}

// ============================================================ largura/pad

/// Remove sequências ANSI (`\x1b[...m`) de uma string.
fn remover_ansi(texto: &str) -> String {
    let mut saida = String::with_capacity(texto.len());
    let mut chars = texto.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // consome até o 'm' (ou até o fim, se a sequência vier cortada)
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            saida.push(ch);
        }
    }
    saida
}

/// Colunas ocupadas pelo texto, ignorando ANSI. Emoji (e a maioria dos
/// símbolos usados neste programa) contam 2 colunas, igual à maioria dos
/// terminais modernos — replicando a lógica manual do xiso-manager, já que
/// a tabela do `unicode-width` sozinha nem sempre classifica emoji como
/// largos.
pub fn largura_visivel(texto: &str) -> usize {
    let limpo = remover_ansi(texto);
    let chars: Vec<char> = limpo.chars().collect();
    let mut largura = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let prox = chars.get(i + 1).copied();

        if prox == Some('\u{FE0F}') {
            // seletor de variação "emoji" força a largura 2 e some da saída
            largura += 2;
            i += 2;
            continue;
        }
        if ch == '\u{FE0E}' {
            i += 1;
            continue;
        }
        if ch as u32 == 0x200D {
            // zero-width joiner: pula ele e o próximo (sequência combinada)
            i += 2;
            continue;
        }

        let cp = ch as u32;
        let emoji_largo = (0x1F300..=0x1FAFF).contains(&cp) || cp == 0x1F004 || cp == 0x1F0CF;
        if emoji_largo {
            largura += 2;
        } else {
            largura += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        i += 1;
    }
    largura
}

/// Preenche `texto` até `largura` colunas visíveis. `alinhamento`:
/// `'<'` esquerda, `'>'` direita, `'^'` centralizado.
pub fn pad(texto: &str, largura: usize, alinhamento: char) -> String {
    let atual = largura_visivel(texto);
    if atual >= largura {
        return texto.to_string();
    }
    let falta = largura - atual;
    match alinhamento {
        '>' => format!("{}{}", " ".repeat(falta), texto),
        '^' => {
            let esq = falta / 2;
            format!("{}{}{}", " ".repeat(esq), texto, " ".repeat(falta - esq))
        }
        _ => format!("{}{}", texto, " ".repeat(falta)),
    }
}

/// Corta `texto` para caber em `limite` colunas visíveis, com reticências.
pub fn cortar(texto: &str, limite: usize) -> String {
    if largura_visivel(texto) <= limite {
        return texto.to_string();
    }
    let mut saida = String::new();
    for ch in texto.chars() {
        let candidato = format!("{saida}{ch}");
        if largura_visivel(&candidato) > limite.saturating_sub(1) {
            break;
        }
        saida = candidato;
    }
    saida.push('\u{2026}');
    saida
}

fn colunas_terminal() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

// ================================================================== emoji

/// Emoji de cada papel, com uma alternativa simples para consoles que não
/// desenham emoji (ver `emoji_ativo`). As alternativas ocupam 2 colunas,
/// como o emoji, para as listas continuarem alinhadas. São funções, e não constantes, porque
/// a escolha só é conhecida em tempo de execução.
pub mod emo {
    macro_rules! emojis {
        ($($nome:ident = $emoji:literal | $simples:literal;)*) => {
            $(
                #[allow(non_snake_case)]
                pub fn $nome() -> &'static str {
                    if super::emoji_ativo() { $emoji } else { $simples }
                }
            )*
        };
    }

    emojis! {
        APP = "\u{1F3AE}" | ">>"; // 🎮
        OK = "\u{2705}" | "OK"; // ✅
        ERRO = "\u{274C}" | "X "; // ❌
        AVISO = "\u{26A0}\u{FE0F}" | "! "; // ⚠️
        INFO = "\u{2139}\u{FE0F}" | "i "; // ℹ️
        DICA = "\u{1F4A1}" | "* "; // 💡
        ORIGEM = "\u{1F4C2}" | "> "; // 📂
        DESTINO = "\u{1F4C1}" | "> "; // 📁
        OPCOES = "\u{2699}\u{FE0F}" | "* "; // ⚙️
        THREADS = "\u{1F9F5}" | "# "; // 🧵
        PROGRESSO = "\u{1F4CA}" | "% "; // 📊
        TEMPO = "\u{23F1}\u{FE0F}" | "~ "; // ⏱️
        ETA = "\u{1F550}" | "~ "; // 🕐
        DADOS = "\u{1F4E6}" | "# "; // 📦
        VELOCIDADE = "\u{1F680}" | "> "; // 🚀
        ANALISAR = "\u{1F50D}" | "? "; // 🔍
        ESTRELA = "\u{2B50}" | "* "; // ⭐
        JOGO = "\u{1F579}\u{FE0F}" | "> "; // 🕹️
        SAIR = "\u{1F6AA}" | "< "; // 🚪
        RESUMO = "\u{1F4CA}" | "% "; // 📊
        DISCO = "\u{1F4BE}" | "# "; // 💾
        SUBIR = "\u{2B06}\u{FE0F}" | "^ "; // ⬆️
    }
}

pub const SIM_OK: &str = "\u{2713}";
pub const SIM_FALHA: &str = "\u{2717}";
pub const SIM_ND: &str = "\u{B7}";
pub const SIM_SETA: &str = "\u{25B8}";
/// Bolinha de status: verde para pronto, vermelho para faltando. A cor faz o
/// estado saltar aos olhos sem obrigar a ler a palavra ao lado.
pub const SIM_PONTO: &str = "\u{25CF}";

/// Garante que a marca sempre ocupe 2 colunas (senão ✓, que ocupa 1, e ⏱️,
/// que ocupa 2, desalinham a lista inteira).
fn marca_larga(simbolo: &str) -> String {
    if largura_visivel(simbolo) >= 2 {
        simbolo.to_string()
    } else {
        format!("{simbolo} ")
    }
}

// =========================================================== primitivas

/// Contexto de renderização: se cores estão ativas. Passado explicitamente
/// (em vez de variável global) para deixar claro que nada aqui é estado
/// oculto influenciando a conversão.
#[derive(Debug, Clone, Copy)]
pub struct Tema {
    pub cor_ativa: bool,
}

impl Tema {
    pub fn detectar() -> Self {
        Self {
            cor_ativa: suporta_cor(),
        }
    }

    /// Aplica cor/estilo respeitando se este tema tem cor ativa.
    pub fn c(&self, texto: &str, estilos: &[&str]) -> String {
        cor(texto, estilos, self.cor_ativa)
    }

    /// Régua horizontal simples (uma cor só).
    pub fn regua(&self, largura: usize) -> String {
        self.c(&"\u{2500}".repeat(largura), &[&c::cinza()])
    }

    /// Régua pintada com o gradiente inteiro azul → verde.
    pub fn regua_gradiente(&self, largura: usize) -> String {
        if !self.cor_ativa {
            return "\u{2500}".repeat(largura);
        }
        let mut saida = String::new();
        for i in 0..largura {
            let pos = i as f64 / (largura.max(2) - 1) as f64;
            saida.push_str(&gradiente(pos));
            saida.push('\u{2500}');
        }
        saida.push_str(RESET);
        saida
    }

    fn borda_gradiente(&self, largura: usize, esquerdo: char, direito: char) -> String {
        if !self.cor_ativa {
            return format!("{esquerdo}{}{direito}", "\u{2500}".repeat(largura));
        }
        let mut saida = String::new();
        saida.push_str(&gradiente(0.0));
        saida.push(esquerdo);
        for i in 0..largura {
            let pos = i as f64 / (largura.max(2) - 1) as f64;
            saida.push_str(&gradiente(pos));
            saida.push('\u{2500}');
        }
        saida.push_str(&gradiente(1.0));
        saida.push(direito);
        saida.push_str(RESET);
        saida
    }

    /// Caixa de título com bordas arredondadas, estilo:
    /// ```text
    /// ╭──────────────────────╮
    /// │ 🎮  iso2god           │
    /// ╰──────────────────────╯
    /// ```
    pub fn caixa_titulo(
        &self,
        texto: &str,
        emoji: &str,
        largura: usize,
        gradiente_borda: bool,
    ) -> String {
        self.caixa_titulo_com(texto, emoji, largura, gradiente_borda, false)
    }

    /// Como `caixa_titulo`, mas podendo pintar também o **texto** com o
    /// gradiente, letra a letra. É o que diferencia a caixa do menu principal
    /// (onde o título é a marca do programa) das caixas de seção.
    pub fn caixa_titulo_com(
        &self,
        texto: &str,
        emoji: &str,
        largura: usize,
        gradiente_borda: bool,
        gradiente_texto: bool,
    ) -> String {
        let texto_pintado = if gradiente_texto {
            self.texto_gradiente(texto, 0.0)
        } else {
            texto.to_string()
        };
        let miolo = if emoji.is_empty() {
            format!(" {texto_pintado}")
        } else {
            format!(" {emoji}  {texto_pintado}")
        };
        let espaco = (largura.saturating_sub(2)).saturating_sub(largura_visivel(&miolo));

        let topo = if gradiente_borda {
            self.borda_gradiente(largura - 2, '\u{256D}', '\u{256E}')
        } else {
            self.c(
                &format!("\u{256D}{}\u{256E}", "\u{2500}".repeat(largura - 2)),
                &[&c::azul()],
            )
        };
        let base = if gradiente_borda {
            self.borda_gradiente(largura - 2, '\u{2570}', '\u{256F}')
        } else {
            self.c(
                &format!("\u{2570}{}\u{256F}", "\u{2500}".repeat(largura - 2)),
                &[&c::azul()],
            )
        };
        // Texto já pintado letra a letra não pode levar uma cor por cima.
        let corpo = if gradiente_texto {
            miolo.clone()
        } else {
            self.c(&miolo, &[NEG, &c::branco()])
        };
        let borda_lateral = self.c("\u{2502}", &[&c::azul()]);

        format!(
            "\n{topo}\n{borda_lateral}{corpo}{}{borda_lateral}\n{base}\n",
            " ".repeat(espaco)
        )
    }

    /// Linha "rótulo: valor" alinhada, com marca de emoji à esquerda.
    pub fn campo(&self, rotulo: &str, valor: &str, emoji: &str, largura_rotulo: usize) -> String {
        let marca = if emoji.is_empty() {
            "  ".to_string()
        } else {
            marca_larga(emoji)
        };
        let mut rotulo_pad = pad(rotulo, largura_rotulo, '<');
        if largura_visivel(&rotulo_pad) >= largura_rotulo && !rotulo_pad.ends_with(' ') {
            rotulo_pad.push(' ');
        }
        format!(
            "  {} {}{}",
            marca,
            self.c(&rotulo_pad, &[&c::cinza()]),
            self.c(valor, &[&c::branco()])
        )
    }

    pub fn marca_ok(&self, rotulo: &str, valor: &str) {
        println!(
            "  {} {}{}",
            self.c(&marca_larga(SIM_OK), &[&c::verde()]),
            self.c(&format!("{:<17}", rotulo), &[&c::cinza()]),
            self.c(valor, &[&c::branco()])
        );
    }

    pub fn marca_falha(&self, rotulo: &str, valor: &str) {
        println!(
            "  {} {}{}",
            self.c(&marca_larga(SIM_FALHA), &[&c::vermelho()]),
            self.c(&format!("{:<17}", rotulo), &[&c::cinza()]),
            self.c(valor, &[&c::vermelho(), NEG])
        );
    }

    pub fn marca_nd(&self, rotulo: &str, valor: &str) {
        println!(
            "  {} {}{}",
            self.c(&marca_larga(SIM_ND), &[&c::cinza()]),
            self.c(&format!("{:<17}", rotulo), &[&c::cinza()]),
            self.c(valor, &[&c::cinza()])
        );
    }

    /// Linha de opção de menu: `[1]  🎮  Texto      descrição`.
    pub fn opcao(&self, chave: &str, texto: &str, emoji: &str, cor_chave: Option<&str>) -> String {
        self.opcao_com(chave, texto, emoji, cor_chave, "")
    }

    /// Como `opcao`, com uma descrição em cinza depois do rótulo — o lugar
    /// para a informação que ajuda a escolher sem competir com o nome da
    /// opção.
    pub fn opcao_com(
        &self,
        chave: &str,
        texto: &str,
        emoji: &str,
        cor_chave: Option<&str>,
        descricao: &str,
    ) -> String {
        let cor_chave = cor_chave.map(String::from).unwrap_or_else(c::verde);
        let marca = if emoji.is_empty() {
            String::new()
        } else {
            format!("{emoji}  ")
        };
        let rotulo = pad(&format!("[{chave}]"), 4, '>');
        let mut linha = format!(
            "  {}  {}{}",
            self.c(&rotulo, &[&cor_chave, NEG]),
            marca,
            self.c(texto, &[&c::branco()])
        );
        if !descricao.is_empty() {
            linha.push_str(&self.c(&format!("   {descricao}"), &[&c::cinza(), ITA]));
        }
        linha
    }

    /// Linha de status com bolinha: `● 12 núcleos`.
    pub fn ponto_status(&self, ok: bool, texto: &str) -> String {
        let cor_ponto = if ok { c::verde() } else { c::vermelho() };
        format!(
            "  {} {}",
            self.c(SIM_PONTO, &[&cor_ponto]),
            self.c(texto, &[&c::branco()])
        )
    }

    /// Cor do gradiente na posição `pos` (0.0 a 1.0), ou `None` quando o tema
    /// está sem cor — pronta para passar como `cor_chave` de `opcao`.
    pub fn cor_do_gradiente(&self, pos: f64) -> Option<String> {
        self.cor_ativa.then(|| gradiente(pos))
    }

    /// Pinta cada letra com um tom do gradiente, em onda (a `fase` desloca a
    /// onda, para quem quiser animar). Espaços ficam sem cor: pintar espaço
    /// só gasta bytes de ANSI.
    pub fn texto_gradiente(&self, texto: &str, fase: f64) -> String {
        if !self.cor_ativa {
            return texto.to_string();
        }
        let total = texto.chars().count().saturating_sub(1).max(1) as f64;
        let mut saida = String::new();
        for (i, ch) in texto.chars().enumerate() {
            if ch == ' ' {
                saida.push(ch);
                continue;
            }
            let p = ((i as f64 / total) + fase) % 1.0;
            let onda = if p < 0.5 { p * 2.0 } else { (1.0 - p) * 2.0 };
            saida.push_str(&gradiente(onda));
            saida.push(ch);
        }
        saida.push_str(RESET);
        saida
    }

    /// Como `texto_gradiente`, mas dentro de um trecho da escala — usado
    /// pelos títulos de grupo do menu, para a tela inteira ler como um único
    /// gradiente descendo em vez de várias listras iguais.
    pub fn texto_gradiente_faixa(&self, texto: &str, inicio: f64, fim: f64) -> String {
        if !self.cor_ativa {
            return texto.to_string();
        }
        let total = texto.chars().count().saturating_sub(1).max(1) as f64;
        let mut saida = String::new();
        for (i, ch) in texto.chars().enumerate() {
            if ch == ' ' {
                saida.push(ch);
                continue;
            }
            saida.push_str(&gradiente(inicio + (fim - inicio) * (i as f64 / total)));
            saida.push(ch);
        }
        saida.push_str(RESET);
        saida
    }

    /// Trecho de régua pintado com uma faixa do gradiente.
    fn regua_gradiente_parcial(&self, largura: usize, inicio: f64, fim: f64) -> String {
        if largura == 0 {
            return String::new();
        }
        if !self.cor_ativa {
            return "\u{2500}".repeat(largura);
        }
        let divisor = (largura.max(2) - 1) as f64;
        let mut saida = String::new();
        for i in 0..largura {
            saida.push_str(&gradiente(inicio + (fim - inicio) * (i as f64 / divisor)));
            saida.push('\u{2500}');
        }
        saida.push_str(RESET);
        saida
    }

    /// Divisória de grupo do menu, com o nome da seção à esquerda e uma nota
    /// à direita:
    ///
    /// ```text
    ///   ── Converter ────────────────────────── GOD ──
    /// ```
    pub fn cabeca_grupo(&self, titulo: &str, nota: &str, faixa: (f64, f64)) -> String {
        let (ini, fim) = faixa;
        let esquerda = format!(
            "  {} {} ",
            self.c("\u{2500}\u{2500}", &[&gradiente_ou_vazio(self, ini)]),
            self.texto_gradiente_faixa(titulo, ini, fim)
        );
        let direita = if nota.is_empty() {
            self.c("\u{2500}\u{2500}", &[&gradiente_ou_vazio(self, fim)])
        } else {
            format!(
                " {} {}",
                self.c(nota, &[&c::cinza()]),
                self.c("\u{2500}\u{2500}", &[&gradiente_ou_vazio(self, fim)])
            )
        };
        let meio = LARGURA
            .saturating_sub(largura_visivel(&esquerda))
            .saturating_sub(largura_visivel(&direita));
        format!(
            "{esquerda}{}{direita}",
            self.regua_gradiente_parcial(meio, ini, fim)
        )
    }

    /// Tabela alinhada, com cabeçalho em ciano e uma linha opcionalmente
    /// destacada. A largura do separador vem da linha mais larga que
    /// realmente aparece, não da soma das colunas.
    pub fn tabela(
        &self,
        cabecalhos: &[&str],
        linhas: &[Vec<String>],
        alinhamentos: &[char],
        destaque: Option<usize>,
    ) -> String {
        if linhas.is_empty() {
            return String::new();
        }
        let colunas = cabecalhos.len();
        let larguras: Vec<usize> = (0..colunas)
            .map(|i| {
                linhas
                    .iter()
                    .map(|l| l.get(i).map(|c| largura_visivel(c)).unwrap_or(0))
                    .chain(std::iter::once(largura_visivel(cabecalhos[i])))
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let alinhar = |i: usize| *alinhamentos.get(i).unwrap_or(&'<');

        // O `pad` da última coluna deixaria espaços no fim de cada linha —
        // invisíveis na tela, mas sujeira ao copiar a saída do terminal.
        let cabecalho = format!(
            "  {}",
            (0..colunas)
                .map(|i| pad(
                    &self.c(cabecalhos[i], &[NEG, &c::ciano()]),
                    larguras[i],
                    alinhar(i)
                ))
                .collect::<Vec<_>>()
                .join("  ")
        );
        let cabecalho = cabecalho.trim_end().to_string();

        let mut corpo = Vec::new();
        for (indice, linha) in linhas.iter().enumerate() {
            let celulas: Vec<String> = (0..colunas)
                .map(|i| {
                    pad(
                        linha.get(i).map(String::as_str).unwrap_or(""),
                        larguras[i],
                        alinhar(i),
                    )
                })
                .collect();
            let linha = if destaque == Some(indice) {
                format!(
                    "{} {}",
                    self.c(SIM_SETA, &[&c::verde()]),
                    celulas.join("  ")
                )
            } else {
                format!("  {}", celulas.join("  "))
            };
            corpo.push(linha.trim_end().to_string());
        }

        let util = std::iter::once(&cabecalho)
            .chain(corpo.iter())
            .map(|l| largura_visivel(l.trim_end()))
            .max()
            .unwrap_or(0);
        let separador = format!(
            "  {}",
            self.c(&"\u{2500}".repeat(util.max(6) - 2), &[&c::cinza()])
        );

        let mut saida = vec![cabecalho, separador];
        saida.extend(corpo);
        saida.join("\n")
    }

    /// Divisória de etapa: `── 1/4 · Origem ──────────────`.
    pub fn etapa(&self, numero: Option<u32>, titulo: &str, total: Option<u32>) -> String {
        let prefixo = match (numero, total) {
            (Some(n), Some(t)) => format!("{n}/{t} \u{B7} {titulo}"),
            _ => titulo.to_string(),
        };
        let cabeca = format!(
            "  {} {} ",
            self.c("\u{2500}\u{2500}", &[&c::cinza()]),
            self.c(&prefixo, &[&c::ciano(), NEG])
        );
        let resto = LARGURA.saturating_sub(largura_visivel(&cabeca));
        format!(
            "\n{cabeca}{}",
            self.c(&"\u{2500}".repeat(resto), &[&c::cinza()])
        )
    }

    /// Eco do que foi aceito, recuado sob a pergunta que o gerou.
    pub fn resposta(&self, texto: &str) -> String {
        format!(
            "    {} {}",
            self.c("\u{2514}", &[&c::cinza()]),
            self.c(texto, &[&c::ciano()])
        )
    }

    pub fn dica(&self, texto: &str) {
        println!(
            "{}  {}",
            self.c(emo::DICA(), &[]),
            self.c(texto, &[&c::ciano(), ITA])
        );
    }

    pub fn sucesso(&self, texto: &str) {
        println!(
            "{}  {}",
            self.c(emo::OK(), &[]),
            self.c(texto, &[&c::verde(), NEG])
        );
    }

    /// Erros e avisos vão para **stderr**, não stdout: `iso2god info x.iso >
    /// arquivo.txt` precisa continuar mostrando a falha no terminal, e quem
    /// consome a saída da ferramenta não deve receber diagnóstico misturado
    /// com dado. (No modo `--json` o erro sai como evento no stdout, que é
    /// o protocolo — ver `crate::progresso`.)
    pub fn erro(&self, texto: &str) {
        eprintln!(
            "{} {}",
            self.c(emo::ERRO(), &[]),
            self.c(texto, &[&c::vermelho(), NEG])
        );
    }

    pub fn aviso(&self, texto: &str) {
        eprintln!(
            "{}  {}",
            self.c(emo::AVISO(), &[]),
            self.c(texto, &[&c::amarelo()])
        );
    }

    pub fn info_linha(&self, emoji: &str, texto: &str) {
        println!("{emoji}  {}", self.c(texto, &[&c::branco()]));
    }

    pub fn prompt(&self, texto: &str) -> String {
        format!("\n  {} {}: ", self.c(SIM_SETA, &[&c::ciano()]), texto)
    }
}

/// Cor do gradiente na posição `pos`, ou string vazia quando o tema está sem
/// cor (assim `Tema::c` não emite ANSI nenhum).
fn gradiente_ou_vazio(tema: &Tema, pos: f64) -> String {
    if tema.cor_ativa {
        gradiente(pos)
    } else {
        String::new()
    }
}

/// `~/Jogos` lê melhor que `/home/usuario/Jogos` e economiza colunas.
pub fn encurtar_home(caminho: &std::path::Path) -> String {
    let texto = caminho.display().to_string();
    let Some(casa) = pasta_pessoal() else {
        return texto;
    };
    let casa = casa.to_string_lossy().to_string();
    if casa.is_empty() {
        return texto;
    }
    match texto.strip_prefix(&casa) {
        Some(resto) => format!("~{resto}"),
        None => texto,
    }
}

// =============================================================== formato

pub fn fmt_bytes(b: u64) -> String {
    if b == 0 {
        return "0 B".to_string();
    }
    let gb = b as f64 / (1024f64.powi(3));
    if gb >= 1.0 {
        return format!("{gb:.2} GiB");
    }
    let mb = b as f64 / (1024f64.powi(2));
    if mb >= 1.0 {
        return format!("{mb:.2} MiB");
    }
    format!("{:.2} KiB", b as f64 / 1024.0)
}

pub fn fmt_velocidade(bps: f64) -> String {
    if bps <= 0.0 {
        return "N/D".to_string();
    }
    format!("{}/s", fmt_bytes(bps as u64))
}

pub fn fmt_tempo(segundos: f64) -> String {
    let s = segundos.max(0.0) as u64;
    let (h, resto) = (s / 3600, s % 3600);
    let (m, s) = (resto / 60, resto % 60);
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

// ======================================================= barra de progresso

/// Barra de progresso com preenchimento em gradiente e onda animada,
/// mesma anatomia visual do xiso-manager. Sem TTY, não desenha nada
/// quadro a quadro — só imprime uma linha final limpa, uma vez.
pub struct BarraProgresso {
    tema: Tema,
    rotulo: String,
    emoji: String,
    total_bytes: u64,
    inicio: Instant,
    quadro: AtomicU64,
    interativo: bool,
}

const LARGURA_BARRA: usize = 24;

impl BarraProgresso {
    pub fn nova(tema: Tema, rotulo: &str, emoji: &str, total_bytes: u64) -> Self {
        Self {
            tema,
            rotulo: rotulo.to_string(),
            emoji: emoji.to_string(),
            total_bytes,
            inicio: Instant::now(),
            quadro: AtomicU64::new(0),
            interativo: std::io::stdout().is_terminal(),
        }
    }

    fn pintar(&self, pct: f64, largura: usize) -> String {
        let cheio = (largura as f64 * pct) as usize;
        let quadro = self.quadro.load(Ordering::Relaxed) as f64;
        let fase = (quadro * 0.05) % 1.0;
        let mut saida = String::new();
        for i in 0..largura {
            if i < cheio {
                let bruto = (i as f64 / (largura.max(2) - 1) as f64) + fase;
                let ciclo = bruto % 1.0;
                let onda = if ciclo < 0.5 {
                    ciclo * 2.0
                } else {
                    (1.0 - ciclo) * 2.0
                };
                if self.tema.cor_ativa {
                    saida.push_str(&gradiente(onda));
                }
                saida.push('\u{2588}');
            } else {
                if self.tema.cor_ativa {
                    saida.push_str(&c::cinza());
                }
                saida.push('\u{2591}');
            }
        }
        if self.tema.cor_ativa {
            saida.push_str(RESET);
        }
        saida
    }

    /// Atualiza a barra com `feito` bytes processados (de `total_bytes`).
    /// `velocidade_bps`/`eta_segundos` vêm de fora (já calculados a partir
    /// de dados reais do processo de conversão — a barra só desenha).
    pub fn atualizar(
        &self,
        feito: u64,
        velocidade_bps: f64,
        eta_segundos: Option<f64>,
        detalhe: &str,
    ) {
        if !self.interativo {
            return; // sem TTY: só a linha final importa, ver `finalizar`
        }

        let pct = if self.total_bytes > 0 {
            (feito as f64 / self.total_bytes as f64).min(1.0)
        } else {
            0.0
        };
        let colunas = colunas_terminal();
        let rotulo_completo = if self.emoji.is_empty() {
            self.rotulo.clone()
        } else {
            format!("{} {}", self.emoji, self.rotulo)
        };

        let mut largura_barra = LARGURA_BARRA;
        while largura_barra > 8
            && largura_visivel(&format!(
                "  {rotulo_completo} [{}] 100.0%",
                "\u{2500}".repeat(largura_barra)
            )) > colunas.saturating_sub(1)
        {
            largura_barra -= 2;
        }

        let essencial_largura = largura_visivel(&format!(
            "  {rotulo_completo} [{}] 100.0%",
            "\u{2500}".repeat(largura_barra)
        ));
        let mut sobra = colunas.saturating_sub(essencial_largura).saturating_sub(1);

        let bytes_txt = format!("  {}/{}", fmt_bytes(feito), fmt_bytes(self.total_bytes));
        let vel_txt = format!("  {}", fmt_velocidade(velocidade_bps));
        let eta_txt = format!(
            "  ETA {}",
            eta_segundos
                .map(fmt_tempo)
                .unwrap_or_else(|| "N/D".to_string())
        );
        let det_txt = if detalhe.is_empty() {
            String::new()
        } else {
            format!("  {}", cortar(detalhe, 18))
        };

        let mut extras = Vec::new();
        for (texto, estilo) in [
            (&bytes_txt, None),
            (&vel_txt, Some(c::amarelo())),
            (&eta_txt, Some(c::cinza())),
            (&det_txt, Some(c::cinza())),
        ] {
            if texto.is_empty() {
                continue;
            }
            let larg = largura_visivel(texto);
            if larg <= sobra {
                sobra -= larg;
                extras.push((texto.clone(), estilo));
            } else {
                break;
            }
        }

        let mut linha = format!(
            "  {} {}{}{} {}",
            rotulo_completo,
            self.tema.c("[", &[&c::cinza()]),
            self.pintar(pct, largura_barra),
            self.tema.c("]", &[&c::cinza()]),
            self.tema
                .c(&format!("{:>5.1}%", pct * 100.0), &[&c::branco(), NEG])
        );
        for (texto, estilo) in extras {
            match estilo {
                Some(cor_txt) => linha.push_str(&self.tema.c(&texto, &[&cor_txt])),
                None => linha.push_str(&texto),
            }
        }

        print!("{}{linha}", limpar_linha());
        use std::io::Write;
        let _ = std::io::stdout().flush();
        self.quadro.fetch_add(1, Ordering::Relaxed);
    }

    /// Linha final: limpa a barra interativa e imprime um resumo de uma
    /// linha só (também é tudo que aparece quando não há TTY).
    pub fn finalizar(&self, feito: u64, sucesso: bool) {
        if self.interativo {
            print!("{}", limpar_linha());
        }
        let decorrido = self.inicio.elapsed().as_secs_f64().max(0.001);
        let vel = feito as f64 / decorrido;
        let marca = if sucesso {
            self.tema.c(emo::OK(), &[])
        } else {
            self.tema.c(emo::ERRO(), &[])
        };
        println!(
            "  {} {}: {} em {} ({})",
            marca,
            self.rotulo,
            fmt_bytes(feito),
            fmt_tempo(decorrido),
            fmt_velocidade(vel)
        );
    }
}
