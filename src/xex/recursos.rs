//! Recursos embutidos na imagem do executável de um `default.xex` (Xbox 360).
//!
//! O nome de exibição e o ícone de um jogo de Xbox 360 não ficam no cabeçalho
//! do XEX: ficam num recurso XDBF dentro da imagem PE, que vem criptografada
//! (AES-128) e comprimida (compressão "básica", que só tira trechos zerados,
//! ou LZX). Equivalente à parte de `IsoDetails.readXex` do Iso2God original
//! que decifra o XEX e extrai o recurso com o nome do Title ID.
//!
//! Tudo que vem do arquivo é conferido antes de indexar ou alocar: um XEX
//! corrompido vira erro, nunca pânico nem alocação de gigabytes.

use aes::Aes128;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, KeyInit};

use crate::erro::{Erro, Resultado};

/// Chave AES dos XEX de console retail (pública, a mesma de toda ferramenta
/// que abre XEX). Decifra a chave do arquivo guardada nas informações de
/// segurança, que por sua vez decifra a imagem.
const CHAVE_RETAIL: [u8; 16] = [
    0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91,
];
/// Chave dos XEX de kit de desenvolvimento (toda em zero).
const CHAVE_DEVKIT: [u8; 16] = [0; 16];

/// Cabeçalhos opcionais do XEX2 usados aqui.
const ID_RECURSOS: u32 = 0x0000_02FF;
const ID_FORMATO_ARQUIVO: u32 = 0x0000_03FF;
const ID_ENDERECO_BASE: u32 = 0x0001_0201;

/// Deslocamentos dentro das informações de segurança do XEX2.
const SEG_TAMANHO_IMAGEM: usize = 0x004;
const SEG_ENDERECO_CARGA: usize = 0x110;
const SEG_CHAVE_ARQUIVO: usize = 0x150;

/// Teto para o tamanho declarado da imagem: executáveis de 360 têm dezenas de
/// MB; um valor muito acima disso é arquivo corrompido, não jogo.
const MAX_IMAGEM: usize = 256 * 1024 * 1024;
/// Um XEX real tem umas 15 entradas; mais que isto é lixo.
const MAX_CABECALHOS: usize = 256;
/// Blocos de dados de um frame LZX descomprimido.
const FRAME_LZX: usize = 0x8000;

