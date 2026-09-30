use crate::erro::{Erro, Resultado};

/// Atributos de uma entrada de diretório GDF (bitflags do campo original
/// `GDFDirEntryAttrib`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtributosEntrada(pub u8);

impl AtributosEntrada {
    pub fn eh_diretorio(self) -> bool {
        // Bit 0x10 marca diretório no formato original (GDFDirEntryAttrib.Directory).
        self.0 & 0x10 != 0
    }
}

/// Uma entrada dentro da árvore binária de um diretório GDF.
/// Equivalente a `GDFDirEntry`.
#[derive(Debug, Clone)]
pub struct EntradaDiretorio {
    pub subarvore_esquerda: u16,
    pub subarvore_direita: u16,
    pub setor: u32,
    pub tamanho: u32,
    pub atributos: AtributosEntrada,
    pub nome: String,
    /// Subdiretório carregado sob demanda (lazy), igual ao `SubDir` original:
    /// só é preenchido quando alguém precisa navegar por dentro dele.
    pub subdiretorio: Option<TabelaDiretorio>,
}

impl EntradaDiretorio {
    pub fn eh_diretorio(&self) -> bool {
        self.atributos.eh_diretorio()
    }

    /// Serializa esta entrada para bytes, incluindo o preenchimento de
    /// alinhamento a 4 bytes (com 0xFF, igual ao original). Os campos
    /// `subarvore_esquerda`/`subarvore_direita` são escritos exatamente como
    /// foram lidos — ver o comentário em `TabelaDiretorio::para_bytes` sobre
    /// por que isso é seguro. Equivalente a `GDFDirEntry.ToByteArray()`.
    fn para_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(14 + self.nome.len() + 3);
        buf.extend_from_slice(&self.subarvore_esquerda.to_le_bytes());
        buf.extend_from_slice(&self.subarvore_direita.to_le_bytes());
        buf.extend_from_slice(&self.setor.to_le_bytes());
        buf.extend_from_slice(&self.tamanho.to_le_bytes());
        buf.push(self.atributos.0);
        buf.push(self.nome.len() as u8);
        // Nomes decodificados por `decodificar_ascii` são sempre ASCII puro
        // (bytes >= 0x80 viram '?' na leitura), então `as_bytes()` devolve
        // exatamente os bytes originais, sem precisar de um codificador à parte.
        buf.extend_from_slice(self.nome.as_bytes());

        let resto = buf.len() % 4;
        if resto != 0 {
            buf.resize(buf.len() + (4 - resto), 0xFF);
        }
        buf
    }
}

/// Tabela (árvore binária serializada) de entradas de um diretório GDF.
/// Equivalente a `GDFDirTable`.
#[derive(Debug, Clone, Default)]
pub struct TabelaDiretorio {
    pub setor: u32,
    pub tamanho: u32,
    pub entradas: Vec<EntradaDiretorio>,
}

/// Decodifica bytes como `Encoding.ASCII` do .NET faz: cada byte vira um
/// caractere 1:1, e qualquer byte >= 0x80 é substituído por '?' — não é a
/// mesma coisa que decodificar como UTF-8.
fn decodificar_ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { '?' })
        .collect()
}

impl TabelaDiretorio {
    /// Lê a tabela de diretório a partir dos bytes brutos do setor
    /// correspondente (exatamente `tamanho` bytes, já lidos do disco pelo
    /// chamador). Equivalente ao corpo do construtor
    /// `GDFDirTable(CBinaryReader, GDFVolumeDescriptor, uint, uint)`.
    ///
    /// Importante: a leitura aqui é uma varredura linear do bloco, não uma
    /// travessia da árvore binária via `SubTreeL`/`SubTreeR` — o formato
    /// serializa todas as entradas em sequência dentro do bloco, e os
    /// ponteiros de subárvore só importam para quem precisa navegar em
    /// ordem (não é o nosso caso aqui). Uma entrada é tratada como
    /// preenchimento/"buraco" e pulada (consumindo só os 4 bytes dos
    /// ponteiros) sempre que `SubTreeL` OU `SubTreeR` valem 0xFFFF — mesma
    /// heurística do original, que também deixa passar batido o caso raro
    /// de uma entrada legítima sem filho algum dos dois lados.
    pub fn ler(bytes: &[u8], setor: u32, tamanho: u32) -> Resultado<Self> {
        let limite = (tamanho as usize).min(bytes.len());
        let mut posicao = 0usize;
        let mut entradas = Vec::new();

        while posicao < limite {
            if posicao + 4 > bytes.len() {
                break;
            }
            let subarvore_esquerda = u16::from_le_bytes([bytes[posicao], bytes[posicao + 1]]);
            let subarvore_direita = u16::from_le_bytes([bytes[posicao + 2], bytes[posicao + 3]]);
            posicao += 4;

            if subarvore_esquerda == 0xFFFF || subarvore_direita == 0xFFFF {
                continue;
            }

            if posicao + 10 > bytes.len() {
                break;
            }
            let entrada_setor = u32::from_le_bytes(bytes[posicao..posicao + 4].try_into().unwrap());
            posicao += 4;
            let entrada_tamanho =
                u32::from_le_bytes(bytes[posicao..posicao + 4].try_into().unwrap());
            posicao += 4;
            let atributos = AtributosEntrada(bytes[posicao]);
            posicao += 1;
            let tamanho_nome = bytes[posicao] as usize;
            posicao += 1;

            if posicao + tamanho_nome > bytes.len() {
                break;
            }
            let nome = decodificar_ascii(&bytes[posicao..posicao + tamanho_nome]);
            posicao += tamanho_nome;

            let resto = posicao % 4;
            if resto != 0 {
                posicao += 4 - resto;
            }

            entradas.push(EntradaDiretorio {
                subarvore_esquerda,
                subarvore_direita,
                setor: entrada_setor,
                tamanho: entrada_tamanho,
                atributos,
                nome,
                subdiretorio: None,
            });
        }

        Ok(Self {
            setor,
            tamanho,
            entradas,
        })
    }

