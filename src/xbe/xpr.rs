//! Decodificação de texturas XPR (o formato usado pelo Xbox original para
//! guardar o thumbnail do jogo dentro do XBE) para um buffer RGBA8 comum.
//! Equivalente a `Chilano.Xbox360.Graphics.XPR` + `DDS`, restrito aos dois
//! formatos usados em thumbnails: DXT1 (comprimido em blocos) e ARGB (bruto,
//! mas com os pixels embaralhados em "ladrilhos" — swizzled — como a GPU do
//! Xbox original guarda texturas na memória).

use crate::erro::{Erro, Resultado};

/// Assinatura no início de todo arquivo/seção XPR: "XPR0" em ASCII, como
/// `u32` little-endian.
const ASSINATURA_XPR0: u32 = 810_700_888;

const FORMATO_DXT1: u8 = 12;
const FORMATO_ARGB: u8 = 6;
/// Igual ao ARGB, mas o quarto byte não é alfa (a imagem é opaca). Usado
/// por alguns jogos, como o Darkwatch.
const FORMATO_XRGB: u8 = 7;
/// Maior lado de miniatura cujo PNG ainda pode caber no cabeçalho (ver
/// `decodificar`). As miniaturas do Xbox têm de 64×64 a 256×256.
const LADO_MAXIMO: u32 = 2048;

/// Uma textura já decodificada para RGBA8 (4 bytes por pixel, linha a
/// linha, sem preenchimento entre linhas).
pub struct Textura {
    pub largura: u32,
    pub altura: u32,
    pub rgba: Vec<u8>,
}

/// Decodifica uma textura XPR (DXT1 ou ARGB) para RGBA8.
/// Equivalente a `XPR` + `XPR.ConvertToDDS` + `DDS.GetImage`.
pub fn decodificar(dados: &[u8]) -> Resultado<Textura> {
    if dados.len() < 28 {
        return Err(Erro::IsoInvalida(
            "dados XPR pequenos demais para ter um cabeçalho".into(),
        ));
    }
    let magic = u32::from_le_bytes(dados[0..4].try_into().unwrap());
    if magic != ASSINATURA_XPR0 {
        return Err(Erro::IsoInvalida(
            "dados não começam com a assinatura XPR0 esperada".into(),
        ));
    }

    let file_size = u32::from_le_bytes(dados[4..8].try_into().unwrap());
    let header_size = u32::from_le_bytes(dados[8..12].try_into().unwrap());
    let formato = dados[25];
    // O original usa o mesmo campo (TextureRes2) tanto para largura quanto
    // para altura — só funciona porque na prática essas texturas de
    // thumbnail são sempre quadradas, e replicamos isso fielmente.
    let expoente_tamanho = dados[27];
    let tamanho = 1u32.checked_shl(expoente_tamanho as u32).ok_or_else(|| {
        Erro::IsoInvalida(format!(
            "expoente de tamanho de textura XPR inválido: {expoente_tamanho}"
        ))
    })?;
    // A miniatura vira um PNG que tem que caber nos 16 KiB do cabeçalho. Um
    // PNG RGBA de lado 4096 tem pelo menos 4096 × (1 + 4 × 4096) / 1032 ≈
    // 65 KB (1032:1 é a maior compressão possível do deflate): seria
    // descartado de qualquer jeito, depois de alocar e comprimir até 4 GiB
    // de pixels.
    if tamanho > LADO_MAXIMO {
        return Err(Erro::IsoInvalida(format!(
            "textura XPR declara um tamanho impossível para uma miniatura \
             ({tamanho}x{tamanho}; o PNG não caberia no cabeçalho)"
        )));
    }

    let inicio_imagem = header_size as usize;
    let fim_imagem = file_size as usize;
    if inicio_imagem > fim_imagem || fim_imagem > dados.len() {
        return Err(Erro::IsoInvalida(
            "cabeçalho XPR declara um tamanho de imagem além do fim dos dados".into(),
        ));
    }
    let imagem = &dados[inicio_imagem..fim_imagem];

    let rgba = match formato {
        FORMATO_DXT1 => decodificar_dxt1(tamanho, tamanho, imagem)?,
        FORMATO_ARGB => decodificar_argb_swizzled(tamanho, tamanho, imagem, false)?,
        FORMATO_XRGB => decodificar_argb_swizzled(tamanho, tamanho, imagem, true)?,
        outro => {
            return Err(Erro::IsoInvalida(format!(
                "formato de textura XPR não suportado: {outro} (só DXT1, ARGB e XRGB)"
            )));
        }
    };

    Ok(Textura {
        largura: tamanho,
        altura: tamanho,
        rgba,
    })
}