fn u16_be(d: &[u8], pos: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        d.get(pos..pos.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn u32_be(d: &[u8], pos: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        d.get(pos..pos.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn corrompido(o_que: &str) -> Erro {
    Erro::IsoInvalida(format!("default.xex: {o_que}"))
}

/// Tabela de cabeçalhos opcionais: (chave, valor).
fn cabecalhos(xex: &[u8]) -> Resultado<Vec<(u32, u32)>> {
    if xex.get(0..4) != Some(b"XEX2") {
        return Err(corrompido("não começa com a assinatura XEX2"));
    }
    let n = u32_be(xex, 20).ok_or_else(|| corrompido("cabeçalho truncado"))? as usize;
    if n > MAX_CABECALHOS {
        return Err(corrompido("tabela de cabeçalhos opcionais grande demais"));
    }
    (0..n)
        .map(|i| {
            let pos = 24 + i * 8;
            match (u32_be(xex, pos), u32_be(xex, pos + 4)) {
                (Some(k), Some(v)) => Ok((k, v)),
                _ => Err(corrompido("tabela de cabeçalhos opcionais truncada")),
            }
        })
        .collect()
}

fn valor(cabs: &[(u32, u32)], chave: u32) -> Option<u32> {
    cabs.iter().find(|(k, _)| *k == chave).map(|(_, v)| *v)
}

fn decifrar_bloco(chave: &[u8; 16], bloco: &[u8; 16]) -> [u8; 16] {
    let aes = Aes128::new(GenericArray::from_slice(chave));
    let mut b = GenericArray::clone_from_slice(bloco);
    aes.decrypt_block(&mut b);
    b.into()
}

/// AES-128-CBC com IV zero, só a parte múltipla de 16 bytes (o resto não é
/// parte da imagem).
fn decifrar_cbc(chave: &[u8; 16], dados: &[u8]) -> Vec<u8> {
    let aes = Aes128::new(GenericArray::from_slice(chave));
    let mut anterior = [0u8; 16];
    let mut saida = Vec::with_capacity(dados.len() / 16 * 16);
    for bloco in dados.as_chunks::<16>().0 {
        let mut b = GenericArray::clone_from_slice(bloco);
        aes.decrypt_block(&mut b);
        for (x, p) in b.iter_mut().zip(anterior) {
            *x ^= p;
        }
        saida.extend_from_slice(&b);
        anterior.copy_from_slice(bloco);
    }
    saida
}

/// Monta a imagem PE a partir dos dados (já decifrados) do arquivo.
fn descomprimir(
    xex: &[u8],
    formato: usize,
    compressao: u16,
    dados: &[u8],
    tamanho_imagem: usize,
) -> Resultado<Vec<u8>> {
    match compressao {
        // sem compressão: os dados são a imagem
        0 => Ok(dados
            .get(..tamanho_imagem.min(dados.len()))
            .unwrap_or(dados)
            .to_vec()),
        // básica: pares (bytes de dados, bytes de zero)
        1 => {
            let tam_info =
                u32_be(xex, formato).ok_or_else(|| corrompido("formato truncado"))? as usize;
            let fim = formato
                .checked_add(tam_info)
                .filter(|f| *f <= xex.len())
                .ok_or_else(|| corrompido("formato de arquivo além do fim"))?;
            let mut imagem = Vec::new();
            let (mut pos, mut lido) = (formato + 8, 0usize);
            while pos + 8 <= fim {
                let (d, z) = (
                    u32_be(xex, pos).unwrap() as usize,
                    u32_be(xex, pos + 4).unwrap() as usize,
                );
                pos += 8;
                let trecho = lido
                    .checked_add(d)
                    .and_then(|f| dados.get(lido..f))
                    .ok_or_else(|| corrompido("bloco comprimido além do fim dos dados"))?;
                if imagem.len() + d + z > tamanho_imagem.max(MAX_IMAGEM) {
                    return Err(corrompido("imagem maior que o tamanho declarado"));
                }
                imagem.extend_from_slice(trecho);
                imagem.resize(imagem.len() + z, 0);
                lido += d;
            }
            Ok(imagem)
        }
        // normal: blocos encadeados com frames LZX
        2 => {
            let janela = u32_be(xex, formato + 8).ok_or_else(|| corrompido("formato truncado"))?;
            let janela = janela_lzx(janela)?;
            let mut tamanho =
                u32_be(xex, formato + 12).ok_or_else(|| corrompido("formato truncado"))? as usize;
            let mut lzx = lzxd::Lzxd::new(janela);
            let mut imagem = Vec::with_capacity(tamanho_imagem);
            let mut pos = 0usize;
            while tamanho > 0 {
                let bloco = pos
                    .checked_add(tamanho)
                    .and_then(|f| dados.get(pos..f))
                    .ok_or_else(|| corrompido("bloco LZX além do fim dos dados"))?;
                let proximo =
                    u32_be(bloco, 0).ok_or_else(|| corrompido("bloco LZX truncado"))? as usize;
                let mut q = 24; // tamanho do próximo bloco (4) + hash SHA1 (20)
                loop {
                    let n =
                        u16_be(bloco, q).ok_or_else(|| corrompido("frame LZX truncado"))? as usize;
                    q += 2;
                    if n == 0 {
                        break;
                    }
                    let frame = bloco
                        .get(q..q + n)
                        .ok_or_else(|| corrompido("frame LZX além do bloco"))?;
                    q += n;
                    let falta = tamanho_imagem.saturating_sub(imagem.len());
                    if falta == 0 {
                        return Err(corrompido("dados LZX além do tamanho declarado da imagem"));
                    }
                    let saida = lzx
                        .decompress_next(frame, falta.min(FRAME_LZX))
                        .map_err(|e| corrompido(&format!("LZX inválido: {e}")))?;
                    imagem.extend_from_slice(saida);
                }
                pos += tamanho;
                tamanho = proximo;
            }
            Ok(imagem)
        }
        3 => Err(corrompido(
            "XEX de patch (compressão delta) não tem recursos",
        )),
        _ => Err(corrompido("tipo de compressão desconhecido")),
    }
}

fn janela_lzx(tamanho: u32) -> Resultado<lzxd::WindowSize> {
    use lzxd::WindowSize as J;
    Ok(match tamanho {
        0x0000_8000 => J::KB32,
        0x0001_0000 => J::KB64,
        0x0002_0000 => J::KB128,
        0x0004_0000 => J::KB256,
        0x0008_0000 => J::KB512,
        0x0010_0000 => J::MB1,
        0x0020_0000 => J::MB2,
        _ => return Err(corrompido(&format!("janela LZX inválida ({tamanho:#x})"))),
    })
}

/// Devolve os bytes do recurso chamado `nome` (normalmente o Title ID em
/// hexadecimal, como "434307D4") embutido na imagem do XEX.
pub fn extrair_recurso(xex: &[u8], nome: &str) -> Resultado<Vec<u8>> {
    let cabs = cabecalhos(xex)?;
    let seg = u32_be(xex, 16).ok_or_else(|| corrompido("cabeçalho truncado"))? as usize;
    let inicio_dados = u32_be(xex, 8).ok_or_else(|| corrompido("cabeçalho truncado"))? as usize;
    let dados = xex
        .get(inicio_dados..)
        .ok_or_else(|| corrompido("dados além do fim do arquivo"))?;

    // tabela de recursos: (nome de 8 bytes, endereço, tamanho)
    let tabela = valor(&cabs, ID_RECURSOS)
        .ok_or_else(|| corrompido("não declara recursos (nome e ícone indisponíveis)"))?
        as usize;
    let tam_tabela =
        u32_be(xex, tabela).ok_or_else(|| corrompido("tabela de recursos truncada"))? as usize;
    let (endereco, tamanho) = (0..tam_tabela.saturating_sub(4) / 16)
        .filter_map(|i| {
            let e = xex.get(tabela + 4 + i * 16..tabela + 20 + i * 16)?;
            let nome_e = String::from_utf8_lossy(&e[..8])
                .trim_end_matches('\0')
                .to_string();
            nome_e
                .eq_ignore_ascii_case(nome)
                .then(|| (u32_be(e, 8), u32_be(e, 12)))
        })
        .find_map(|(a, t)| Some((a? as usize, t? as usize)))
        .ok_or_else(|| corrompido(&format!("não tem o recurso {nome}")))?;

    let formato = valor(&cabs, ID_FORMATO_ARQUIVO)
        .ok_or_else(|| corrompido("não declara o formato do arquivo"))? as usize;
    let cripto = u16_be(xex, formato + 4).ok_or_else(|| corrompido("formato truncado"))?;
    let compressao = u16_be(xex, formato + 6).ok_or_else(|| corrompido("formato truncado"))?;

    let tamanho_imagem = u32_be(xex, seg + SEG_TAMANHO_IMAGEM)
        .ok_or_else(|| corrompido("informações de segurança truncadas"))?
        as usize;
    if tamanho_imagem > MAX_IMAGEM {
        return Err(corrompido("tamanho de imagem absurdo"));
    }
    let base = match valor(&cabs, ID_ENDERECO_BASE) {
        Some(b) => b,
        None => u32_be(xex, seg + SEG_ENDERECO_CARGA)
            .ok_or_else(|| corrompido("informações de segurança truncadas"))?,
    } as usize;
    let deslocamento = endereco
        .checked_sub(base)
        .ok_or_else(|| corrompido("recurso fora da imagem"))?;

    let imagem = match cripto {
        0 => descomprimir(xex, formato, compressao, dados, tamanho_imagem)?,
        1 => {
            let chave_arquivo: [u8; 16] = xex
                .get(seg + SEG_CHAVE_ARQUIVO..seg + SEG_CHAVE_ARQUIVO + 16)
                .ok_or_else(|| corrompido("informações de segurança truncadas"))?
                .try_into()
                .unwrap();
            // Retail primeiro; se a imagem não sair um executável ("MZ"),
            // tenta a chave de kit de desenvolvimento.
            let mut ultima = None;
            let mut achada = None;
            for mestra in [CHAVE_RETAIL, CHAVE_DEVKIT] {
                let sessao = decifrar_bloco(&mestra, &chave_arquivo);
                let claros = decifrar_cbc(&sessao, dados);
                match descomprimir(xex, formato, compressao, &claros, tamanho_imagem) {
                    Ok(img) if img.get(0..2) == Some(b"MZ") => {
                        achada = Some(img);
                        break;
                    }
                    Ok(_) => ultima = Some(corrompido("nenhuma chave decifrou a imagem")),
                    Err(e) => ultima = Some(e),
                }
            }
            achada.ok_or_else(|| ultima.unwrap_or_else(|| corrompido("não decifrou")))?
        }
        _ => return Err(corrompido("tipo de criptografia desconhecido")),
    };

    deslocamento
        .checked_add(tamanho)
        .and_then(|fim| imagem.get(deslocamento..fim))
        .map(<[u8]>::to_vec)
        .ok_or_else(|| corrompido(&format!("o recurso {nome} aponta além do fim da imagem")))
}

#[cfg(test)]
pub(crate) mod testes {
    use super::*;

    /// Cifra com AES-128-CBC (IV zero): só para montar XEX de teste.
    fn cifrar_cbc(chave: &[u8; 16], dados: &[u8]) -> Vec<u8> {
        use aes::cipher::BlockEncrypt;
        let aes = Aes128::new(GenericArray::from_slice(chave));
        let mut anterior = [0u8; 16];
        let mut saida = Vec::new();
        for bloco in dados.as_chunks::<16>().0 {
            let mut b = GenericArray::clone_from_slice(bloco);
            for (x, p) in b.iter_mut().zip(anterior) {
                *x ^= p;
            }
            aes.encrypt_block(&mut b);
            saida.extend_from_slice(&b);
            anterior.copy_from_slice(&b);
        }
        saida
    }

    fn cifrar_bloco(chave: &[u8; 16], bloco: &[u8; 16]) -> [u8; 16] {
        use aes::cipher::BlockEncrypt;
        let aes = Aes128::new(GenericArray::from_slice(chave));
        let mut b = GenericArray::clone_from_slice(bloco);
        aes.encrypt_block(&mut b);
        b.into()
    }

    /// Monta um XEX2 com o `recurso` chamado `nome` dentro de uma imagem PE
    /// sintética, cifrada com a chave retail e com a compressão pedida
    /// (1 = básica, 2 = LZX). Serve de fixture para os testes.
    pub(crate) fn montar_xex(nome: &str, recurso: &[u8], compressao: u16) -> Vec<u8> {
        const BASE: u32 = 0x8200_0000;
        const OFF_RECURSO: usize = 0x3000;
        // imagem: "MZ", zeros, o recurso, e dados pseudoaleatórios
        let mut imagem = vec![0u8; OFF_RECURSO];
        imagem[0..2].copy_from_slice(b"MZ");
        imagem.extend_from_slice(recurso);
        let mut x = 0x1234_5678u32;
        for _ in 0..70_000 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            imagem.push((x % 7) as u8);
        }
        while !imagem.len().is_multiple_of(16) {
            imagem.push(0);
        }

        let (formato, dados): (Vec<u8>, Vec<u8>) = match compressao {
            1 => {
                // um bloco de dados e um de zeros no fim
                let mut f = Vec::new();
                f.extend_from_slice(&16u32.to_be_bytes()); // tamanho da info
                f.extend_from_slice(&1u16.to_be_bytes()); // AES
                f.extend_from_slice(&1u16.to_be_bytes()); // básica
                f.extend_from_slice(&(imagem.len() as u32).to_be_bytes());
                f.extend_from_slice(&0x2000u32.to_be_bytes());
                (f, imagem.clone())
            }
            2 => {
                // frames de 32 KB, cada um "u16 tamanho | dados", num bloco só
                let mut enc = lzxc::Encoder::new(lzxc::WindowSize::KB64);
                let mut bloco = vec![0u8; 24]; // próximo bloco = 0 + hash (não conferido)
                for trecho in imagem.chunks(FRAME_LZX) {
                    let c = enc.encode_chunk(trecho);
                    bloco.extend_from_slice(&(c.len() as u16).to_be_bytes());
                    bloco.extend_from_slice(&c);
                }
                bloco.extend_from_slice(&0u16.to_be_bytes());
                while !bloco.len().is_multiple_of(16) {
                    bloco.push(0);
                }
                let mut f = Vec::new();
                f.extend_from_slice(&36u32.to_be_bytes());
                f.extend_from_slice(&1u16.to_be_bytes()); // AES
                f.extend_from_slice(&2u16.to_be_bytes()); // LZX
                f.extend_from_slice(&0x1_0000u32.to_be_bytes()); // janela 64 KB
                f.extend_from_slice(&(bloco.len() as u32).to_be_bytes());
                f.extend_from_slice(&[0u8; 20]);
                (f, bloco)
            }
            _ => unreachable!(),
        };

        let tamanho_imagem = imagem.len() as u32 + if compressao == 1 { 0x2000 } else { 0 };
        let chave_arquivo_clara = [0x5Au8; 16];
        let chave_arquivo_cifrada = cifrar_bloco(&CHAVE_RETAIL, &chave_arquivo_clara);

        // layout: cabeçalho, 3 opcionais, segurança em 0x100, recursos em
        // 0x300, formato em 0x340, dados em 0x1000
        let mut xex = vec![0u8; 0x1000];
        xex[0..4].copy_from_slice(b"XEX2");
        xex[8..12].copy_from_slice(&0x1000u32.to_be_bytes());
        xex[16..20].copy_from_slice(&0x100u32.to_be_bytes());
        xex[20..24].copy_from_slice(&3u32.to_be_bytes());
        for (i, (k, v)) in [
            (ID_RECURSOS, 0x300u32),
            (ID_FORMATO_ARQUIVO, 0x340),
            (ID_ENDERECO_BASE, BASE),
        ]
        .iter()
        .enumerate()
        {
            xex[24 + i * 8..28 + i * 8].copy_from_slice(&k.to_be_bytes());
            xex[28 + i * 8..32 + i * 8].copy_from_slice(&v.to_be_bytes());
        }
        xex[0x100 + SEG_TAMANHO_IMAGEM..0x104 + SEG_TAMANHO_IMAGEM]
            .copy_from_slice(&tamanho_imagem.to_be_bytes());
        xex[0x100 + SEG_ENDERECO_CARGA..0x104 + SEG_ENDERECO_CARGA]
            .copy_from_slice(&BASE.to_be_bytes());
        xex[0x100 + SEG_CHAVE_ARQUIVO..0x110 + SEG_CHAVE_ARQUIVO]
            .copy_from_slice(&chave_arquivo_cifrada);
        xex[0x300..0x304].copy_from_slice(&20u32.to_be_bytes());
        let mut n = [0u8; 8];
        n[..nome.len()].copy_from_slice(nome.as_bytes());
        xex[0x304..0x30C].copy_from_slice(&n);
        xex[0x30C..0x310].copy_from_slice(&(BASE + OFF_RECURSO as u32).to_be_bytes());
        xex[0x310..0x314].copy_from_slice(&(recurso.len() as u32).to_be_bytes());
        xex[0x340..0x340 + formato.len()].copy_from_slice(&formato);
        xex.extend_from_slice(&cifrar_cbc(&chave_arquivo_clara, &dados));
        xex
    }

    #[test]
    fn extrai_recurso_de_xex_com_compressao_basica() {
        let recurso = b"XDBF recurso de teste 1234567890".repeat(20);
        let xex = montar_xex("4D5308BF", &recurso, 1);
        assert_eq!(extrair_recurso(&xex, "4D5308BF").unwrap(), recurso);
        assert_eq!(
            extrair_recurso(&xex, "4d5308bf").unwrap(),
            recurso,
            "nome sem diferenciar caixa"
        );
    }

    #[test]
    fn extrai_recurso_de_xex_com_lzx() {
        let recurso = b"XDBF recurso em LZX ".repeat(300);
        let xex = montar_xex("434307D4", &recurso, 2);
        assert_eq!(extrair_recurso(&xex, "434307D4").unwrap(), recurso);
    }

    #[test]
    fn recurso_inexistente_e_erro_e_nao_panico() {
        let xex = montar_xex("4D5308BF", b"XDBF", 1);
        assert!(extrair_recurso(&xex, "DEADBEEF").is_err());
    }

    #[test]
    fn xex_truncado_em_qualquer_ponto_nunca_entra_em_panico() {
        for compressao in [1, 2] {
            let xex = montar_xex("4D5308BF", &b"XDBF".repeat(50), compressao);
            for corte in (0..xex.len()).step_by(97) {
                let _ = extrair_recurso(&xex[..corte], "4D5308BF");
            }
        }
    }

    #[test]
    fn bytes_corrompidos_nunca_entram_em_panico() {
        for compressao in [1, 2] {
            let base = montar_xex("4D5308BF", &b"XDBF".repeat(50), compressao);
            for pos in (0..0x400).chain((0x1000..base.len()).step_by(211)) {
                for v in [0x00, 0xFF, 0x7F] {
                    let mut x = base.clone();
                    x[pos] = v;
                    let _ = extrair_recurso(&x, "4D5308BF");
                }
            }
        }
    }
}
