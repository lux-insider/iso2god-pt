pub mod xpr;

use md5::{Digest, Md5};

use crate::erro::{Erro, Resultado};

/// Assinatura no início de todo arquivo XBE válido: "XBEH" em ASCII,
/// lido como um `u32` little-endian.
const ASSINATURA_XBEH: u32 = 1_212_498_520;

/// Deslocamento, dentro do cabeçalho do XBE, do campo `BaseAddress` (u32).
const OFF_BASE_ADDRESS: usize = 260;

/// Deslocamento do campo `CertificateAddress` (u32) — um endereço virtual
/// que precisa ser convertido em deslocamento de arquivo subtraindo
/// `BaseAddress`, já que o XBE guarda endereços como se o arquivo estivesse
/// mapeado em memória a partir de `BaseAddress`.
const OFF_CERTIFICATE_ADDRESS: usize = 280;

/// Tamanho mínimo do certificado que realmente lemos: Size(4), TimeData(4),
/// TitleID(4), TitleName(80), AltTitleIDs(64), AllowedMedia(4),
/// GameRegion(4), GameRatings(4) e DiskNumber(4), somando 172 bytes.
/// O certificado de verdade continua depois disso (Version, chaves de
/// assinatura, etc.), mas não precisamos desses campos.
const TAMANHO_CERTIFICADO_LIDO: usize = 172;

/// Deslocamentos dos campos do cabeçalho usados para localizar a tabela de
/// seções: `NumberOfSections` (u32) e `SectionHeadersAddress` (u32),
/// ambos logo depois de `CertificateAddress`.
const OFF_NUMBER_OF_SECTIONS: usize = 284;
const OFF_SECTION_HEADERS_ADDRESS: usize = 288;

/// Tamanho de um registro `XbeSectionHeader`: Flags(4), VirtualAddress(4),
/// VirtualSize(4), RawAddress(4), RawSize(4), SectionNameAddress(4),
/// SectionNameRefCount(4), HeadSharedPageRefCountAddress(4),
/// TailSharedPageRefCountAddress(4) e SectionDigest(20), somando 56 bytes.
const TAMANHO_CABECALHO_SECAO: usize = 56;

/// Nomes das seções do XBE que podem conter o thumbnail do jogo, na mesma
/// ordem de prioridade do original (`IsoDetails.readXbe`: tenta
/// "$$XSIMAGE" primeiro, cai para "$$XTIMAGE" se não achar).
const NOMES_SECAO_THUMBNAIL: [&str; 2] = ["$$XSIMAGE", "$$XTIMAGE"];

/// Informações extraídas do certificado de um XBE (Xbox original) — o
/// suficiente para popular o cabeçalho LIVE. Equivalente, restrito aos
/// campos usados, a `Chilano.Xbox360.Xbe.XbeCertifcate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoCertificado {
    pub title_id: u32,
    pub titulo: String,
    pub disco_numero: u32,
}

impl InfoCertificado {
    /// Formata o Title ID como 8 dígitos hexadecimais maiúsculos.
    ///
    /// Nota: o original formata isso com `titleID.ToString("X02")`, que para
    /// um `uint` (diferente de um `byte`) só garante um mínimo de 2 dígitos —
    /// um Title ID com bytes altos zerados sairia mais curto que 8
    /// caracteres, gerando um hex de tamanho variável que quebraria a
    /// decodificação de 4 bytes esperada em `escrever_ids`. Aqui sempre
    /// formatamos com 8 dígitos fixos (`{:08X}`), que é claramente a intenção
    /// original (e o que o caminho do XEX já faz corretamente por operar
    /// byte a byte) — uma correção deliberada, não uma reprodução do bug.
    pub fn title_id_hex(&self) -> String {
        format!("{:08X}", self.title_id)
    }
}