/// Quantos bytes ocupa uma imagem RGBA de `largura` x `altura`, ou erro se a
/// conta não couber em `usize`.
///
/// Isso não é paranoia: a dimensão da textura vem de um **expoente** lido do
/// arquivo (`1 << dados[27]`), então `2^31 x 2^31` é uma entrada trivial de
/// forjar — e `2^31 * 2^31 * 4` estoura um `usize` de 64 bits, o que dava
/// pânico de multiplicação em debug e, em release, um tamanho embaralhado
/// que levava a um acesso fora dos limites logo adiante.
fn bytes_rgba(largura: u32, altura: u32) -> Resultado<usize> {
    (largura as usize)
        .checked_mul(altura as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| {
            Erro::IsoInvalida(format!(
                "textura XPR declara um tamanho impossível ({largura}x{altura})"
            ))
        })
}

/// Expande um componente de 5 bits (0..31) para 8 bits, com a mesma
/// aritmética inteira do original (equivalente a `round(v * 255 / 31)`,
/// calculado sem ponto flutuante).
fn expandir_5_bits(v: u32) -> u8 {
    let t = v * 255 + 16;
    ((t / 32 + t) / 32) as u8
}

/// Igual a `expandir_5_bits`, mas para um componente de 6 bits (0..63).
fn expandir_6_bits(v: u32) -> u8 {
    let t = v * 255 + 32;
    ((t / 64 + t) / 64) as u8
}

/// Decodifica um bloco DXT1 (BC1) de 4x4 pixels, 8 bytes por bloco: duas
/// cores RGB565 seguidas de uma tabela de índices de 2 bits por pixel.
/// Equivalente a `DDS.decompressBlockDXT1`.
fn decodificar_bloco_dxt1(bloco: &[u8; 8]) -> [[u8; 4]; 16] {
    let cor0 = u16::from_le_bytes([bloco[0], bloco[1]]);
    let cor1 = u16::from_le_bytes([bloco[2], bloco[3]]);

    let r0 = expandir_5_bits(((cor0 >> 11) & 0x1F) as u32);
    let g0 = expandir_6_bits(((cor0 >> 5) & 0x3F) as u32);
    let b0 = expandir_5_bits((cor0 & 0x1F) as u32);
    let r1 = expandir_5_bits(((cor1 >> 11) & 0x1F) as u32);
    let g1 = expandir_6_bits(((cor1 >> 5) & 0x3F) as u32);
    let b1 = expandir_5_bits((cor1 & 0x1F) as u32);

    let indices = u32::from_le_bytes([bloco[4], bloco[5], bloco[6], bloco[7]]);

    let modo_4_cores = cor0 > cor1;
    let mut pixels = [[0u8; 4]; 16];

    for i in 0..4u32 {
        for j in 0..4u32 {
            let codigo = ((indices >> (2 * (4 * i + j))) & 3) as u8;
            let cor = if modo_4_cores {
                match codigo {
                    0 => [r0, g0, b0, 255],
                    1 => [r1, g1, b1, 255],
                    2 => [
                        ((2 * r0 as u32 + r1 as u32) / 3) as u8,
                        ((2 * g0 as u32 + g1 as u32) / 3) as u8,
                        ((2 * b0 as u32 + b1 as u32) / 3) as u8,
                        255,
                    ],
                    _ => [
                        ((r0 as u32 + 2 * r1 as u32) / 3) as u8,
                        ((g0 as u32 + 2 * g1 as u32) / 3) as u8,
                        ((b0 as u32 + 2 * b1 as u32) / 3) as u8,
                        255,
                    ],
                }
            } else {
                match codigo {
                    0 => [r0, g0, b0, 255],
                    1 => [r1, g1, b1, 255],
                    2 => [
                        ((r0 as u32 + r1 as u32) / 2) as u8,
                        ((g0 as u32 + g1 as u32) / 2) as u8,
                        ((b0 as u32 + b1 as u32) / 2) as u8,
                        255,
                    ],
                    _ => [0, 0, 0, 255],
                }
            };
            pixels[(i * 4 + j) as usize] = cor;
        }
    }

    pixels
}