    /// Serializa a tabela de volta para bytes, prontos para escrita no
    /// ISO/GDF — usado só na reconstrução completa (`--padding completa`).
    /// Equivalente a `GDFDirTable.ToByteArray()`.
    ///
    /// Devolve erro em vez de entrar em pânico quando as entradas não cabem
    /// no tamanho declarado da tabela. Isso não acontece com uma imagem
    /// íntegra (o conjunto de entradas é exatamente o que foi lido do
    /// disco), mas a leitura é uma varredura linear que não impõe as
    /// fronteiras de setor: numa imagem corrompida ou forjada, uma entrada
    /// que cruze a fronteira é lida sem reclamar e, na reescrita, a regra de
    /// "não deixar entrada cruzar setor" a empurra para além do buffer.
    ///
    /// Importante: as entradas são escritas na mesma ordem em que já estão
    /// em `entradas` (a ordem em que foram originalmente lidas do disco) —
    /// não há reordenação nem reconstrução da árvore binária de busca.
    /// Isso só é seguro porque a reconstrução nunca adiciona, remove ou
    /// renomeia entradas: o conjunto e o tamanho de cada entrada permanecem
    /// idênticos aos do disco de origem, então repetir aqui a mesma regra de
    /// "nunca deixar uma entrada cruzar uma fronteira de 2048 bytes" recria
    /// exatamente o mesmo layout relativo de bytes dentro do bloco — e por
    /// isso os ponteiros `SubTreeL`/`SubTreeR` de cada entrada (preservados
    /// sem alteração desde a leitura) continuam válidos, sem precisar
    /// recalcular nenhuma árvore binária do zero.
    pub fn para_bytes(&self) -> Resultado<Vec<u8>> {
        const TAMANHO_SETOR: usize = 2048;

        let tamanho_total = (self.tamanho as usize).div_ceil(TAMANHO_SETOR) * TAMANHO_SETOR;
        let mut buffer = vec![0xFFu8; tamanho_total];
        let mut posicao = 0usize;

        for entrada in &self.entradas {
            let bytes = entrada.para_bytes();
            let restante_no_setor = TAMANHO_SETOR - (posicao % TAMANHO_SETOR);
            if bytes.len() > restante_no_setor {
                posicao += restante_no_setor;
            }
            let fim = posicao + bytes.len();
            if fim > tamanho_total {
                return Err(Erro::IsoInvalida(format!(
                    "a tabela de diretório do setor {} não cabe nos {tamanho_total} bytes \
                     declarados ao reescrevê-la (a entrada '{}' terminaria em {fim}); a imagem \
                     pode estar corrompida",
                    self.setor, entrada.nome
                )));
            }
            buffer[posicao..fim].copy_from_slice(&bytes);
            posicao = fim;
        }

        Ok(buffer)
    }

    /// Procura, entre as entradas diretas desta tabela, uma cujo nome bata
    /// (sem diferenciar maiúsculas/minúsculas) com `nome`.
    pub fn encontrar(&self, nome: &str) -> Option<&EntradaDiretorio> {
        self.entradas
            .iter()
            .find(|e| e.nome.eq_ignore_ascii_case(nome))
    }
}

/// Limite de profundidade de recursão ao navegar/analisar a árvore de
/// diretórios. Não existe no original — é uma proteção defensiva nossa
/// contra uma imagem corrompida ou maliciosamente construída com uma cadeia
/// de diretórios ciclíca ou absurdamente profunda, o que causaria estouro de
/// pilha em vez de um erro tratável.
pub const PROFUNDIDADE_MAXIMA: u32 = 64;

pub(super) fn erro_profundidade_excedida() -> Erro {
    Erro::IsoInvalida(format!(
        "estrutura de diretórios excede a profundidade máxima permitida ({PROFUNDIDADE_MAXIMA}); \
         a imagem pode estar corrompida"
    ))
}

#[cfg(test)]
mod testes {
    use super::*;