/// Extrai Title ID, nome de exibição e número do disco do certificado de um
/// arquivo XBE já lido inteiramente em memória (tipicamente o
/// `default.xbe` de dentro da ISO). Equivalente a `XbeHeader` +
/// `XbeCertifcate`, restrito aos campos que usamos.
pub fn ler_info_certificado(xbe: &[u8]) -> Resultado<InfoCertificado> {
    if xbe.len() < OFF_CERTIFICATE_ADDRESS + 4 {
        return Err(Erro::IsoInvalida(
            "arquivo pequeno demais para conter um cabeçalho XBE válido".into(),
        ));
    }

    let magic = u32::from_le_bytes(xbe[0..4].try_into().unwrap());
    if magic != ASSINATURA_XBEH {
        return Err(Erro::IsoInvalida(
            "arquivo não começa com a assinatura XBE esperada".into(),
        ));
    }

    let base_address = ler_u32_le(xbe, OFF_BASE_ADDRESS);
    let certificate_address = ler_u32_le(xbe, OFF_CERTIFICATE_ADDRESS);

    let offset_certificado = certificate_address.checked_sub(base_address).ok_or_else(|| {
        Erro::IsoInvalida(
            "endereço do certificado do XBE é menor que o endereço base (arquivo corrompido)"
                .into(),
        )
    })? as usize;

    let fim = offset_certificado
        .checked_add(TAMANHO_CERTIFICADO_LIDO)
        .filter(|&fim| fim <= xbe.len())
        .ok_or_else(|| {
            Erro::IsoInvalida("o certificado do XBE aponta além do fim do arquivo".into())
        })?;

    let cert = &xbe[offset_certificado..fim];
    let title_id = u32::from_le_bytes(cert[8..12].try_into().unwrap());
    let titulo = decodificar_utf16le_ate_nul(&cert[12..92]);
    let disco_numero = u32::from_le_bytes(cert[168..172].try_into().unwrap());

    Ok(InfoCertificado { title_id, titulo, disco_numero })
}

/// Deriva um substituto de Media ID a partir do MD5 do arquivo XBE inteiro —
/// mesma solução do original (`IsoDetailsResults`, comentário "we need a
/// unique id, 8 chars from md5 is good enough"), já que discos de Xbox
/// original não têm um Media ID nativo como os de Xbox 360.
pub fn media_id_substituto(xbe: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(xbe);
    let hash = hasher.finalize();
    hash[0..4].iter().map(|b| format!("{b:02X}")).collect()
}

/// Extrai o thumbnail do jogo (uma das seções "$$XSIMAGE"/"$$XTIMAGE" do
/// XBE, contendo uma textura XPR) e devolve como PNG já codificado.
/// Equivalente a `IsoDetails.thumbXpr`, chamado para "$$XSIMAGE" e depois
/// "$$XTIMAGE" em `IsoDetails.readXbe`.
pub fn extrair_thumbnail(xbe: &[u8]) -> Resultado<Vec<u8>> {
    let secao = encontrar_secao_thumbnail(xbe)?;
    let textura = xpr::decodificar(secao)?;
    xpr::rgba_para_png(textura.largura, textura.altura, &textura.rgba)
}

fn encontrar_secao_thumbnail(xbe: &[u8]) -> Resultado<&[u8]> {
    if xbe.len() < OFF_SECTION_HEADERS_ADDRESS + 4 {
        return Err(Erro::IsoInvalida(
            "arquivo pequeno demais para conter a tabela de seções do XBE".into(),
        ));
    }
    let magic = u32::from_le_bytes(xbe[0..4].try_into().unwrap());
    if magic != ASSINATURA_XBEH {
        return Err(Erro::IsoInvalida("arquivo não começa com a assinatura XBE esperada".into()));
    }

    let base_address = ler_u32_le(xbe, OFF_BASE_ADDRESS);
    let num_secoes = ler_u32_le(xbe, OFF_NUMBER_OF_SECTIONS);
    let secoes_address = ler_u32_le(xbe, OFF_SECTION_HEADERS_ADDRESS);
    let offset_secoes = secoes_address.checked_sub(base_address).ok_or_else(|| {
        Erro::IsoInvalida("endereço da tabela de seções do XBE é inválido".into())
    })? as usize;

    for nome_procurado in NOMES_SECAO_THUMBNAIL {
        for indice in 0..num_secoes {
            let offset_cabecalho = offset_secoes + indice as usize * TAMANHO_CABECALHO_SECAO;
            let Some(fim_cabecalho) = offset_cabecalho.checked_add(TAMANHO_CABECALHO_SECAO) else {
                break;
            };
            if fim_cabecalho > xbe.len() {
                break;
            }

            let raw_address = ler_u32_le(xbe, offset_cabecalho + 12);
            let raw_size = ler_u32_le(xbe, offset_cabecalho + 16);
            let nome_address = ler_u32_le(xbe, offset_cabecalho + 20);

            let Some(offset_nome) = nome_address.checked_sub(base_address) else { continue };
            let Some(nome) = ler_string_ascii_ate_nul(xbe, offset_nome as usize) else { continue };

            if nome != nome_procurado {
                continue;
            }

            let inicio = raw_address as usize;
            let Some(fim) = inicio.checked_add(raw_size as usize).filter(|&f| f <= xbe.len()) else {
                continue;
            };
            return Ok(&xbe[inicio..fim]);
        }
    }

    Err(Erro::IsoInvalida(
        "nenhuma seção de thumbnail (\"$$XSIMAGE\"/\"$$XTIMAGE\") encontrada no XBE".into(),
    ))
}

