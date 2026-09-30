//! Inspeção de uma imagem ISO (tipo de disco, plataforma, Title ID, thumbnail
//! etc.), compartilhada entre `iso2god info` e o assistente interativo — os
//! dois mostram exatamente o mesmo resumo, só que um decide ISO/destino via
//! flags e o outro via prompts.

use std::path::Path;

use serde::Serialize;

use crate::erro::Resultado;
use crate::gdf;
use crate::terminal::{LARGURA, Tema, emo, fmt_bytes};
use crate::{xbe, xex};

/// Resumo estruturado de uma ISO, usado tanto para a saída de texto quanto
/// para `info --json`.
#[derive(Debug, Serialize)]
pub struct InfoIso {
    pub tipo_disco: String,
    pub setor_dir_raiz: u32,
    pub tamanho_dir_raiz: u32,
    pub deslocamento_raiz: u64,
    pub tamanho_volume: u64,
    pub setores_volume: u32,
    pub entradas_raiz: usize,
    pub contem_default_xex: bool,
    pub contem_default_xbe: bool,
    pub plataforma_detectada: Option<&'static str>,
    pub title_id: Option<String>,
    pub media_id: Option<String>,
    pub titulo: Option<String>,
    pub disco: Option<u8>,
    pub total_discos: Option<u8>,
    pub thumbnail_png_base64: Option<String>,
    pub ultimo_setor: u32,
}

/// Abre e analisa a ISO em `origem`, detectando plataforma (Xbox Original /
/// Xbox 360) e metadados (Title ID, Media ID, título, thumbnail) a partir do
/// `default.xex`/`default.xbe` embutido, quando presente.
pub fn analisar(origem: &Path) -> Resultado<InfoIso> {
    let mut gdf = gdf::Gdf::abrir(origem)?;

    let contem_xex = gdf.existe("default.xex");
    let contem_xbe = gdf.existe("default.xbe");

    let mut info = InfoIso {
        tipo_disco: format!("{:?}", gdf.tipo),
        setor_dir_raiz: gdf.descritor.setor_dir_raiz,
        tamanho_dir_raiz: gdf.descritor.tamanho_dir_raiz,
        deslocamento_raiz: gdf.descritor.deslocamento_raiz,
        tamanho_volume: gdf.descritor.tamanho_volume,
        setores_volume: gdf.descritor.setores_volume,
        entradas_raiz: gdf.raiz.as_ref().map_or(0, |r| r.entradas.len()),
        contem_default_xex: contem_xex,
        contem_default_xbe: contem_xbe,
        plataforma_detectada: None,
        title_id: None,
        media_id: None,
        titulo: None,
        disco: None,
        total_discos: None,
        thumbnail_png_base64: None,
        ultimo_setor: 0,
    };

    if contem_xex {
        info.plataforma_detectada = Some("xbox360");
        if let Ok(bytes) = gdf.ler_arquivo("default.xex")
            && let Ok(exec) = xex::ler_info_execucao(&bytes)
        {
            info.title_id = Some(exec.title_id_hex());
            info.media_id = Some(exec.media_id_hex());
            info.disco = Some(exec.disco_numero);
            info.total_discos = Some(exec.disco_total);
            if let Ok(t) = xex::ler_titulo(&bytes) {
                info.titulo = t.titulo;
                if let Some(png) = t.icone_png {
                    use base64::Engine;
                    info.thumbnail_png_base64 =
                        Some(base64::engine::general_purpose::STANDARD.encode(&png));
                }
            }
        }
    } else if contem_xbe {
        info.plataforma_detectada = Some("xbox");
        if let Ok(bytes) = gdf.ler_arquivo("default.xbe") {
            if let Ok(cert) = xbe::ler_info_certificado(&bytes) {
                info.title_id = Some(cert.title_id_hex());
                info.media_id = Some(xbe::media_id_substituto(&bytes));
                info.titulo = Some(cert.titulo);
                info.disco = Some(if cert.disco_numero == 0 {
                    1
                } else {
                    cert.disco_numero as u8
                });
                info.total_discos = Some(1);
            }
            if let Ok(png) = xbe::extrair_thumbnail(&bytes) {
                use base64::Engine;
                info.thumbnail_png_base64 =
                    Some(base64::engine::general_purpose::STANDARD.encode(&png));
            }
        }
    }

    info.ultimo_setor = gdf.analisar_diretorios()?;

    Ok(info)
}