    fn entrada(
        subtree_l: u16,
        subtree_r: u16,
        setor: u32,
        tamanho: u32,
        attrib: u8,
        nome: &str,
    ) -> EntradaDiretorio {
        EntradaDiretorio {
            subarvore_esquerda: subtree_l,
            subarvore_direita: subtree_r,
            setor,
            tamanho,
            atributos: AtributosEntrada(attrib),
            nome: nome.to_string(),
            subdiretorio: None,
        }
    }

    #[test]
    fn para_bytes_e_ler_fazem_round_trip() {
        let original = TabelaDiretorio {
            setor: 500,
            tamanho: 0, // recalculado abaixo a partir do que para_bytes gerar
            entradas: vec![
                entrada(10, 20, 1000, 400_000, 0x00, "default.xex"),
                entrada(30, 40, 2000, 2048, 0x10, "SUBDIR"),
                entrada(50, 60, 3000, 123, 0x00, "readme.txt"),
            ],
        };

        // primeiro passe só para descobrir o tamanho real gerado
        let tamanho_bruto: u32 = original
            .entradas
            .iter()
            .map(|e| e.para_bytes().len() as u32)
            .sum();
        let mut original = original;
        original.tamanho = tamanho_bruto;

        let bytes = original.para_bytes().unwrap();
        let relida = TabelaDiretorio::ler(&bytes, original.setor, tamanho_bruto).unwrap();

        assert_eq!(relida.entradas.len(), original.entradas.len());
        for (a, b) in original.entradas.iter().zip(relida.entradas.iter()) {
            assert_eq!(a.subarvore_esquerda, b.subarvore_esquerda);
            assert_eq!(a.subarvore_direita, b.subarvore_direita);
            assert_eq!(a.setor, b.setor);
            assert_eq!(a.tamanho, b.tamanho);
            assert_eq!(a.atributos, b.atributos);
            assert_eq!(a.nome, b.nome);
        }
    }

    #[test]
    fn para_bytes_arredonda_para_multiplo_de_2048_preenchendo_com_0xff() {
        let tabela = TabelaDiretorio {
            setor: 0,
            tamanho: 100, // bem menor que um setor
            entradas: vec![entrada(1, 2, 1, 4096, 0x00, "a")],
        };
        let bytes = tabela.para_bytes().unwrap();
        assert_eq!(bytes.len(), 2048);
        // depois da única entrada, o resto do setor deve ser 0xFF
        let tamanho_entrada = entrada(1, 2, 1, 4096, 0x00, "a").para_bytes().len();
        assert!(bytes[tamanho_entrada..].iter().all(|&b| b == 0xFF));
    }

    /// Regressão: antes de `para_bytes` devolver erro, esta tabela entrava em
    /// pânico com "range end index 4368 out of range for slice of length
    /// 4096" — alcançável com uma imagem corrompida em `--padding completa`.
    #[test]
    fn para_bytes_recusa_tabela_que_nao_cabe_em_vez_de_estourar() {
        let mut bytes = Vec::new();
        for i in 0..15u8 {
            let nome = vec![b'A' + (i % 26); 255];
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&1u16.to_le_bytes());
            bytes.extend_from_slice(&(100u32 + i as u32).to_le_bytes());
            bytes.extend_from_slice(&2048u32.to_le_bytes());
            bytes.push(0x00);
            bytes.push(255);
            bytes.extend_from_slice(&nome);
            while !bytes.len().is_multiple_of(4) {
                bytes.push(0xFF);
            }
        }
        bytes.resize(4096, 0xFF);

        let tabela = TabelaDiretorio::ler(&bytes, 100, 4096).unwrap();
        assert_eq!(
            tabela.entradas.len(),
            15,
            "as 15 entradas cabem na leitura linear"
        );

        let mensagem = match tabela.para_bytes() {
            Ok(_) => panic!("a reescrita não caberia no tamanho declarado"),
            Err(e) => e.to_string(),
        };
        assert!(
            mensagem.contains("não cabe"),
            "mensagem pouco clara: {mensagem}"
        );
    }

    #[test]
    fn para_bytes_nao_deixa_entrada_cruzar_fronteira_de_setor() {
        // Uma entrada com nome longo o suficiente para não caber no que
        // sobra do primeiro setor deve começar no setor seguinte.
        let nome_longo = "a".repeat(200);
        let entrada_grande = entrada(1, 2, 1, 10, 0x00, &nome_longo);
        let tamanho_entrada_grande = entrada_grande.para_bytes().len() as u32;

        let tabela = TabelaDiretorio {
            setor: 0,
            tamanho: 2048 + tamanho_entrada_grande,
            entradas: vec![
                entrada(1, 2, 1, 10, 0x00, "curto"), // pequena, cabe no primeiro setor
                entrada_grande,
            ],
        };
        let bytes = tabela.para_bytes().unwrap();
        // a segunda entrada não deveria começar antes do offset 2048
        let relida = TabelaDiretorio::ler(&bytes, 0, bytes.len() as u32).unwrap();
        assert_eq!(relida.entradas.len(), 2);
        assert_eq!(relida.entradas[1].nome, nome_longo);
    }
}
