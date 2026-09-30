use crate::erro::{Erro, Resultado};

/// Tamanho total, em bytes, do cabeçalho LIVE escrito no início do container GOD.
pub const TAMANHO_CABECALHO_LIVE: usize = 45_056;

/// Modelo binário usado como ponto de partida do cabeçalho: o recurso
/// embutido `Resources.emptyLIVE` do projeto original (extraído do `.resx` e
/// incluído aqui byte a byte). Ele já vem com vários campos da estrutura
/// STFS/LIVE preenchidos (o "LIVE" mágico, uma string de descrição fixa,
/// tabelas de licença, etc.) que nenhum método de `ConHeaderWriter` escreve
/// — nunca foram documentados nem reconstruídos aqui, só preservados
/// fielmente a partir do original, exatamente como o C# fazia ao carregar
/// esse mesmo recurso em vez de montar o cabeçalho do zero.
const MODELO_LIVE: &[u8; TAMANHO_CABECALHO_LIVE] = include_bytes!("live_template.bin");

// Deslocamentos dos campos escritos ativamente por `ConHeaderWriter`. Os
// nomes e valores vêm diretamente dos `bw.Seek(...)` do original.
const OFF_HASH_ASSINATURA: usize = 812;
const OFF_TIPO_CONTEUDO: usize = 836;
const OFF_MEDIA_ID: usize = 852;
const OFF_TITLE_ID: usize = 864;
const OFF_PLATAFORMA: usize = 868;
const OFF_TIPO_EXECUTAVEL: usize = 869;
const OFF_DISCO: usize = 870;
const OFF_TOTAL_DISCOS: usize = 871;
const OFF_MHT_HASH: usize = 893;
const OFF_BLOCOS_ALOCADOS: usize = 914; // 3 bytes (WriteUint24)
const OFF_BLOCOS_NAO_ALOCADOS: usize = 917; // 2 bytes (WriteUint16)
const OFF_NUM_PARTES: usize = 928; // 4 bytes, little-endian
const OFF_TAMANHO_PARTES: usize = 932; // 4 bytes, big-endian, valor/256
const OFF_TITULO_1: usize = 1041; // UTF-16BE
const OFF_TITULO_2: usize = 5777; // segunda cópia do título
const OFF_ICONE_TAMANHO_1: usize = 5906; // 4 bytes, big-endian
const OFF_ICONE_TAMANHO_2: usize = 5910; // 4 bytes, big-endian (mesmo valor)
const OFF_ICONE_DADOS_1: usize = 5914;
const OFF_ICONE_DADOS_2: usize = 22298; // segunda cópia do ícone

// Três bytes que o template traz preenchidos (herdados de uma versão inicial
// do cabeçalho, `LiveHeader`, ou de outra origem não documentada) e que o
// `ConHeaderWriter.Write()` original zera explicitamente na etapa final,
// antes de calcular o hash de assinatura e gravar o arquivo.
const OFFSETS_ZERADOS_NA_GRAVACAO_FINAL: [usize; 3] = [859, 863, 913];

/// Início (offset) e tamanho da região sobre a qual o hash de assinatura é
/// calculado: do byte 836 até o fim do buffer (836 + 44220 = 45056).
const INICIO_REGIAO_HASH: usize = 836;

/// Tamanho de PNG usado quando nenhum thumbnail é fornecido — o original
/// escreve um array de 20 bytes zerados nesse caso, em vez de nada.
const TAMANHO_ICONE_PADRAO: usize = 20;

/// Tamanho, em bytes, dos campos de Title ID e Media ID. Os dois têm 4
/// bytes fixos: um hexadecimal mais curto preenchia só parte do campo e
/// deixava o resto com o que estivesse no modelo, produzindo um pacote com
/// identidade silenciosamente errada.
pub const TAMANHO_ID: usize = 4;

/// Confere que um identificador hexadecimal ocupa exatamente o campo.
pub fn validar_id(rotulo: &str, hex: &str) -> Resultado<()> {
    let bytes = hex_para_bytes(hex)?;
    if bytes.len() != TAMANHO_ID {
        return Err(Erro::IsoInvalida(format!(
            "{rotulo} precisa ter exatamente {} dígitos hexadecimais (4 bytes); recebi {hex:?}",
            TAMANHO_ID * 2
        )));
    }
    Ok(())
}

