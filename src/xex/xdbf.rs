//! XDBF: o banco de dados de título de um jogo de Xbox 360 (o recurso com o
//! nome do Title ID dentro do `default.xex`). Daqui saem o nome de exibição,
//! em cada idioma, e o ícone do jogo — o que o painel do console mostra.
//!
//! Formato (big-endian): cabeçalho "XDBF" de 24 bytes, tabela de entradas de
//! 18 bytes (namespace, id, deslocamento, tamanho), tabela de espaço livre de
//! 8 bytes por entrada, e os dados. Namespace 1 = metadados (o `XSTC` guarda o
//! idioma padrão), 2 = imagens (id 0x8000 = ícone do título, PNG), 3 = tabelas
//! de texto `XSTR`, uma por idioma (texto 0x8000 = nome do título).

use crate::erro::{Erro, Resultado};

const ID_TITULO: u64 = 0x8000;
const NS_METADADOS: u16 = 1;
const NS_IMAGENS: u16 = 2;
const NS_TEXTOS: u16 = 3;
const ID_XSTC: u64 = 0x5853_5443; // "XSTC"
const IDIOMA_INGLES: u64 = 1;
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Nome e ícone do jogo, quando o XDBF os traz.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TituloXdbf {
    pub titulo: Option<String>,
    pub icone_png: Option<Vec<u8>>,
}

fn u16_be(d: &[u8], p: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(p..p.checked_add(2)?)?.try_into().ok()?))
}
fn u32_be(d: &[u8], p: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(p..p.checked_add(4)?)?.try_into().ok()?))
}
fn u64_be(d: &[u8], p: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(p..p.checked_add(8)?)?.try_into().ok()?))
}

fn invalido(o_que: &str) -> Erro {
    Erro::IsoInvalida(format!("XDBF do jogo: {o_que}"))
}

/// Lê o nome (no idioma padrão do jogo, ou em inglês) e o ícone.
pub fn ler(xdbf: &[u8]) -> Resultado<TituloXdbf> {
    if xdbf.get(0..4) != Some(b"XDBF") {
        return Err(invalido("não começa com a assinatura XDBF"));
    }
    let tam_tabela = u32_be(xdbf, 8).ok_or_else(|| invalido("cabeçalho truncado"))? as usize;
    let n = u32_be(xdbf, 12).ok_or_else(|| invalido("cabeçalho truncado"))? as usize;
    let tam_livre = u32_be(xdbf, 16).ok_or_else(|| invalido("cabeçalho truncado"))? as usize;
    if n > tam_tabela {
        return Err(invalido("mais entradas que a tabela comporta"));
    }
    let base = tam_tabela
        .checked_mul(18)
        .and_then(|t| t.checked_add(tam_livre.checked_mul(8)?))
        .and_then(|t| t.checked_add(24))
        .ok_or_else(|| invalido("tabelas de tamanho absurdo"))?;

    let entrada = |ns: u16, id: u64| -> Option<&[u8]> {
        (0..n).find_map(|i| {
            let p = 24 + i * 18;
            if u16_be(xdbf, p)? != ns || u64_be(xdbf, p + 2)? != id {
                return None;
            }
            let off = base.checked_add(u32_be(xdbf, p + 10)? as usize)?;
            let tam = u32_be(xdbf, p + 14)? as usize;
            xdbf.get(off..off.checked_add(tam)?)
        })
    };

    let idioma_padrao = entrada(NS_METADADOS, ID_XSTC)
        .filter(|b| b.get(0..4) == Some(b"XSTC"))
        .and_then(|b| u32_be(b, 12))
        .map(u64::from)
        .unwrap_or(IDIOMA_INGLES);

    let titulo = [idioma_padrao, IDIOMA_INGLES]
        .iter()
        .find_map(|&idioma| entrada(NS_TEXTOS, idioma).and_then(|b| texto_xstr(b, ID_TITULO as u16)))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let icone_png = entrada(NS_IMAGENS, ID_TITULO)
        .filter(|b| b.starts_with(PNG))
        .map(<[u8]>::to_vec);

    Ok(TituloXdbf { titulo, icone_png })
}

/// Um texto de uma tabela `XSTR`: "XSTR", versão, tamanho, quantidade (u16) e
/// entradas (id u16, tamanho u16, UTF-8).
fn texto_xstr(xstr: &[u8], id: u16) -> Option<String> {
    if xstr.get(0..4) != Some(b"XSTR") {
        return None;
    }
    let n = u16_be(xstr, 12)? as usize;
    let mut p = 14;
    for _ in 0..n {
        let este = u16_be(xstr, p)?;
        let tam = u16_be(xstr, p + 2)? as usize;
        let texto = xstr.get(p + 4..p + 4 + tam)?;
        if este == id {
            return Some(String::from_utf8_lossy(texto).into_owned());
        }
        p += 4 + tam;
    }
    None
}