/// Decodifica uma imagem inteira comprimida em DXT1, bloco a bloco, em
/// ordem raster (sem "des-ladrilhamento" — o original não faz isso para
/// DXT1, só para ARGB). Equivalente a `DDS.blockDecompressImageDXT1`.
fn decodificar_dxt1(largura: u32, altura: u32, dados: &[u8]) -> Resultado<Vec<u8>> {
    let blocos_x = largura.div_ceil(4);
    let blocos_y = altura.div_ceil(4);
    let bytes_necessarios = (blocos_x as usize)
        .checked_mul(blocos_y as usize)
        .and_then(|blocos| blocos.checked_mul(8))
        .ok_or_else(|| {
            Erro::IsoInvalida(format!(
                "textura XPR declara um tamanho impossível ({largura}x{altura})"
            ))
        })?;
    if dados.len() < bytes_necessarios {
        return Err(Erro::IsoInvalida(
            "dados DXT1 insuficientes para o tamanho de textura declarado".into(),
        ));
    }

    let mut rgba = vec![0u8; bytes_rgba(largura, altura)?];
    let mut posicao = 0usize;

    for by in 0..blocos_y {
        for bx in 0..blocos_x {
            let bloco: [u8; 8] = dados[posicao..posicao + 8].try_into().unwrap();
            posicao += 8;
            let pixels = decodificar_bloco_dxt1(&bloco);

            for i in 0..4u32 {
                for j in 0..4u32 {
                    let x = bx * 4 + j;
                    let y = by * 4 + i;
                    if x >= largura || y >= altura {
                        continue;
                    }
                    let cor = pixels[(i * 4 + j) as usize];
                    let destino = (y as usize * largura as usize + x as usize) * 4;
                    rgba[destino..destino + 4].copy_from_slice(&cor);
                }
            }
        }
    }

    Ok(rgba)
}

/// Decodifica uma imagem ARGB bruta (4 bytes por pixel, na ordem B,G,R,A),
/// desfazendo primeiro o "ladrilhamento" (swizzle) usado pela GPU do Xbox
/// original para guardar texturas na memória. Equivalente a
/// `DDS.rgbaDecompressImage` + `DDS.UnswizzleRect`.
/// Com `opaco`, o quarto byte é ignorado e o alfa vira 255 (formato XRGB).
fn decodificar_argb_swizzled(
    largura: u32,
    altura: u32,
    dados: &[u8],
    opaco: bool,
) -> Resultado<Vec<u8>> {
    let tamanho_necessario = bytes_rgba(largura, altura)?;
    if dados.len() < tamanho_necessario {
        return Err(Erro::IsoInvalida(
            "dados ARGB insuficientes para o tamanho de textura declarado".into(),
        ));
    }

    let desembaralhado = desembaralhar(dados, largura, altura, 4);

    let mut rgba = vec![0u8; tamanho_necessario];
    for pixel in 0..(largura as usize * altura as usize) {
        let b = desembaralhado[pixel * 4];
        let g = desembaralhado[pixel * 4 + 1];
        let r = desembaralhado[pixel * 4 + 2];
        let a = if opaco {
            0xFF
        } else {
            desembaralhado[pixel * 4 + 3]
        };
        rgba[pixel * 4..pixel * 4 + 4].copy_from_slice(&[r, g, b, a]);
    }

    Ok(rgba)
}