/// Capacidade, em caracteres UTF-16, de cada campo de título do cabeçalho.
/// Não é um número escolhido por nós: o campo em `OFF_TITULO_2` tem
/// exatamente 128 bytes (5777..5905, com os campos de tamanho do thumbnail
/// começando logo em 5906), e cada caractere ocupa 2 bytes em UTF-16 — daí
/// 64. O original não verifica isso e deixa um título longo transbordar por
/// cima dos campos seguintes; como o título padrão é o *nome do arquivo* da
/// ISO, nomes de release completos passam desse limite com facilidade.
pub const MAX_TITULO_UTF16: usize = 64;

/// Capacidade, em bytes, de cada região de thumbnail do cabeçalho: a
/// primeira começa em `OFF_ICONE_DADOS_1` e vai exatamente até onde a
/// segunda começa (22298 - 5914 = 16384). Um PNG maior que isso
/// transbordaria silenciosamente para o campo seguinte.
pub const MAX_ICONE: usize = OFF_ICONE_DADOS_2 - OFF_ICONE_DADOS_1;

/// Corta `titulo` para caber em `MAX_TITULO_UTF16` unidades UTF-16, sempre
/// numa fronteira de caractere (cortar no meio de um par substituto
/// produziria UTF-16 inválido). Devolve também se houve corte, para quem
/// chama poder avisar o usuário.
pub fn ajustar_titulo(titulo: &str) -> (String, bool) {
    if titulo.encode_utf16().count() <= MAX_TITULO_UTF16 {
        return (titulo.to_string(), false);
    }
    let mut cortado = String::new();
    let mut unidades = 0usize;
    for ch in titulo.chars() {
        let custo = ch.len_utf16();
        if unidades + custo > MAX_TITULO_UTF16 {
            break;
        }
        unidades += custo;
        cortado.push(ch);
    }
    (cortado, true)
}

/// Tipo de conteúdo armazenado no container, equivalente a
/// `Chilano.Iso2God.ConStructures.ContentType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TipoConteudo {
    XboxOriginal,
    GamesOnDemand,
}

impl TipoConteudo {
    pub fn valor(self) -> u32 {
        match self {
            TipoConteudo::XboxOriginal => 0x5000,
            TipoConteudo::GamesOnDemand => 0x7000,
        }
    }
}

/// Constrói o cabeçalho LIVE (formato CON/STFS) escrito antes dos dados do
/// container GOD. Equivalente a `Chilano.Iso2God.ConHeader.ConHeaderWriter`.
#[derive(Debug, Clone)]
pub struct EscritorCabecalho {
    buffer: Box<[u8; TAMANHO_CABECALHO_LIVE]>,
}

impl Default for EscritorCabecalho {
    fn default() -> Self {
        Self::novo()
    }
}

impl EscritorCabecalho {
    pub fn novo() -> Self {
        Self {
            buffer: Box::new(*MODELO_LIVE),
        }
    }

    /// Copia `dados` para dentro do buffer a partir de `offset`, verificando
    /// os limites antes de escrever (o original, ao gravar além da
    /// capacidade de um `MemoryStream` de tamanho fixo, lançaria uma
    /// exceção; preferimos um erro tratável a um pânico de índice).
    fn escrever_em(&mut self, offset: usize, dados: &[u8]) -> Resultado<()> {
        let fim = offset
            .checked_add(dados.len())
            .filter(|&fim| fim <= self.buffer.len())
            .ok_or_else(|| {
                Erro::IsoInvalida(format!(
                    "dado de {} bytes não cabe no cabeçalho LIVE a partir do deslocamento {offset}",
                    dados.len()
                ))
            })?;
        self.buffer[offset..fim].copy_from_slice(dados);
        Ok(())
    }

