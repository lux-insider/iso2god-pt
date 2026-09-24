use sha1::{Digest, Sha1};

use crate::erro::{Erro, Resultado};

/// Tamanho de um hash SHA1, em bytes.
const TAMANHO_HASH: usize = 20;

/// Tamanho fixo, em bytes, de uma tabela de hash serializada (Sub ou Master).
pub const TAMANHO_TABELA: usize = 4096;

/// Tamanho, em bytes, de um bloco de dados dentro de uma "Part" do GOD —
/// é também o valor usado como "tamanho de setor" do container GOD (não
/// confundir com o setor de 2048 bytes da GDF).
pub const TAMANHO_BLOCO: u32 = 4096;

/// Quantas entradas de 20 bytes cabem fisicamente em uma tabela de 4096
/// bytes (204 * 20 = 4080, sobrando 16 bytes sempre zerados). Este é o limite
/// físico das duas tabelas; o significado de "quantos slots são realmente
/// usados" varia por contexto orquestrado em `god::mod` (203 Sub Hash Tables
/// por Master Hash Table, deixando 1 slot de sobra para o encadeamento entre
/// partes; 204 blocos de dados por Sub Hash Table, ocupando a tabela por
/// completo).
pub const CAPACIDADE_MAXIMA: usize = TAMANHO_TABELA / TAMANHO_HASH;

/// Calcula o SHA1 de um bloco de bytes. Usado tanto para hashear blocos de
/// dados individuais quanto para hashear a representação serializada de uma
/// Sub/Master Hash Table inteira.
pub fn sha1(dados: &[u8]) -> [u8; TAMANHO_HASH] {
    let mut hasher = Sha1::new();
    hasher.update(dados);
    hasher.finalize().into()
}

/// Implementação compartilhada por `SubHashTable` e `MasterHashTable`: uma
/// lista de hashes SHA1 que serializa para um buffer fixo de 4096 bytes
/// (zerado no que sobra) e pode ser lida de volta a partir desse buffer.
#[derive(Debug, Clone, Default)]
struct ListaHashes(Vec<[u8; TAMANHO_HASH]>);

impl ListaHashes {
    fn adicionar(&mut self, hash: [u8; TAMANHO_HASH]) -> Resultado<()> {
        if self.0.len() >= CAPACIDADE_MAXIMA {
            return Err(Erro::LimiteHashExcedido(CAPACIDADE_MAXIMA));
        }
        self.0.push(hash);
        Ok(())
    }

    fn para_bytes(&self) -> [u8; TAMANHO_TABELA] {
        let mut buffer = [0u8; TAMANHO_TABELA];
        for (indice, hash) in self.0.iter().enumerate() {
            let inicio = indice * TAMANHO_HASH;
            buffer[inicio..inicio + TAMANHO_HASH].copy_from_slice(hash);
        }
        buffer
    }

    /// Lê hashes sequencialmente até encontrar a primeira entrada totalmente
    /// zerada (que marca o fim das entradas válidas) — mesma heurística de
    /// `MasterHashtable.ReadMHT` no original.
    fn ler(bytes: &[u8; TAMANHO_TABELA]) -> Self {
        let mut hashes = Vec::new();
        for indice in 0..CAPACIDADE_MAXIMA {
            let inicio = indice * TAMANHO_HASH;
            let fatia = &bytes[inicio..inicio + TAMANHO_HASH];
            if fatia.iter().all(|&b| b == 0) {
                break;
            }
            let mut hash = [0u8; TAMANHO_HASH];
            hash.copy_from_slice(fatia);
            hashes.push(hash);
        }
        Self(hashes)
    }
}

/// Sub Hash Table: contém os hashes SHA1 de até 204 blocos de dados
/// consecutivos. Equivalente a `Chilano.Iso2God.ConStructures.SubHashTable`.
///
/// No original só é escrita, nunca relida do disco (os hashes de blocos
/// individuais nunca precisam ser recalculados a partir de uma SHT já
/// gravada) — por isso não há um método `ler` aqui, só em
/// `MasterHashTable`.
#[derive(Debug, Clone, Default)]
pub struct SubHashTable(ListaHashes);