/// Desfaz o "ladrilhamento" (swizzle): a GPU do Xbox original guarda
/// texturas na memória numa ordem de bytes intercalada (parecida com um
/// código de Morton) em vez de linha a linha. Equivalente a
/// `DDS.UnswizzleBox`/`GenerateSwizzleMasks`/`GetSwizzledOffset`/`FillPattern`.
/// Só é chamada depois de o tamanho já ter sido validado contra o tamanho
/// real dos dados (ver `decodificar_argb_swizzled`), então aqui a alocação
/// não pode estourar.
fn desembaralhar(origem: &[u8], largura: u32, altura: u32, bytes_por_pixel: u32) -> Vec<u8> {
    let (mascara_x, mascara_y) = gerar_mascaras_swizzle(largura, altura);

    let mut destino = vec![0u8; largura as usize * altura as usize * bytes_por_pixel as usize];
    for y in 0..altura {
        for x in 0..largura {
            let offset_origem = deslocamento_embaralhado(x, y, mascara_x, mascara_y) as usize
                * bytes_por_pixel as usize;
            let offset_destino =
                (y as usize * largura as usize + x as usize) * bytes_por_pixel as usize;
            if offset_origem + bytes_por_pixel as usize <= origem.len() {
                destino[offset_destino..offset_destino + bytes_por_pixel as usize].copy_from_slice(
                    &origem[offset_origem..offset_origem + bytes_por_pixel as usize],
                );
            }
        }
    }
    destino
}

fn gerar_mascaras_swizzle(largura: u32, altura: u32) -> (u32, u32) {
    let (mut mascara_x, mut mascara_y) = (0u32, 0u32);
    let mut bit = 1u32;
    let mut bit_mascara = 1u32;
    loop {
        let mut terminou = true;
        if bit < largura {
            mascara_x |= bit_mascara;
            bit_mascara <<= 1;
            terminou = false;
        }
        if bit < altura {
            mascara_y |= bit_mascara;
            bit_mascara <<= 1;
            terminou = false;
        }
        bit <<= 1;
        if terminou {
            break;
        }
    }
    (mascara_x, mascara_y)
}

fn deslocamento_embaralhado(x: u32, y: u32, mascara_x: u32, mascara_y: u32) -> u32 {
    preencher_padrao(mascara_x, x) | preencher_padrao(mascara_y, y)
}

fn preencher_padrao(padrao: u32, mut valor: u32) -> u32 {
    let mut resultado = 0u32;
    let mut bit = 1u32;
    while valor != 0 {
        if padrao & bit != 0 {
            if valor & 1 != 0 {
                resultado |= bit;
            }
            valor >>= 1;
        }
        bit <<= 1;
    }
    resultado
}

/// Codifica um buffer RGBA8 como PNG.
pub fn rgba_para_png(largura: u32, altura: u32, rgba: &[u8]) -> Resultado<Vec<u8>> {
    let mut saida = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut saida, largura, altura);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut escritor = encoder
            .write_header()
            .map_err(|e| Erro::IsoInvalida(format!("falha ao escrever cabeçalho PNG: {e}")))?;
        escritor
            .write_image_data(rgba)
            .map_err(|e| Erro::IsoInvalida(format!("falha ao codificar dados PNG: {e}")))?;
    }
    Ok(saida)
}

#[cfg(test)]
mod testes {
    use super::*;