#[cfg(test)]
pub(crate) mod testes {
    use super::*;

    fn xstr(textos: &[(u16, &str)]) -> Vec<u8> {
        let mut corpo = Vec::new();
        for (id, t) in textos {
            corpo.extend_from_slice(&id.to_be_bytes());
            corpo.extend_from_slice(&(t.len() as u16).to_be_bytes());
            corpo.extend_from_slice(t.as_bytes());
        }
        let mut b = b"XSTR".to_vec();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&(corpo.len() as u32 + 2).to_be_bytes());
        b.extend_from_slice(&(textos.len() as u16).to_be_bytes());
        b.extend(corpo);
        b
    }

    /// XDBF de teste com o nome em inglês e japonês, idioma padrão
    /// `padrao` e, opcionalmente, um ícone.
    pub(crate) fn montar_xdbf(ingles: &str, japones: &str, padrao: u32, icone: Option<&[u8]>) -> Vec<u8> {
        let mut xstc = b"XSTC".to_vec();
        xstc.extend_from_slice(&1u32.to_be_bytes());
        xstc.extend_from_slice(&4u32.to_be_bytes());
        xstc.extend_from_slice(&padrao.to_be_bytes());
        let mut blobs: Vec<(u16, u64, Vec<u8>)> = vec![
            (NS_METADADOS, ID_XSTC, xstc),
            (NS_TEXTOS, 1, xstr(&[(0x0001, "outro texto"), (0x8000, ingles)])),
            (NS_TEXTOS, 2, xstr(&[(0x8000, japones)])),
        ];
        if let Some(png) = icone {
            blobs.push((NS_IMAGENS, ID_TITULO, png.to_vec()));
        }
        let n = blobs.len() as u32;
        let mut cab = b"XDBF".to_vec();
        cab.extend_from_slice(&0x1_0000u32.to_be_bytes());
        cab.extend_from_slice(&n.to_be_bytes()); // tamanho da tabela
        cab.extend_from_slice(&n.to_be_bytes()); // entradas usadas
        cab.extend_from_slice(&1u32.to_be_bytes()); // tabela de livres
        cab.extend_from_slice(&0u32.to_be_bytes());
        let mut dados = Vec::new();
        for (ns, id, b) in &blobs {
            cab.extend_from_slice(&ns.to_be_bytes());
            cab.extend_from_slice(&id.to_be_bytes());
            cab.extend_from_slice(&(dados.len() as u32).to_be_bytes());
            cab.extend_from_slice(&(b.len() as u32).to_be_bytes());
            dados.extend_from_slice(b);
        }
        cab.extend_from_slice(&[0u8; 8]); // uma entrada de espaço livre
        cab.extend(dados);
        cab
    }

    fn png_falso() -> Vec<u8> {
        let mut p = PNG.to_vec();
        p.extend_from_slice(b"resto do png");
        p
    }

    #[test]
    fn le_nome_no_idioma_padrao_e_icone() {
        let x = montar_xdbf("RESIDENT EVIL 5", "BIOHAZARD 5", 1, Some(&png_falso()));
        let t = ler(&x).unwrap();
        assert_eq!(t.titulo.as_deref(), Some("RESIDENT EVIL 5"));
        assert_eq!(t.icone_png, Some(png_falso()));
    }

    #[test]
    fn respeita_idioma_padrao_do_jogo() {
        let x = montar_xdbf("RESIDENT EVIL 5", "BIOHAZARD 5", 2, None);
        assert_eq!(ler(&x).unwrap().titulo.as_deref(), Some("BIOHAZARD 5"));
    }

    #[test]
    fn idioma_padrao_sem_nome_cai_no_ingles() {
        let x = montar_xdbf("RESIDENT EVIL 5", "BIOHAZARD 5", 7, None);
        assert_eq!(ler(&x).unwrap().titulo.as_deref(), Some("RESIDENT EVIL 5"));
    }

    #[test]
    fn imagem_que_nao_e_png_nao_vira_icone() {
        let x = montar_xdbf("X", "X", 1, Some(b"nao sou png"));
        assert_eq!(ler(&x).unwrap().icone_png, None);
    }

    #[test]
    fn xdbf_corrompido_nunca_entra_em_panico() {
        let base = montar_xdbf("RESIDENT EVIL 5", "BIOHAZARD 5", 1, Some(&png_falso()));
        for corte in 0..base.len() {
            let _ = ler(&base[..corte]);
        }
        for pos in 0..base.len() {
            for v in [0x00, 0xFF, 0x80] {
                let mut x = base.clone();
                x[pos] = v;
                let _ = ler(&x);
            }
        }
    }
}
