//! Teste de integração de ponta a ponta: converte uma ISO sintética minúscula
//! em um container GOD de verdade e verifica a estrutura resultante no disco
//! — cabeçalho LIVE, layout de diretórios e conteúdo dos blocos de dados.

use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

use iso2god::cli::{Plataforma, RemocaoPadding};
use iso2god::god::{self, OpcoesConversao};

const SETOR: u64 = 2048;
const BASE_ASSINATURA: u64 = 32 * SETOR;
const DESLOCAMENTO_XGD3: u64 = 34_078_720;
const ASSINATURA: &[u8] = b"MICROSOFT*XBOX*MEDIA";
const TAMANHO_BLOCO: usize = 4096;

/// Cria uma ISO sintética do tipo XGD3 cujo conteúdo (a partir do
/// deslocamento raiz) é exatamente `conteudo` — mas com a assinatura e o
/// descritor de volume sobrescritos no meio dele, na posição absoluta real
/// onde `Gdf::abrir` sempre procura por eles (deslocamento raiz + setor 32).
/// Isso garante que "quantos bytes converter" (modo `Nenhuma`, que usa o
/// tamanho do volume inteiro) seja exatamente `conteudo.len()`, sem nenhuma
/// folga extra — necessário para poder comparar os blocos de dados gerados
/// byte a byte contra este mesmo buffer.
///
/// O diretório raiz aponta para um setor inválido de propósito — não é
/// usado neste teste (a conversão sem remoção de padding não depende da
/// árvore de diretórios), e uma falha ao lê-lo é tolerada silenciosamente
/// por `Gdf::abrir`.
fn criar_iso_com_conteudo(nome_arquivo: &str, conteudo: &mut [u8]) -> PathBuf {
    let mut descritor = Vec::new();
    descritor.extend_from_slice(ASSINATURA);
    descritor.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // setor raiz inválido de propósito
    descritor.extend_from_slice(&0u32.to_le_bytes()); // tamanho do dir. raiz
    descritor.extend_from_slice(b"TESTDATA");

    let inicio = BASE_ASSINATURA as usize;
    assert!(
        conteudo.len() >= inicio + descritor.len(),
        "conteúdo de teste pequeno demais para acomodar o descritor de volume"
    );
    conteudo[inicio..inicio + descritor.len()].copy_from_slice(&descritor);

    let caminho = std::env::temp_dir().join(nome_arquivo);
    let mut f = File::create(&caminho).expect("criar arquivo de teste");
    f.set_len(DESLOCAMENTO_XGD3 + conteudo.len() as u64).unwrap();
    f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3)).unwrap();
    f.write_all(conteudo).unwrap();

    caminho
}

fn opcoes_padrao(origem: PathBuf, destino: PathBuf) -> OpcoesConversao {
    OpcoesConversao {
        origem,
        destino,
        padding: RemocaoPadding::Nenhuma,
        numero_disco: false,
        plataforma: Some(Plataforma::Xbox360),
        title_id: Some("4D5308BF".to_string()),
        media_id: Some("AABBCCDD".to_string()),
        titulo: Some("Jogo de Teste".to_string()),
        disco: Some(1),
        total_discos: Some(1),
        plataforma_byte: Some(2),
        tipo_executavel_byte: Some(0),
        icone: None,
        threads: 4,
        progresso_json: false,
    }
}