impl InfoIso {
    /// Nome de exibição da plataforma detectada ("Xbox Original"/"Xbox 360"),
    /// pronto para mostrar ao usuário.
    pub fn nome_plataforma(&self) -> Option<&'static str> {
        match self.plataforma_detectada {
            Some("xbox360") => Some("Xbox 360"),
            Some("xbox") => Some("Xbox Original"),
            _ => None,
        }
    }

    /// Imprime o resumo em caixas/campos coloridos (ver `crate::terminal`).
    pub fn imprimir(&self, tema: &Tema) {
        println!(
            "{}",
            tema.caixa_titulo("Análise da imagem ISO", emo::ANALISAR(), LARGURA, true)
        );

        println!(
            "{}",
            tema.campo("Tipo de disco", &self.tipo_disco, emo::DISCO(), 20)
        );
        println!(
            "{}",
            tema.campo("Tamanho do volume", &fmt_bytes(self.tamanho_volume), "", 20)
        );
        println!(
            "{}",
            tema.campo(
                "Setores no volume",
                &self.setores_volume.to_string(),
                "",
                20
            )
        );
        println!(
            "{}",
            tema.campo("Entradas na raiz", &self.entradas_raiz.to_string(), "", 20)
        );

        if self.contem_default_xex {
            tema.marca_ok("default.xex", "encontrado");
        } else {
            tema.marca_nd("default.xex", "não encontrado");
        }
        if self.contem_default_xbe {
            tema.marca_ok("default.xbe", "encontrado");
        } else {
            tema.marca_nd("default.xbe", "não encontrado");
        }

        if let Some(nome_plataforma) = self.nome_plataforma() {
            println!(
                "{}",
                tema.etapa(None, &format!("Detectado: {nome_plataforma}"), None)
            );
            if let Some(t) = &self.title_id {
                println!("{}", tema.campo("Title ID", t, emo::JOGO(), 20));
            }
            if let Some(m) = &self.media_id {
                println!("{}", tema.campo("Media ID", m, "", 20));
            }
            if let Some(t) = &self.titulo {
                println!("{}", tema.campo("Título", t, emo::ESTRELA(), 20));
            }
            if let (Some(d), Some(td)) = (self.disco, self.total_discos) {
                println!(
                    "{}",
                    tema.campo("Disco", &format!("{d} / {td}"), emo::DISCO(), 20)
                );
            }
            if let Some(png) = &self.thumbnail_png_base64 {
                println!(
                    "{}",
                    tema.campo(
                        "Thumbnail",
                        &format!("{} (PNG, base64)", fmt_bytes(png.len() as u64)),
                        emo::ESTRELA(),
                        20
                    )
                );
            }
        } else {
            tema.aviso("Plataforma não detectada (nenhum default.xex/default.xbe encontrado)");
        }

        println!(
            "{}",
            tema.campo(
                "Último setor ocupado",
                &self.ultimo_setor.to_string(),
                "",
                20
            )
        );
        // `info` continua mostrando o número (é diagnóstico), mas avisa quando
        // ele não faz sentido para o tamanho da imagem — é o sintoma de uma
        // entrada de diretório corrompida, e a conversão vai recusar.
        if self.ultimo_setor as u64 > self.setores_volume as u64 {
            tema.aviso(&format!(
                "a árvore de diretórios aponta além do fim da imagem ({} setores num volume de \
                 {}): imagem truncada ou corrompida",
                self.ultimo_setor, self.setores_volume
            ));
        }
        println!();
    }
}
