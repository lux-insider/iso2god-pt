use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Ferramenta CLI para converter imagens ISO de Xbox/Xbox 360 em containers GOD.
#[derive(Debug, Parser)]
#[command(name = "iso2god", version, about, long_about = None)]
pub struct Cli {
    /// Sem nenhum subcomando, abre o assistente interativo.
    #[command(subcommand)]
    pub comando: Option<Comando>,
}

#[derive(Debug, Subcommand)]
pub enum Comando {
    /// Converte uma imagem ISO em um container GOD (Games on Demand).
    Converter(ArgsConverter),

    /// Exibe detalhes de uma imagem ISO (Title ID, plataforma, etc).
    Info(ArgsInfo),
}

#[derive(Debug, Parser)]
pub struct ArgsConverter {
    /// Caminho da imagem ISO de origem.
    pub origem: PathBuf,

    /// Diretório de destino onde o container GOD será criado.
    pub destino: PathBuf,

    /// Estratégia de remoção de padding do ISO.
    #[arg(long, value_enum, default_value_t = RemocaoPadding::Parcial)]
    pub padding: RemocaoPadding,

    /// Adiciona o número do disco ao nome do título quando houver múltiplos discos.
    #[arg(long, default_value_t = false)]
    pub numero_disco: bool,

    /// Quantas threads usar para ler e calcular os hashes SHA1 dos blocos de
    /// dados durante a conversão. Use 0 para detectar automaticamente a
    /// quantidade de núcleos disponíveis. O padrão é 1 (sequencial, igual ao
    /// programa original) — aumente esse número em SSDs/NVMes para acelerar
    /// a conversão; em HDs mecânicos, mais threads pode só piorar o
    /// desempenho por causa da movimentação da cabeça de leitura.
    #[arg(short = 'j', long, default_value_t = 1)]
    pub threads: usize,

    /// Relata o progresso como uma linha JSON por evento em stdout, em vez
    /// da barra de terminal colorida. Pensado para consumo por outro
    /// programa (ex: um script) — não afeta a conversão em si.
    #[arg(long, default_value_t = false)]
    pub progresso_json: bool,

    /// Plataforma do jogo (define o tipo de conteúdo do container: Xbox
    /// Original ou Games on Demand). Se omitido, é detectada automaticamente
    /// verificando se a ISO contém "default.xex" (Xbox 360) ou "default.xbe"
    /// (Xbox original).
    #[arg(long, value_enum)]
    pub plataforma: Option<Plataforma>,

    /// Title ID do jogo, em hexadecimal (ex: 4D5308BF). Se omitido, é lido
    /// automaticamente do default.xex (Xbox 360) ou default.xbe (Xbox
    /// original).
    #[arg(long)]
    pub title_id: Option<String>,

    /// Media ID do jogo, em hexadecimal. Se omitido, é lido automaticamente
    /// do default.xex; para Xbox original (sem Media ID nativo), é derivado
    /// do MD5 do default.xbe. Mesma prioridade do `--title-id`.
    #[arg(long)]
    pub media_id: Option<String>,

    /// Nome de exibição do jogo, gravado no cabeçalho LIVE. Se omitido, usa
    /// o nome embutido no certificado do default.xbe (Xbox original) quando
    /// disponível, senão o nome do arquivo da ISO (sem extensão) — o nome
    /// de exibição de jogos de Xbox 360 fica em um recurso XDBF separado
    /// que ainda não foi portado.
    #[arg(long)]
    pub titulo: Option<String>,

    /// Número deste disco (para jogos multi-disco). Se omitido, é lido do
    /// default.xex/default.xbe quando possível; senão assume 1.
    #[arg(long)]
    pub disco: Option<u8>,

    /// Total de discos do jogo. Mesma observação do `--disco` (para Xbox
    /// original, sempre assume 1 — o formato não registra esse total).
    #[arg(long)]
    pub total_discos: Option<u8>,

    /// Byte de plataforma bruto do cabeçalho de execução (campo informativo
    /// do STFS). Se omitido, é lido do default.xex quando possível; senão
    /// assume 0 (sempre 0 para Xbox original).
    #[arg(long)]
    pub plataforma_byte: Option<u8>,

    /// Byte de tipo de executável bruto do cabeçalho de execução (mesma
    /// observação do `--plataforma-byte`).
    #[arg(long)]
    pub tipo_executavel_byte: Option<u8>,

    /// Caminho para uma imagem PNG a ser usada como ícone/thumbnail do jogo.
    /// Se omitido, é extraído automaticamente do default.xbe (Xbox
    /// original, seção "$$XSIMAGE"/"$$XTIMAGE" em formato XPR/DXT1 ou ARGB).
    /// Para Xbox 360 (default.xex), o thumbnail fica num recurso XDBF
    /// separado que ainda não foi portado — informe manualmente se quiser um.
    #[arg(long)]
    pub icone: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Plataforma {
    Xbox,
    Xbox360,
}

#[derive(Debug, Parser)]
pub struct ArgsInfo {
    /// Caminho da imagem ISO a ser inspecionada.
    pub origem: PathBuf,

    /// Imprime as informações como um único objeto JSON em stdout, em vez
    /// do texto formatado de sempre. Pensado para consumo por outro
    /// programa (ex: um script).
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RemocaoPadding {
    /// Não remove nenhum padding do ISO original.
    Nenhuma,
    /// Remove apenas o padding no final da imagem.
    Parcial,
    /// Reconstrói a estrutura GDF por completo, remapeando setores.
    Completa,
}
