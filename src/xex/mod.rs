use crate::erro::{Erro, Resultado};

/// Assinatura no início de todo arquivo XEX2 válido.
const ASSINATURA_XEX2: &[u8; 4] = b"XEX2";

/// ID do campo "Execution Info" na tabela de cabeçalhos opcionais do XEX2 —
/// valor documentado do formato (`XexExecutionInfo.Signature` no original,
/// como big-endian: `{0,4,0,6}` -> `0x00040006`).
const ID_EXECUTION_INFO: u32 = 0x0004_0006;

/// Tamanho, em bytes, do bloco "Execution Info" a partir do endereço
/// apontado pela tabela de cabeçalhos opcionais: Media ID (4), Version (4),
/// Base Version (4), Title ID (4), Platform (1), Executable Type (1),
/// Disc Number (1) e Disc Count (1).
const TAMANHO_EXECUTION_INFO: usize = 20;

/// Informações extraídas do bloco "Execution Info" do cabeçalho de um XEX —
/// o suficiente para popular o cabeçalho LIVE do container GOD sem precisar
/// que o usuário informe Title ID/Media ID/plataforma manualmente.
/// Equivalente a `Chilano.Xbox360.Xex.XexExecutionInfo`, restrito aos campos
/// que usamos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoExecucao {
    pub title_id: [u8; 4],
    pub media_id: [u8; 4],
    pub plataforma: u8,
    pub tipo_executavel: u8,
    pub disco_numero: u8,
    pub disco_total: u8,
}

impl InfoExecucao {
    pub fn title_id_hex(&self) -> String {
        bytes_para_hex(&self.title_id)
    }

    pub fn media_id_hex(&self) -> String {
        bytes_para_hex(&self.media_id)
    }
}

/// Extrai as informações de execução do cabeçalho de um arquivo XEX2 já lido
/// inteiramente em memória (tipicamente o `default.xex` de dentro da ISO).
///
/// Equivalente a `XexHeader` (a varredura da tabela de cabeçalhos opcionais)
/// combinado com `XexExecutionInfo.Parse` — mas, diferente do original, que
/// monta um dicionário com *todos* os campos reconhecidos do cabeçalho
/// (ModuleFlags, ResourceInfo, CompressionInfo, etc.), aqui procuramos
/// apenas pelo único campo que nos interessa (`ExecutionInfo`), já que os
/// outros não são usados para gerar o cabeçalho LIVE.
///
/// Tolera um cabeçalho truncado (pára de procurar e retorna erro de "não
/// encontrado" em vez de estourar limites) — mesmo espírito do
/// `catch (EndOfStreamException)` do original.
pub fn ler_info_execucao(xex: &[u8]) -> Resultado<InfoExecucao> {
    if xex.len() < 24 || &xex[0..4] != ASSINATURA_XEX2 {
        return Err(Erro::IsoInvalida(
            "arquivo não começa com a assinatura XEX2 esperada".into(),
        ));
    }

    let num_entradas = u32::from_be_bytes(xex[20..24].try_into().unwrap());

    let mut posicao = 24usize;
    for _ in 0..num_entradas {
        if posicao + 8 > xex.len() {
            break;
        }
        let chave = u32::from_be_bytes(xex[posicao..posicao + 4].try_into().unwrap());
        let valor = u32::from_be_bytes(xex[posicao + 4..posicao + 8].try_into().unwrap());
        posicao += 8;

        if chave == ID_EXECUTION_INFO {
            return ler_execution_info(xex, valor as usize);
        }
    }

    Err(Erro::IsoInvalida(
        "o XEX não contém um bloco ExecutionInfo (Title ID/Media ID indisponíveis)".into(),
    ))
}

fn ler_execution_info(xex: &[u8], endereco: usize) -> Resultado<InfoExecucao> {
    let fim = endereco.checked_add(TAMANHO_EXECUTION_INFO).ok_or_else(|| {
        Erro::IsoInvalida("endereço do bloco ExecutionInfo do XEX é inválido".into())
    })?;
    if fim > xex.len() {
        return Err(Erro::IsoInvalida(
            "o bloco ExecutionInfo do XEX aponta além do fim do arquivo".into(),
        ));
    }

    let bloco = &xex[endereco..fim];
    // bloco[4..8] = Version, bloco[8..12] = BaseVersion — lidos no original,
    // mas não usados para gerar o cabeçalho LIVE.
    Ok(InfoExecucao {
        media_id: bloco[0..4].try_into().unwrap(),
        title_id: bloco[12..16].try_into().unwrap(),
        plataforma: bloco[16],
        tipo_executavel: bloco[17],
        disco_numero: bloco[18],
        disco_total: bloco[19],
    })
}