fn ler_string_ascii_ate_nul(bytes: &[u8], offset: usize) -> Option<String> {
    let fatia = bytes.get(offset..)?;
    let fim = fatia.iter().position(|&b| b == 0)?;
    Some(fatia[..fim].iter().map(|&b| b as char).collect())
}

fn ler_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// Decodifica um buffer UTF-16LE de tamanho fixo, parando no primeiro
/// caractere nulo (o original não faz esse corte e deixa os zeros de
/// preenchimento como caracteres literais U+0000 no meio da string — uma
/// limpeza deliberada nossa, não uma reprodução do comportamento).
fn decodificar_utf16le_ate_nul(bytes: &[u8]) -> String {
    let unidades: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&unidades)
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Monta um XBE sintético mínimo: assinatura + campos fixos do cabeçalho
    /// até `CertificateAddress`, com o certificado colocado logo após o
    /// cabeçalho (BaseAddress escolhido para que a tradução
    /// endereço-para-deslocamento dê um valor simples de conferir).
    fn montar_xbe_sintetico(title_id: u32, titulo: &str, disco_numero: u32) -> Vec<u8> {
        const BASE_ADDRESS: u32 = 0x0001_0000;
        const TAMANHO_CABECALHO: u32 = 284 + 4; // até o fim de CertificateAddress

        let mut xbe = vec![0u8; TAMANHO_CABECALHO as usize];
        xbe[0..4].copy_from_slice(&ASSINATURA_XBEH.to_le_bytes());
        xbe[OFF_BASE_ADDRESS..OFF_BASE_ADDRESS + 4].copy_from_slice(&BASE_ADDRESS.to_le_bytes());

        let certificate_address = BASE_ADDRESS + TAMANHO_CABECALHO;
        xbe[OFF_CERTIFICATE_ADDRESS..OFF_CERTIFICATE_ADDRESS + 4]
            .copy_from_slice(&certificate_address.to_le_bytes());

        let mut cert = vec![0u8; TAMANHO_CERTIFICADO_LIDO];
        cert[0..4].copy_from_slice(&0u32.to_le_bytes()); // Size (não usado)
        cert[8..12].copy_from_slice(&title_id.to_le_bytes());
        let nome_utf16: Vec<u8> = titulo
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        cert[12..12 + nome_utf16.len()].copy_from_slice(&nome_utf16);
        // o resto de cert[12..92] já fica zerado (padding/terminador nulo)
        cert[168..172].copy_from_slice(&disco_numero.to_le_bytes());

        xbe.extend_from_slice(&cert);
        xbe
    }

    #[test]
    fn extrai_certificado_de_um_xbe_sintetico() {
        let xbe = montar_xbe_sintetico(0x1234_5678, "Meu Jogo", 2);
        let info = ler_info_certificado(&xbe).expect("deveria extrair o certificado");

        assert_eq!(info.title_id, 0x1234_5678);
        assert_eq!(info.title_id_hex(), "12345678");
        assert_eq!(info.titulo, "Meu Jogo");
        assert_eq!(info.disco_numero, 2);
    }

    #[test]
    fn title_id_hex_sempre_tem_oito_digitos_mesmo_com_bytes_altos_zerados() {
        let xbe = montar_xbe_sintetico(0x0000_00AB, "X", 1);
        let info = ler_info_certificado(&xbe).unwrap();
        assert_eq!(info.title_id_hex(), "000000AB");
    }

    #[test]
    fn titulo_para_no_primeiro_caractere_nulo() {
        let xbe = montar_xbe_sintetico(1, "Curto", 1);
        let info = ler_info_certificado(&xbe).unwrap();
        assert_eq!(info.titulo, "Curto");
        assert!(!info.titulo.contains('\0'));
    }

    #[test]
    fn rejeita_arquivo_sem_assinatura_xbeh() {
        let mut xbe = vec![0u8; 512];
        xbe[0..4].copy_from_slice(b"NADA");
        assert!(ler_info_certificado(&xbe).is_err());
    }

    #[test]
    fn rejeita_arquivo_truncado() {
        let xbe = vec![0u8; 10];
        assert!(ler_info_certificado(&xbe).is_err());
    }

    #[test]
    fn media_id_substituto_e_deterministico_e_tem_oito_hex_chars() {
        let dados = b"conteudo de teste do xbe";
        let a = media_id_substituto(dados);
        let b = media_id_substituto(dados);
        assert_eq!(a, b);
        assert_eq!(a.len(), 8);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
    }

    #[test]
    fn media_id_substituto_bate_com_md5_calculado_independentemente() {
        // md5("abc") = 900150983cd24fb0d6963f7d28e17f72 (vetor de teste RFC 1321)
        let info = media_id_substituto(b"abc");
        assert_eq!(info, "90015098");
    }
}