#[test]
fn converte_iso_pequena_e_gera_god_valido() {
    // 70000 bytes -> maior que o deslocamento onde o descritor de volume é
    // embutido (65536), e ceil(70000/4096) = 18 blocos, o último parcial.
    let tamanho_conteudo = 70_000usize;
    let mut conteudo: Vec<u8> = (0..tamanho_conteudo).map(|i| (i % 251) as u8).collect();

    let origem = criar_iso_com_conteudo("iso2god_teste_conversao_origem.iso", &mut conteudo);
    let destino = std::env::temp_dir().join("iso2god_teste_conversao_destino");
    fs::remove_dir_all(&destino).ok();

    let opcoes = opcoes_padrao(origem.clone(), destino.clone());
    god::converter(&opcoes).expect("conversão deveria ter sucesso");

    // <destino>/<TitleID>/<tipo de conteúdo>, o layout que o console procura
    let pasta_conteudo = destino.join("4D5308BF").join("00007000");
    assert!(pasta_conteudo.is_dir(), "pasta de tipo de conteúdo deveria existir");

    let mut entradas: Vec<_> = fs::read_dir(&pasta_conteudo)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entradas.sort();
    // Deve haver exatamente dois itens: o arquivo de cabeçalho e a pasta ".data".
    assert_eq!(entradas.len(), 2, "esperado cabeçalho + pasta .data, achou: {entradas:?}");

    let nome_pasta_dados = entradas
        .iter()
        .find(|n| n.ends_with(".data"))
        .expect("deveria haver uma pasta .data");
    let nome_base = nome_pasta_dados.strip_suffix(".data").unwrap();
    assert!(
        entradas.contains(&nome_base.to_string()),
        "deveria haver um arquivo de cabeçalho com o mesmo nome base da pasta .data"
    );

    // --- Cabeçalho LIVE ---
    let caminho_cabecalho = pasta_conteudo.join(nome_base);
    let cabecalho = fs::read(&caminho_cabecalho).unwrap();
    assert_eq!(cabecalho.len(), 45_056);
    assert_eq!(&cabecalho[0..4], b"LIVE");

    // Tipo de conteúdo (offset 836, 4 bytes big-endian) = GamesOnDemand (0x7000).
    assert_eq!(&cabecalho[836..840], &0x7000u32.to_be_bytes());

    // Title ID / Media ID (offsets 864 e 852).
    assert_eq!(&cabecalho[864..868], &[0x4D, 0x53, 0x08, 0xBF]);
    assert_eq!(&cabecalho[852..856], &[0xAA, 0xBB, 0xCC, 0xDD]);

    let blocos_necessarios = tamanho_conteudo.div_ceil(TAMANHO_BLOCO);
    assert_eq!(blocos_necessarios, 18);

    // Contagem de blocos alocados (offset 914, 24 bits big-endian).
    let blocos_bytes = (blocos_necessarios as u32).to_be_bytes();
    assert_eq!(&cabecalho[914..917], &blocos_bytes[1..4]);

    // Número de partes (offset 928, 4 bytes little-endian) = 1.
    assert_eq!(&cabecalho[928..932], &1u32.to_le_bytes());

    // --- Data0000 ---
    let caminho_parte = pasta_conteudo.join(nome_pasta_dados).join("Data0000");
    let parte = fs::read(&caminho_parte).unwrap();

    let tamanho_esperado = TAMANHO_BLOCO * 2 + blocos_necessarios * TAMANHO_BLOCO; // MHT + SHT0 + blocos
    assert_eq!(parte.len(), tamanho_esperado);

    let inicio_dados = TAMANHO_BLOCO * 2;
    let dados = &parte[inicio_dados..];
    assert_eq!(&dados[0..tamanho_conteudo], conteudo.as_slice());
    assert!(
        dados[tamanho_conteudo..blocos_necessarios * TAMANHO_BLOCO]
            .iter()
            .all(|&b| b == 0),
        "preenchimento do último bloco deveria ser zero"
    );

    // Hash da MHT (offset 893 do cabeçalho, 20 bytes) deve bater com o SHA1
    // dos primeiros 4096 bytes do Data0000 (a própria MHT da única parte).
    let hash_mht_esperado = {
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        h.update(&parte[0..TAMANHO_BLOCO]);
        let resultado: [u8; 20] = h.finalize().into();
        resultado
    };
    assert_eq!(&cabecalho[893..913], hash_mht_esperado.as_slice());

    fs::remove_file(&origem).ok();
    fs::remove_dir_all(&destino).ok();
}

/// Regressão: uma imagem em que a análise não encontra setor ocupado nenhum
/// (raiz ilegível, imagem truncada) fazia a conversão calcular zero blocos e
/// então subtrair 1 de um `u32` zerado — pânico em debug, e em release um
/// arquivo de terabytes que morria com `File too large`. Agora tem que ser
/// um erro explicado, sem deixar nada para trás no destino.
#[test]
fn conversao_sem_nenhum_setor_ocupado_erra_em_vez_de_estourar() {
    let mut conteudo = vec![0u8; (BASE_ASSINATURA as usize) + 4096];
    let origem = criar_iso_com_conteudo("iso2god_teste_zero_blocos.iso", &mut conteudo);
    let destino = std::env::temp_dir().join("iso2god_teste_zero_blocos_saida");
    fs::remove_dir_all(&destino).ok();

    let mut opcoes = opcoes_padrao(origem.clone(), destino.clone());
    // Parcial usa o "último setor ocupado" da árvore de diretórios, que nesta
    // imagem não pôde ser lida — daí o zero.
    opcoes.padding = RemocaoPadding::Parcial;

    let mensagem = match god::converter(&opcoes) {
        Ok(()) => panic!("uma imagem sem setores ocupados não deveria converter"),
        Err(e) => e.to_string(),
    };
    assert!(
        mensagem.contains("não há nada para converter"),
        "erro pouco explicativo para imagem vazia: {mensagem}"
    );

    let sobrou: Vec<_> = fs::read_dir(&destino)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        sobrou.is_empty(),
        "a falha deixou saída pela metade no destino: {sobrou:?}"
    );

    fs::remove_file(&origem).ok();
    fs::remove_dir_all(&destino).ok();
}

