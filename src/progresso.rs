//! Relato de progresso da conversão — em dois formatos possíveis:
//!
//! - **Rico** (padrão): a experiência visual de terminal (caixas, gradiente
//!   azul→verde, barra animada), via `crate::terminal`. É o que qualquer
//!   pessoa vê rodando o programa direto no terminal.
//! - **JSON** (`--progresso-json`): uma linha JSON por evento em stdout,
//!   pensada para ser consumida por outro programa (ex: um script) sem
//!   precisar interpretar códigos ANSI.
//!
//! Em nenhum dos dois casos isso influencia a lógica de conversão em si —
//! é só uma questão de *como* o progresso (já calculado a partir de
//! bytes/blocos reais) chega para fora.

use std::io::{IsTerminal, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

use crate::god::hashtable;
use crate::terminal::{BarraProgresso, NEG, SIM_SETA, Tema, c, emo};

/// Um evento de progresso, serializado como uma linha JSON quando
/// `--progresso-json` está ativo. O formato é estável: programas que já
/// consomem esse protocolo continuam funcionando sem nenhuma mudança.
#[derive(Debug, Serialize)]
#[serde(tag = "evento", rename_all = "snake_case")]
pub enum EventoProgresso {
    Fase {
        fase: &'static str,
        mensagem: String,
    },
    Progresso {
        blocos: u64,
        total_blocos: u64,
        bytes: u64,
        total_bytes: u64,
        velocidade_bps: f64,
        eta_segundos: Option<f64>,
    },
    Concluido {
        pasta: String,
        duracao_segundos: f64,
        mensagem: String,
    },
    Erro {
        mensagem: String,
    },
}

fn emitir(evento: &EventoProgresso) {
    if let Ok(linha) = serde_json::to_string(evento) {
        println!("{linha}");
    }
}

/// Relata erros de nível superior (ex: falha logo no início, antes de haver
/// um `Reporter`) no formato certo — JSON em stdout se `json` estiver
/// ativo, ou uma linha rica em stderr caso contrário.
pub fn relatar_erro_final(mensagem: &str, json: bool) {
    if json {
        emitir(&EventoProgresso::Erro {
            mensagem: mensagem.to_string(),
        });
    } else {
        Tema::detectar().erro(mensagem);
    }
}

/// Anuncia uma fase sem precisar de um `Reporter` já construído (usado
/// antes de sabermos quantos blocos serão convertidos, ex: durante a
/// reconstrução da GDF em `--padding completa`).
pub fn anunciar_fase(fase: &'static str, mensagem: &str, json: bool) {
    if json {
        emitir(&EventoProgresso::Fase {
            fase,
            mensagem: mensagem.to_string(),
        });
    } else {
        let tema = Tema::detectar();
        println!(
            "\n{} {}",
            tema_seta(&tema),
            tema.c(mensagem, &[&c::branco(), NEG])
        );
    }
}

fn tema_seta(tema: &Tema) -> String {
    tema.c(SIM_SETA, &[&c::azul(), NEG])
}

/// Relata o progresso da conversão, numa de duas formas (ver módulo).
pub enum Reporter {
    Rico(EstadoRico),
    Json(EstadoJson),
}

pub struct EstadoRico {
    tema: Tema,
    total_blocos: u64,
    total_bytes: u64,
    processados: AtomicU64,
    inicio: Instant,
    ultimo_emitido_ms: AtomicU64,
    barra: BarraProgresso,
    detalhe: Mutex<String>,
}

pub struct EstadoJson {
    total_blocos: u64,
    total_bytes: u64,
    processados: AtomicU64,
    inicio: Instant,
    ultimo_emitido_ms: AtomicU64,
}

/// Intervalo mínimo entre atualizações de progresso (redesenho da barra ou
/// evento JSON), para não gerar uma linha/redesenho por bloco — mais rápido
/// do que qualquer terminal ou consumidor consegue (ou precisa) acompanhar.
const INTERVALO_MINIMO_MS: u64 = 120;

impl Reporter {
    pub fn novo(total_blocos: u32, json: bool) -> Self {
        let total_bytes = total_blocos as u64 * hashtable::TAMANHO_BLOCO as u64;
        if json {
            Reporter::Json(EstadoJson {
                total_blocos: total_blocos as u64,
                total_bytes,
                processados: AtomicU64::new(0),
                inicio: Instant::now(),
                ultimo_emitido_ms: AtomicU64::new(0),
            })
        } else {
            let tema = Tema::detectar();
            let barra = BarraProgresso::nova(tema, "Convertendo", emo::VELOCIDADE(), total_bytes);
            Reporter::Rico(EstadoRico {
                tema,
                total_blocos: total_blocos as u64,
                total_bytes,
                processados: AtomicU64::new(0),
                inicio: Instant::now(),
                ultimo_emitido_ms: AtomicU64::new(0),
                barra,
                detalhe: Mutex::new(String::new()),
            })
        }
    }

    /// Anuncia uma mudança de fase de alto nível (ex: "convertendo",
    /// "calculando_hash") — acontece só algumas vezes por conversão, então
    /// vira uma linha própria (não some dentro da barra de progresso).
    pub fn fase(&self, fase: &'static str, mensagem: &str) {
        match self {
            Reporter::Rico(estado) => {
                println!(
                    "\n{} {}",
                    tema_seta(&estado.tema),
                    estado.tema.c(mensagem, &[&c::branco(), NEG])
                );
            }
            Reporter::Json(_) => {
                emitir(&EventoProgresso::Fase {
                    fase,
                    mensagem: mensagem.to_string(),
                });
            }
        }
    }

    /// Detalhe de alta frequência (ex: "Escrevendo parte 5/39...") — no
    /// modo rico isso só atualiza o texto ao lado da barra de progresso
    /// (senão viraria uma linha nova a cada parte); no modo JSON continua
    /// saindo como evento de fase, exatamente como antes.
    pub fn detalhe_bloco(&self, mensagem: &str) {
        match self {
            Reporter::Rico(estado) => {
                if let Ok(mut d) = estado.detalhe.lock() {
                    *d = mensagem.to_string();
                }
            }
            Reporter::Json(_) => {
                emitir(&EventoProgresso::Fase {
                    fase: "convertendo",
                    mensagem: mensagem.to_string(),
                });
            }
        }
    }

    /// Apaga a linha da barra de progresso, sem escrever resumo nenhum.
    /// Usado quando a conversão termina mal (erro ou cancelamento): a barra
    /// é desenhada com `\r` e sem quebra de linha, então qualquer mensagem
    /// impressa depois sairia colada nos restos dela.
    pub fn interromper_barra(&self) {
        if matches!(self, Reporter::Rico(_)) && std::io::stdout().is_terminal() {
            print!("{}", crate::terminal::limpar_linha());
            let _ = std::io::stdout().flush();
        }
    }

    /// Aviso não-fatal (ex: diretório de saída substituído).
    pub fn aviso(&self, mensagem: &str) {
        match self {
            Reporter::Rico(estado) => estado.tema.aviso(mensagem),
            Reporter::Json(_) => {
                emitir(&EventoProgresso::Fase {
                    fase: "aviso",
                    mensagem: mensagem.to_string(),
                });
            }
        }
    }

    /// Registra que mais `n` blocos foram processados.
    pub fn inc(&self, n: u64) {
        match self {
            Reporter::Rico(estado) => {
                let processados = estado.processados.fetch_add(n, Ordering::Relaxed) + n;
                if !deve_emitir(
                    &estado.ultimo_emitido_ms,
                    &estado.inicio,
                    processados,
                    estado.total_blocos,
                ) {
                    return;
                }
                let (bytes, velocidade, eta) =
                    calcular_metricas(processados, estado.total_bytes, &estado.inicio);
                let detalhe = estado.detalhe.lock().map(|d| d.clone()).unwrap_or_default();
                estado.barra.atualizar(bytes, velocidade, eta, &detalhe);
            }
            Reporter::Json(estado) => {
                let processados = estado.processados.fetch_add(n, Ordering::Relaxed) + n;
                if !deve_emitir(
                    &estado.ultimo_emitido_ms,
                    &estado.inicio,
                    processados,
                    estado.total_blocos,
                ) {
                    return;
                }
                let (bytes, velocidade_bps, eta_segundos) =
                    calcular_metricas(processados, estado.total_bytes, &estado.inicio);
                emitir(&EventoProgresso::Progresso {
                    blocos: processados,
                    total_blocos: estado.total_blocos,
                    bytes,
                    total_bytes: estado.total_bytes,
                    velocidade_bps,
                    eta_segundos,
                });
            }
        }
    }

    /// Marca a etapa de escrita de blocos como concluída.
    pub fn fim_dos_blocos(&self, _mensagem: &str) {
        if let Reporter::Rico(estado) = self {
            let feito =
                estado.processados.load(Ordering::Relaxed) * hashtable::TAMANHO_BLOCO as u64;
            estado.barra.finalizar(feito, true);
        }
    }

    /// Anuncia a conclusão de toda a conversão.
    pub fn concluido(&self, pasta: &str, duracao_segundos: f64, mensagem: &str) {
        match self {
            Reporter::Rico(estado) => estado.tema.sucesso(mensagem),
            Reporter::Json(_) => {
                emitir(&EventoProgresso::Concluido {
                    pasta: pasta.to_string(),
                    duracao_segundos,
                    mensagem: mensagem.to_string(),
                });
            }
        }
    }
}

/// Decide se já passou tempo suficiente (ou se é o último evento) para
/// valer a pena redesenhar/emitir de novo.
fn deve_emitir(ultimo_ms: &AtomicU64, inicio: &Instant, processados: u64, total: u64) -> bool {
    let agora_ms = inicio.elapsed().as_millis() as u64;
    let ultimo = ultimo_ms.load(Ordering::Relaxed);
    let e_o_ultimo = processados >= total;
    if !e_o_ultimo && agora_ms.saturating_sub(ultimo) < INTERVALO_MINIMO_MS {
        return false;
    }
    ultimo_ms.store(agora_ms, Ordering::Relaxed);
    true
}

fn calcular_metricas(
    processados: u64,
    total_bytes: u64,
    inicio: &Instant,
) -> (u64, f64, Option<f64>) {
    let bytes = processados * hashtable::TAMANHO_BLOCO as u64;
    let segundos = inicio.elapsed().as_secs_f64();
    let velocidade_bps = if segundos > 0.0 {
        bytes as f64 / segundos
    } else {
        0.0
    };
    let eta_segundos = if velocidade_bps > 0.0 {
        Some((total_bytes.saturating_sub(bytes)) as f64 / velocidade_bps)
    } else {
        None
    };
    (bytes, velocidade_bps, eta_segundos)
}