    /// Grava Title ID, Media ID (ambos strings hexadecimais) e o nome do
    /// jogo, codificado como UTF-16 *big-endian* (o original grava em UTF-16
    /// little-endian e depois troca os bytes de cada par manualmente — aqui
    /// codificamos direto como big-endian, resultado idêntico). O nome é
    /// escrito duas vezes, em dois offsets diferentes do cabeçalho.
    /// Equivalente a `ConHeaderWriter.WriteIDs` — com uma proteção que o
    /// original não tem: o título é cortado em `MAX_TITULO_UTF16`
    /// caracteres, senão transbordaria por cima dos campos seguintes do
    /// cabeçalho (ver a constante).
    pub fn escrever_ids(&mut self, title_id: &str, media_id: &str, titulo: &str) -> Resultado<()> {
        validar_id("o Title ID", title_id)?;
        validar_id("o Media ID", media_id)?;
        self.escrever_em(OFF_TITLE_ID, &hex_para_bytes(title_id)?)?;
        self.escrever_em(OFF_MEDIA_ID, &hex_para_bytes(media_id)?)?;

        let (titulo, _) = ajustar_titulo(titulo);
        let titulo_utf16be: Vec<u8> = titulo
            .encode_utf16()
            .flat_map(|unidade| unidade.to_be_bytes())
            .collect();
        self.escrever_em(OFF_TITULO_1, &titulo_utf16be)?;
        self.escrever_em(OFF_TITULO_2, &titulo_utf16be)?;
        Ok(())
    }

    /// Grava plataforma, tipo de executável, número e contagem de discos.
    /// Equivalente a `ConHeaderWriter.WriteExecutionDetails`.
    pub fn escrever_detalhes_execucao(
        &mut self,
        disco: u8,
        total_discos: u8,
        plataforma: u8,
        tipo_exec: u8,
    ) -> Resultado<()> {
        self.escrever_em(OFF_PLATAFORMA, &[plataforma])?;
        self.escrever_em(OFF_TIPO_EXECUTAVEL, &[tipo_exec])?;
        self.escrever_em(OFF_DISCO, &[disco])?;
        self.escrever_em(OFF_TOTAL_DISCOS, &[total_discos])?;
        Ok(())
    }

    /// Grava a contagem de blocos alocados (24 bits) e não alocados (16
    /// bits), ambos big-endian. Equivalente a
    /// `ConHeaderWriter.WriteBlockCounts`.
    pub fn escrever_contagem_blocos(&mut self, alocados: u32, nao_alocados: u16) -> Resultado<()> {
        if alocados > 0x00FF_FFFF {
            return Err(Erro::IsoInvalida(format!(
                "contagem de blocos alocados ({alocados}) não cabe em 24 bits"
            )));
        }
        let bytes = alocados.to_be_bytes(); // [_, b2, b1, b0] em big-endian de 32 bits
        self.escrever_em(OFF_BLOCOS_ALOCADOS, &bytes[1..4])?;
        self.escrever_em(OFF_BLOCOS_NAO_ALOCADOS, &nao_alocados.to_be_bytes())?;
        Ok(())
    }

    /// Grava número de partes de dados (little-endian) e o tamanho total das
    /// partes dividido por 256 (big-endian). Equivalente a
    /// `ConHeaderWriter.WriteDataPartsInfo`.
    pub fn escrever_info_partes(&mut self, num_partes: u32, tamanho_total: u64) -> Resultado<()> {
        let escala = tamanho_total / 256;
        let escala: u32 = escala.try_into().map_err(|_| {
            Erro::IsoInvalida(format!(
                "tamanho total das partes ({tamanho_total} bytes) grande demais para o cabeçalho LIVE"
            ))
        })?;
        self.escrever_em(OFF_NUM_PARTES, &num_partes.to_le_bytes())?;
        self.escrever_em(OFF_TAMANHO_PARTES, &escala.to_be_bytes())?;
        Ok(())
    }

    /// Grava o ícone/thumbnail do jogo (PNG) em duas posições do cabeçalho,
    /// junto com seu tamanho (também duplicado). Quando `png` é `None`,
    /// grava um marcador de 20 bytes zerados em vez de dados reais — mesmo
    /// comportamento do original para jogos sem thumbnail.
    /// Equivalente a `ConHeaderWriter.WriteGameIcon`.
    pub fn escrever_icone(&mut self, png: Option<&[u8]>) -> Resultado<()> {
        let vazio;
        let dados = match png {
            Some(dados) => dados,
            None => {
                vazio = [0u8; TAMANHO_ICONE_PADRAO];
                &vazio
            }
        };

        if dados.len() > MAX_ICONE {
            return Err(Erro::IsoInvalida(format!(
                "ícone de {} bytes não cabe no cabeçalho LIVE: o limite do campo de thumbnail \
                 é de {MAX_ICONE} bytes ({} KiB)",
                dados.len(),
                MAX_ICONE / 1024
            )));
        }

        let tamanho: u32 = dados.len().try_into().map_err(|_| {
            Erro::IsoInvalida(format!(
                "ícone de {} bytes grande demais para o cabeçalho LIVE",
                dados.len()
            ))
        })?;

        self.escrever_em(OFF_ICONE_TAMANHO_1, &tamanho.to_be_bytes())?;
        self.escrever_em(OFF_ICONE_TAMANHO_2, &tamanho.to_be_bytes())?;
        self.escrever_em(OFF_ICONE_DADOS_1, dados)?;
        self.escrever_em(OFF_ICONE_DADOS_2, dados)?;
        Ok(())
    }