fn bytes_para_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Monta um XEX2 sintético mínimo: assinatura + cabeçalhos fixos +
    /// exatamente uma entrada na tabela de cabeçalhos opcionais, apontando
    /// para um bloco ExecutionInfo colocado logo em seguida.
    fn montar_xex_sintetico(execution_info: &[u8; TAMANHO_EXECUTION_INFO]) -> Vec<u8> {
        let mut xex = Vec::new();
        xex.extend_from_slice(ASSINATURA_XEX2); // offset 0: assinatura
        xex.extend_from_slice(&0u32.to_be_bytes()); // offset 4: ModuleFlags (não usado)
        xex.extend_from_slice(&0u32.to_be_bytes()); // offset 8: CodeOffset (não usado)
        xex.extend_from_slice(&[0u8; 4]); // offset 12: padding até CertificateOffset
        xex.extend_from_slice(&0u32.to_be_bytes()); // offset 16: CertificateOffset (não usado)
        xex.extend_from_slice(&1u32.to_be_bytes()); // offset 20: num_entradas = 1

        let endereco_execution_info = 32u32; // logo após a única entrada de 8 bytes
        xex.extend_from_slice(&ID_EXECUTION_INFO.to_be_bytes()); // offset 24: chave
        xex.extend_from_slice(&endereco_execution_info.to_be_bytes()); // offset 28: valor (endereço)

        assert_eq!(xex.len(), endereco_execution_info as usize);
        xex.extend_from_slice(execution_info);
        xex
    }

    fn montar_execution_info(
        media_id: [u8; 4],
        title_id: [u8; 4],
        plataforma: u8,
        tipo_executavel: u8,
        disco_numero: u8,
        disco_total: u8,
    ) -> [u8; TAMANHO_EXECUTION_INFO] {
        let mut b = [0u8; TAMANHO_EXECUTION_INFO];
        b[0..4].copy_from_slice(&media_id);
        // b[4..12] = Version/BaseVersion, deixados em zero (não usados)
        b[12..16].copy_from_slice(&title_id);
        b[16] = plataforma;
        b[17] = tipo_executavel;
        b[18] = disco_numero;
        b[19] = disco_total;
        b
    }

    #[test]
    fn extrai_execution_info_de_um_xex_sintetico() {
        let info_bruta =
            montar_execution_info([0xAA, 0xBB, 0xCC, 0xDD], [0x4D, 0x53, 0x08, 0xBF], 2, 0, 1, 2);
        let xex = montar_xex_sintetico(&info_bruta);

        let info = ler_info_execucao(&xex).expect("deveria extrair ExecutionInfo");

        assert_eq!(info.media_id, [0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(info.title_id, [0x4D, 0x53, 0x08, 0xBF]);
        assert_eq!(info.plataforma, 2);
        assert_eq!(info.tipo_executavel, 0);
        assert_eq!(info.disco_numero, 1);
        assert_eq!(info.disco_total, 2);
        assert_eq!(info.title_id_hex(), "4D5308BF");
        assert_eq!(info.media_id_hex(), "AABBCCDD");
    }

    #[test]
    fn rejeita_arquivo_sem_assinatura_xex2() {
        let mut xex = vec![0u8; 64];
        xex[0..4].copy_from_slice(b"NADA");
        assert!(ler_info_execucao(&xex).is_err());
    }

    #[test]
    fn rejeita_xex_sem_bloco_execution_info() {
        let mut xex = Vec::new();
        xex.extend_from_slice(ASSINATURA_XEX2);
        xex.extend_from_slice(&[0u8; 16]); // ModuleFlags/CodeOffset/pad/CertOffset
        xex.extend_from_slice(&1u32.to_be_bytes()); // num_entradas = 1
        // uma entrada com uma chave desconhecida, não ExecutionInfo
        xex.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        xex.extend_from_slice(&0u32.to_be_bytes());

        assert!(ler_info_execucao(&xex).is_err());
    }

    #[test]
    fn tolera_cabecalho_truncado_sem_estourar_limites() {
        let mut xex = Vec::new();
        xex.extend_from_slice(ASSINATURA_XEX2);
        xex.extend_from_slice(&[0u8; 16]);
        xex.extend_from_slice(&5u32.to_be_bytes()); // diz que tem 5 entradas...
        // ...mas o arquivo acaba logo em seguida, sem nenhuma entrada de verdade.
        assert!(ler_info_execucao(&xex).is_err());
    }

    #[test]
    fn rejeita_endereco_de_execution_info_alem_do_arquivo() {
        let mut xex = Vec::new();
        xex.extend_from_slice(ASSINATURA_XEX2);
        xex.extend_from_slice(&[0u8; 16]);
        xex.extend_from_slice(&1u32.to_be_bytes());
        xex.extend_from_slice(&ID_EXECUTION_INFO.to_be_bytes());
        xex.extend_from_slice(&1_000_000u32.to_be_bytes()); // endereço absurdo

        assert!(ler_info_execucao(&xex).is_err());
    }
}