/// Regressão: quando a conversão falha no meio, a pasta `.data` já criada
/// não pode ficar para trás — do lado de fora ela é indistinguível de um
/// pacote pronto. Aqui a falha é forçada apagando a ISO de origem depois de
/// a análise já ter acontecido.
#[test]
fn falha_no_meio_da_conversao_nao_deixa_pacote_pela_metade() {
    let mut conteudo = vec![7u8; (BASE_ASSINATURA as usize) + 64 * 1024];
    let origem = criar_iso_com_conteudo("iso2god_teste_falha_meio.iso", &mut conteudo);
    let destino = std::env::temp_dir().join("iso2god_teste_falha_meio_saida");
    fs::remove_dir_all(&destino).ok();

    let opcoes = opcoes_padrao(origem.clone(), destino.clone());

    // A conversão abre a origem de novo por thread, a cada parte: sem o
    // arquivo, isso falha depois de a pasta de saída já existir.
    fs::remove_file(&origem).unwrap();

    assert!(god::converter(&opcoes).is_err(), "sem a ISO de origem a conversão tem que falhar");

    // A limpeza também remove os diretórios <TitleID>/<tipo> que ela mesma
    // criou e que ficaram vazios — o destino tem que voltar como estava.
    let sobrou: Vec<_> = fs::read_dir(&destino)
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    assert!(
        sobrou.is_empty(),
        "a falha deixou diretórios para trás no destino: {sobrou:?}"
    );

    fs::remove_dir_all(&destino).ok();
}

/// Regressão: uma única entrada de diretório com tamanho absurdo fazia a
/// conversão achar que a imagem ia até o setor ~16 milhões e gravar **32 GB**
/// de zeros a partir de uma imagem de 2 MiB (74 s de trabalho para produzir
/// lixo, ou um disco cheio em máquina menor). Agora a árvore é conferida
/// contra o tamanho real do volume antes de qualquer escrita.
#[test]
fn entrada_de_diretorio_alem_do_fim_da_imagem_nao_gera_pacote_gigante() {
    const SETOR_RAIZ: u32 = 100;

    // uma entrada de arquivo declarando 2 GiB, num volume de poucos KiB
    let mut entrada = Vec::new();
    entrada.extend_from_slice(&0u16.to_le_bytes());
    entrada.extend_from_slice(&1u16.to_le_bytes());
    entrada.extend_from_slice(&200u32.to_le_bytes()); // setor
    entrada.extend_from_slice(&(2u32 << 30).to_le_bytes()); // tamanho absurdo
    entrada.push(0x00);
    entrada.push(b"grande.bin".len() as u8);
    entrada.extend_from_slice(b"grande.bin");
    while !entrada.len().is_multiple_of(4) {
        entrada.push(0xFF);
    }
    let mut bloco_raiz = entrada;
    while !(bloco_raiz.len() as u64).is_multiple_of(SETOR) {
        bloco_raiz.push(0xFF);
    }

    let mut descritor = Vec::new();
    descritor.extend_from_slice(ASSINATURA);
    descritor.extend_from_slice(&SETOR_RAIZ.to_le_bytes());
    descritor.extend_from_slice(&(bloco_raiz.len() as u32).to_le_bytes());
    descritor.extend_from_slice(b"TESTDATA");

    let origem = std::env::temp_dir().join("iso2god_teste_setor_absurdo.iso");
    let mut f = File::create(&origem).unwrap();
    f.set_len(DESLOCAMENTO_XGD3 + (SETOR_RAIZ as u64 + 8) * SETOR).unwrap();
    f.seek(SeekFrom::Start(BASE_ASSINATURA + DESLOCAMENTO_XGD3)).unwrap();
    f.write_all(&descritor).unwrap();
    f.seek(SeekFrom::Start(DESLOCAMENTO_XGD3 + SETOR_RAIZ as u64 * SETOR)).unwrap();
    f.write_all(&bloco_raiz).unwrap();
    drop(f);

    let destino = std::env::temp_dir().join("iso2god_teste_setor_absurdo_saida");
    fs::remove_dir_all(&destino).ok();

    let mut opcoes = opcoes_padrao(origem.clone(), destino.clone());
    opcoes.padding = RemocaoPadding::Parcial;
    opcoes.plataforma = Some(Plataforma::Xbox);

    let mensagem = match god::converter(&opcoes) {
        Ok(()) => panic!("não deveria converter uma árvore que aponta além da imagem"),
        Err(e) => e.to_string(),
    };
    assert!(
        mensagem.contains("além do fim da imagem"),
        "erro pouco explicativo: {mensagem}"
    );
    assert!(!destino.exists() || fs::read_dir(&destino).unwrap().next().is_none());

    fs::remove_file(&origem).ok();
    fs::remove_dir_all(&destino).ok();
}