    /// Grava o tipo de conteúdo (Xbox Original vs Games on Demand).
    /// Equivalente a `ConHeaderWriter.WriteContentType`.
    pub fn escrever_tipo_conteudo(&mut self, tipo: TipoConteudo) -> Resultado<()> {
        self.escrever_em(OFF_TIPO_CONTEUDO, &tipo.valor().to_be_bytes())
    }

    /// Grava o hash da cadeia de Master Hash Tables.
    /// Equivalente a `ConHeaderWriter.WriteMhtHash`.
    pub fn escrever_hash_mht(&mut self, hash: &[u8; 20]) -> Resultado<()> {
        self.escrever_em(OFF_MHT_HASH, hash)
    }

    /// Calcula e grava o SHA1 de assinatura sobre a região do cabeçalho que
    /// vai do byte 836 até o final (45056), a mesma região coberta pelo
    /// original (`sha1.ComputeHash(buffer, 836, 44220)`).
    /// Equivalente a `ConHeaderWriter.WriteHash`.
    pub fn escrever_hash_assinatura(&mut self) -> Resultado<()> {
        let hash = super::hashtable::sha1(&self.buffer[INICIO_REGIAO_HASH..]);
        self.escrever_em(OFF_HASH_ASSINATURA, &hash)
    }

    /// Zera os três campos residuais do template, recalcula o hash de
    /// assinatura e grava o cabeçalho completo no caminho informado.
    /// Equivalente a `ConHeaderWriter.Write`.
    pub fn gravar(&mut self, caminho: &std::path::Path) -> Resultado<()> {
        for offset in OFFSETS_ZERADOS_NA_GRAVACAO_FINAL {
            self.buffer[offset] = 0;
        }
        self.escrever_hash_assinatura()?;
        std::fs::write(caminho, self.buffer.as_slice())?;
        Ok(())
    }
}