    fn montar_xpr(formato: u8, tamanho_lado: u8, imagem: &[u8]) -> Vec<u8> {
        let mut xpr = Vec::new();
        xpr.extend_from_slice(&ASSINATURA_XPR0.to_le_bytes());
        let header_size = 28u32;
        xpr.extend_from_slice(&(header_size + imagem.len() as u32).to_le_bytes()); // FileSize
        xpr.extend_from_slice(&header_size.to_le_bytes()); // HeaderSize
        xpr.extend_from_slice(&0u32.to_le_bytes()); // TextureCommon
        xpr.extend_from_slice(&0u32.to_le_bytes()); // TextureData
        xpr.extend_from_slice(&0u32.to_le_bytes()); // TextureLock
        xpr.push(0); // TextureMisc1
        xpr.push(formato);
        xpr.push(0); // TextureRes1
        xpr.push(tamanho_lado); // TextureRes2 (expoente: 2^tamanho_lado)
        xpr.extend_from_slice(imagem);
        xpr
    }

    /// Um bloco DXT1 4x4 com color0 = vermelho puro (RGB565) e color1 = azul
    /// puro, índices todos zero (usa color0 em todo o bloco). Como
    /// color0 > color1, entra no modo de 4 cores, e código 0 sempre mapeia
    /// para color0 — o bloco inteiro deveria decodificar para vermelho
    /// (aproximadamente 255,0,0, já que a expansão de 5 bits arredonda).
    #[test]
    fn bloco_dxt1_solido_decodifica_para_a_cor_esperada() {
        let color0: u16 = 0xF800; // RGB565: R=31,G=0,B=0
        let color1: u16 = 0x001F; // RGB565: R=0,G=0,B=31
        assert!(color0 > color1);

        let mut bloco = [0u8; 8];
        bloco[0..2].copy_from_slice(&color0.to_le_bytes());
        bloco[2..4].copy_from_slice(&color1.to_le_bytes());
        // indices = 0 (todos code 0 -> color0)

        let pixels = decodificar_bloco_dxt1(&bloco);
        for p in pixels {
            assert_eq!(p, [255, 0, 0, 255], "pixel deveria ser vermelho sólido");
        }
    }

    #[test]
    fn expandir_5_bits_e_6_bits_cobrem_toda_a_faixa() {
        assert_eq!(expandir_5_bits(0), 0);
        assert_eq!(expandir_5_bits(31), 255);
        assert_eq!(expandir_6_bits(0), 0);
        assert_eq!(expandir_6_bits(63), 255);
    }

    #[test]
    fn decodificar_xpr_dxt1_completo_bate_com_bloco_isolado() {
        let color0: u16 = 0xF800; // RGB565: R=31,G=0,B=0
        let color1: u16 = 0x001F; // RGB565: R=0,G=0,B=31
        let mut bloco = [0u8; 8];
        bloco[0..2].copy_from_slice(&color0.to_le_bytes());
        bloco[2..4].copy_from_slice(&color1.to_le_bytes());

        // tamanho_lado = 2 -> largura=altura=2^2=4 (exatamente um bloco DXT1)
        let xpr = montar_xpr(FORMATO_DXT1, 2, &bloco);
        let textura = decodificar(&xpr).expect("deveria decodificar");

        assert_eq!(textura.largura, 4);
        assert_eq!(textura.altura, 4);
        assert_eq!(textura.rgba.len(), 4 * 4 * 4);
        for pixel in textura.rgba.as_chunks::<4>().0 {
            assert_eq!(pixel, &[255, 0, 0, 255]);
        }
    }