impl SubHashTable {
    pub fn nova() -> Self {
        Self::default()
    }

    /// Adiciona o hash de mais um bloco de dados. Retorna erro se a tabela
    /// já tiver os 204 slots ocupados.
    pub fn adicionar(&mut self, hash: [u8; TAMANHO_HASH]) -> Resultado<()> {
        self.0.adicionar(hash)
    }

    pub fn len(&self) -> usize {
        self.0.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.0.is_empty()
    }

    /// Serializa a tabela para os 4096 bytes gravados no disco (com o
    /// restante do buffer zerado).
    pub fn para_bytes(&self) -> [u8; TAMANHO_TABELA] {
        self.0.para_bytes()
    }
}

/// Master Hash Table: contém os hashes SHA1 de até 203 Sub Hash Tables (mais,
/// potencialmente, um 204º hash usado para encadear com a próxima Master
/// Hash Table — ver `god::mod`). Equivalente a
/// `Chilano.Iso2God.ConStructures.MasterHashtable`.
#[derive(Debug, Clone, Default)]
pub struct MasterHashTable(ListaHashes);

impl MasterHashTable {
    pub fn nova() -> Self {
        Self::default()
    }

    pub fn adicionar(&mut self, hash: [u8; TAMANHO_HASH]) -> Resultado<()> {
        self.0.adicionar(hash)
    }

    pub fn len(&self) -> usize {
        self.0.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.0.is_empty()
    }

    pub fn para_bytes(&self) -> [u8; TAMANHO_TABELA] {
        self.0.para_bytes()
    }

    /// Lê uma Master Hash Table já existente a partir dos primeiros 4096
    /// bytes de uma "Part". Equivalente a `MasterHashtable.ReadMHT`.
    pub fn ler(bytes: &[u8; TAMANHO_TABELA]) -> Self {
        Self(ListaHashes::ler(bytes))
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn sha1_bate_com_vetor_de_teste_conhecido() {
        // SHA1("abc") é um vetor de teste padrão (FIPS 180-1).
        let hash = sha1(b"abc");
        assert_eq!(
            hash,
            [
                0xA9, 0x99, 0x3E, 0x36, 0x47, 0x06, 0x81, 0x6A, 0xBA, 0x3E, 0x25, 0x71, 0x78,
                0x50, 0xC2, 0x6C, 0x9C, 0xD0, 0xD8, 0x9D,
            ]
        );
    }

    #[test]
    fn round_trip_para_bytes_e_ler() {
        let mut original = MasterHashTable::nova();
        // Começa em 1: um hash `[0; 20]` seria indistinguível do marcador de
        // "fim de tabela" usado por `ler` (mesma limitação do original).
        for i in 1u8..=5 {
            original.adicionar([i; TAMANHO_HASH]).unwrap();
        }
        let bytes = original.para_bytes();
        let lida = MasterHashTable::ler(&bytes);
        assert_eq!(lida.len(), 5);
        assert_eq!(lida.0.0, original.0.0);
    }

    #[test]
    fn ler_para_no_primeiro_hash_totalmente_zerado() {
        let mut tabela = MasterHashTable::nova();
        tabela.adicionar([1; TAMANHO_HASH]).unwrap();
        tabela.adicionar([0; TAMANHO_HASH]).unwrap(); // marca "fim"
        tabela.adicionar([2; TAMANHO_HASH]).unwrap(); // nunca deveria ser lido de volta

        let bytes = tabela.para_bytes();
        let lida = MasterHashTable::ler(&bytes);
        assert_eq!(lida.len(), 1);
    }

    #[test]
    fn adicionar_falha_apos_capacidade_maxima() {
        let mut tabela = SubHashTable::nova();
        for i in 0..CAPACIDADE_MAXIMA {
            tabela.adicionar([i as u8; TAMANHO_HASH]).unwrap();
        }
        assert!(tabela.adicionar([0; TAMANHO_HASH]).is_err());
    }
}