/// Decodifica uma string hexadecimal em bytes, equivalente a
/// `ConHeaderWriter.hexStringToBytes` — mas retornando erro em vez de
/// lançar exceção para uma entrada malformada.
fn hex_para_bytes(hex: &str) -> Resultado<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return Err(Erro::IsoInvalida(format!(
            "string hexadecimal com tamanho ímpar: {hex:?}"
        )));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| Erro::IsoInvalida(format!("string hexadecimal inválida: {hex:?}")))
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn modelo_tem_o_tamanho_certo_e_a_assinatura_live() {
        assert_eq!(MODELO_LIVE.len(), TAMANHO_CABECALHO_LIVE);
        assert_eq!(&MODELO_LIVE[0..4], b"LIVE");
    }

    #[test]
    fn escrever_ids_grava_nos_offsets_certos_e_duplica_o_titulo() {
        let mut c = EscritorCabecalho::novo();
        c.escrever_ids("4D5308BF", "AABBCCDD", "Título").unwrap();

        assert_eq!(
            &c.buffer[OFF_TITLE_ID..OFF_TITLE_ID + 4],
            &[0x4D, 0x53, 0x08, 0xBF]
        );
        assert_eq!(
            &c.buffer[OFF_MEDIA_ID..OFF_MEDIA_ID + 4],
            &[0xAA, 0xBB, 0xCC, 0xDD]
        );

        let titulo_esperado: Vec<u8> = "Título"
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(
            &c.buffer[OFF_TITULO_1..OFF_TITULO_1 + titulo_esperado.len()],
            titulo_esperado.as_slice()
        );
        assert_eq!(
            &c.buffer[OFF_TITULO_2..OFF_TITULO_2 + titulo_esperado.len()],
            titulo_esperado.as_slice()
        );
    }

    /// Regressão: um título mais longo que o campo (o padrão é o nome do
    /// arquivo da ISO, então nomes de release passam disso fácil) não pode
    /// escrever um único byte além de `OFF_TITULO_2 + 128`, onde já começam
    /// os campos de tamanho do thumbnail.
    #[test]
    fn titulo_longo_nao_invade_os_campos_seguintes() {
        let modelo = EscritorCabecalho::novo();
        let mut c = EscritorCabecalho::novo();

        let titulo_gigante =
            "Tom Clancy's Splinter Cell Double Agent (USA) (En,Fr,Es) (Disc 1 of 2) Platinum Hits";
        assert!(
            titulo_gigante.chars().count() > MAX_TITULO_UTF16,
            "o título de teste precisa estourar o campo"
        );
        c.escrever_ids("4D5308BF", "AABBCCDD", titulo_gigante)
            .unwrap();

        let fim_do_campo = OFF_TITULO_2 + MAX_TITULO_UTF16 * 2;
        assert_eq!(
            &c.buffer[fim_do_campo..OFF_ICONE_DADOS_1 + 64],
            &modelo.buffer[fim_do_campo..OFF_ICONE_DADOS_1 + 64],
            "o título transbordou por cima dos campos seguintes do cabeçalho"
        );

        // e o que coube foi realmente gravado, cortado no limite
        let esperado: Vec<u8> = ajustar_titulo(titulo_gigante)
            .0
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        assert_eq!(esperado.len(), MAX_TITULO_UTF16 * 2);
        assert_eq!(&c.buffer[OFF_TITULO_2..fim_do_campo], esperado.as_slice());
    }

    #[test]
    fn ajustar_titulo_nao_corta_no_meio_de_um_par_substituto() {
        // Cada emoji fora do BMP custa 2 unidades UTF-16: 33 deles dariam 66
        // unidades, então o corte tem que cair entre emojis, nunca no meio.
        let titulo: String = std::iter::repeat_n('\u{1F3AE}', 33).collect();
        let (cortado, truncou) = ajustar_titulo(&titulo);
        assert!(truncou);
        assert_eq!(cortado.chars().count(), 32);
        assert_eq!(cortado.encode_utf16().count(), MAX_TITULO_UTF16);
        // se tivesse cortado no meio do par, isto viraria U+FFFD
        assert!(cortado.chars().all(|ch| ch == '\u{1F3AE}'));
    }

    #[test]
    fn ajustar_titulo_deixa_passar_o_que_ja_cabe() {
        let (saida, truncou) = ajustar_titulo("Halo 3");
        assert_eq!(saida, "Halo 3");
        assert!(!truncou);
    }

    #[test]
    fn escrever_icone_rejeita_png_maior_que_o_campo() {
        let mut c = EscritorCabecalho::novo();
        assert!(c.escrever_icone(Some(&vec![0xABu8; MAX_ICONE])).is_ok());
        assert!(
            c.escrever_icone(Some(&vec![0xABu8; MAX_ICONE + 1]))
                .is_err(),
            "um PNG maior que o campo transbordaria para o thumbnail seguinte"
        );
    }

    /// Regressão: um Title ID mais curto que o campo preenchia só parte dele
    /// e deixava o resto com os bytes do modelo — o pacote saía com
    /// identidade errada e ninguém avisava. Um ID vazio era pior ainda: o
    /// nível de diretório do Title ID sumia do caminho de saída.
    #[test]
    fn ids_precisam_ocupar_o_campo_inteiro() {
        for curto in ["", "AB", "ABCD", "ABCDEF", "545400A3BB"] {
            assert!(
                validar_id("o Title ID", curto).is_err(),
                "{curto:?} não ocupa os 4 bytes do campo e deveria ser recusado"
            );
        }
        assert!(validar_id("o Title ID", "545400A3").is_ok());
        assert!(validar_id("o Title ID", "00000000").is_ok());

        let mut c = EscritorCabecalho::novo();
        assert!(c.escrever_ids("ABCD", "AABBCCDD", "Jogo").is_err());
        assert!(c.escrever_ids("AABBCCDD", "ABCD", "Jogo").is_err());
    }

    #[test]
    fn escrever_ids_rejeita_hex_invalido() {
        let mut c = EscritorCabecalho::novo();
        assert!(c.escrever_ids("XYZ", "AABBCCDD", "t").is_err());
        assert!(c.escrever_ids("ABC", "AABBCCDD", "t").is_err()); // tamanho ímpar
    }

    #[test]
    fn escrever_contagem_blocos_usa_24_bits_big_endian() {
        let mut c = EscritorCabecalho::novo();
        c.escrever_contagem_blocos(0x01_0203, 0x0405).unwrap();
        assert_eq!(
            &c.buffer[OFF_BLOCOS_ALOCADOS..OFF_BLOCOS_ALOCADOS + 3],
            &[0x01, 0x02, 0x03]
        );
        assert_eq!(
            &c.buffer[OFF_BLOCOS_NAO_ALOCADOS..OFF_BLOCOS_NAO_ALOCADOS + 2],
            &[0x04, 0x05]
        );
    }

    #[test]
    fn escrever_contagem_blocos_rejeita_acima_de_24_bits() {
        let mut c = EscritorCabecalho::novo();
        assert!(c.escrever_contagem_blocos(0x0100_0000, 0).is_err());
    }

    #[test]
    fn escrever_info_partes_usa_endians_diferentes_por_campo() {
        let mut c = EscritorCabecalho::novo();
        c.escrever_info_partes(7, 256 * 1000).unwrap();
        assert_eq!(
            &c.buffer[OFF_NUM_PARTES..OFF_NUM_PARTES + 4],
            &7u32.to_le_bytes()
        );
        assert_eq!(
            &c.buffer[OFF_TAMANHO_PARTES..OFF_TAMANHO_PARTES + 4],
            &1000u32.to_be_bytes()
        );
    }

    #[test]
    fn escrever_icone_duplica_tamanho_e_dados_e_usa_marcador_sem_png() {
        let mut c = EscritorCabecalho::novo();
        c.escrever_icone(None).unwrap();
        assert_eq!(
            &c.buffer[OFF_ICONE_TAMANHO_1..OFF_ICONE_TAMANHO_1 + 4],
            &20u32.to_be_bytes()
        );
        assert_eq!(
            &c.buffer[OFF_ICONE_TAMANHO_2..OFF_ICONE_TAMANHO_2 + 4],
            &20u32.to_be_bytes()
        );

        let png = vec![0xAB; 100];
        c.escrever_icone(Some(&png)).unwrap();
        assert_eq!(
            &c.buffer[OFF_ICONE_TAMANHO_1..OFF_ICONE_TAMANHO_1 + 4],
            &100u32.to_be_bytes()
        );
        assert_eq!(
            &c.buffer[OFF_ICONE_DADOS_1..OFF_ICONE_DADOS_1 + 100],
            png.as_slice()
        );
        assert_eq!(
            &c.buffer[OFF_ICONE_DADOS_2..OFF_ICONE_DADOS_2 + 100],
            png.as_slice()
        );
    }

    #[test]
    fn escrever_tipo_conteudo_grava_valor_correto() {
        let mut c = EscritorCabecalho::novo();
        c.escrever_tipo_conteudo(TipoConteudo::GamesOnDemand)
            .unwrap();
        assert_eq!(
            &c.buffer[OFF_TIPO_CONTEUDO..OFF_TIPO_CONTEUDO + 4],
            &0x7000u32.to_be_bytes()
        );
    }

    #[test]
    fn gravar_zera_campos_residuais_e_recalcula_hash() {
        let mut c = EscritorCabecalho::novo();
        // Confirma que o template realmente trazia esses bytes não-zerados
        // antes de gravarmos, para validar que a limpeza faz diferença.
        assert_ne!(c.buffer[859], 0);
        assert_ne!(c.buffer[863], 0);
        assert_ne!(c.buffer[913], 0);

        let arquivo_temp = std::env::temp_dir().join("iso2god_teste_cabecalho.bin");
        c.gravar(&arquivo_temp).unwrap();

        assert_eq!(c.buffer[859], 0);
        assert_eq!(c.buffer[863], 0);
        assert_eq!(c.buffer[913], 0);

        let hash_esperado = super::super::hashtable::sha1(&c.buffer[INICIO_REGIAO_HASH..]);
        assert_eq!(
            &c.buffer[OFF_HASH_ASSINATURA..OFF_HASH_ASSINATURA + 20],
            &hash_esperado
        );

        let gravado = std::fs::read(&arquivo_temp).unwrap();
        assert_eq!(gravado.len(), TAMANHO_CABECALHO_LIVE);
        assert_eq!(gravado, c.buffer.as_slice());

        std::fs::remove_file(&arquivo_temp).ok();
    }
}