    /// Verificação manual do desembaralhamento (swizzle) para uma textura
    /// 4x2: calculei à mão a permutação esperada (índice de destino em
    /// ordem raster -> índice de origem embaralhado) a partir do algoritmo
    /// `GenerateSwizzleMasks`/`FillPattern` do original, e confiro aqui que
    /// o código bate com essa conta.
    #[test]
    fn desembaralhar_reproduz_permutacao_calculada_a_mao() {
        const LARGURA: u32 = 4;
        const ALTURA: u32 = 2;
        // destino em ordem raster (y*4+x) -> índice de origem esperado
        let permutacao_esperada = [0usize, 1, 4, 5, 2, 3, 6, 7];

        // cada "pixel" de origem (1 byte só, para simplificar) contém seu
        // próprio índice, para conseguirmos verificar a permutação direto.
        let origem: Vec<u8> = (0..8u8).collect();

        let destino = desembaralhar(&origem, LARGURA, ALTURA, 1);

        assert_eq!(destino.len(), 8);
        for (indice_destino, &indice_origem_esperado) in permutacao_esperada.iter().enumerate() {
            assert_eq!(
                destino[indice_destino], origem[indice_origem_esperado],
                "pixel destino {indice_destino} deveria vir da origem {indice_origem_esperado}"
            );
        }
    }

    /// Monta um cabeçalho XPR mínimo com formato e expoente de tamanho
    /// escolhidos — o suficiente para `decodificar` chegar ao decodificador.
    fn cabecalho_xpr(formato: u8, expoente: u8) -> Vec<u8> {
        let mut xpr = vec![0u8; 64];
        xpr[0..4].copy_from_slice(&ASSINATURA_XPR0.to_le_bytes());
        xpr[4..8].copy_from_slice(&64u32.to_le_bytes()); // file_size
        xpr[8..12].copy_from_slice(&28u32.to_le_bytes()); // header_size
        xpr[25] = formato;
        xpr[27] = expoente;
        xpr
    }

    /// Regressão: a dimensão da textura vem de `1 << dados[27]`, então uma
    /// textura de 2^31 x 2^31 é trivial de forjar — e `2^31 * 2^31 * 4`
    /// estourava a multiplicação (pânico em debug; em release, um tamanho
    /// embaralhado e acesso fora dos limites logo adiante). Alcançável só
    /// com `iso2god info` numa imagem preparada.
    #[test]
    fn recusa_textura_de_tamanho_impossivel_em_vez_de_estourar() {
        for formato in [FORMATO_ARGB, FORMATO_DXT1] {
            let mensagem = match decodificar(&cabecalho_xpr(formato, 31)) {
                Ok(_) => panic!("uma textura 2^31 x 2^31 não deveria decodificar"),
                Err(e) => e.to_string(),
            };
            assert!(
                mensagem.contains("impossível") || mensagem.contains("insuficientes"),
                "formato {formato}: mensagem pouco clara: {mensagem}"
            );
        }
    }

    #[test]
    fn recusa_expoente_de_tamanho_alem_de_32_bits() {
        assert!(decodificar(&cabecalho_xpr(FORMATO_ARGB, 32)).is_err());
        assert!(decodificar(&cabecalho_xpr(FORMATO_ARGB, 255)).is_err());
    }

    #[test]
    fn xrgb_ignora_o_quarto_byte_e_sai_opaco() {
        // 2x2, B,G,R,X com X = 0: no ARGB seria transparente; no XRGB, opaco
        let pixels = [10u8, 20, 30, 0].repeat(4);
        let t = decodificar(&montar_xpr(FORMATO_XRGB, 1, &pixels)).unwrap();
        assert_eq!((t.largura, t.altura), (2, 2));
        for px in t.rgba.chunks(4) {
            assert_eq!(px, [30, 20, 10, 255]);
        }
        let t = decodificar(&montar_xpr(FORMATO_ARGB, 1, &pixels)).unwrap();
        assert!(t.rgba.chunks(4).all(|px| px == [30, 20, 10, 0]));
    }

    #[test]
    fn rejeita_xpr_sem_assinatura() {
        let mut dados = vec![0u8; 64];
        dados[0..4].copy_from_slice(b"NADA");
        assert!(decodificar(&dados).is_err());
    }

    #[test]
    fn rgba_para_png_produz_png_valido() {
        let rgba = vec![
            255u8, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
        ];
        let png = rgba_para_png(2, 2, &rgba).expect("deveria codificar");
        assert_eq!(
            &png[0..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
    }
}
