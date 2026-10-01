# Auditoria do iso2god-pt 0.1.4

Escopo: todo o código em `src/` e o `Cargo.toml` da versão 0.1.4 (commit
`b882089`). Os números de linha citados são dessa versão.

**Regra que manda em tudo:** a ferramenta já foi validada em jogos reais no
Xbox 360. Nenhuma correção pode mudar os bytes gravados hoje para entradas
válidas: as partes `DataNNNN`, o cabeçalho LIVE, as tabelas de hash, o nome,
o ícone e o número do disco. Por isso, antes de qualquer correção,
`tests/saida_golden.rs` passou a guardar o tamanho e o SHA-1 de cada arquivo
que a 0.1.4 grava em seis cenários com ISOs sintéticas:

- Xbox 360 (XGD3) com `default.xex` cifrado em AES e **compressão básica**,
  XDBF com nome acentuado ("Jogo de Ação") e ícone, um arquivo com nome
  Latin-1 ("Canção.ogg"), subpasta e pasta vazia — com `--padding parcial`,
  `nenhuma` e `completa`;
- Xbox 360 (XGD2) com `default.xex` em **LZX**, disco 2 de 2, nome em japonês
  como idioma padrão, `--numero-disco` e `-j 4`;
- Xbox original (Xsf) com `default.xbe` (título UTF-16 com acento) e
  miniatura XPR DXT1 na seção `$$XTIMAGE`;
- opções manuais (`--titulo` com acento e travessão, `--icone`, `--disco`,
  `--total-discos`, bytes de plataforma e de tipo de executável);
- duas partes (mais de 41.412 blocos), com a cadeia de hash entre as Master
  Hash Tables, comparando `-j 1` com `-j 4`;
- o programa de verdade: a sequência de eventos e os campos do
  `--progresso-json`, o `info --json` e os códigos de saída 0, 1 e 2.

Cada teste também confere no cabeçalho o título (as duas cópias), o ícone e o
número do disco, para garantir que o golden cobre a leitura do XEX/XBE e não
um caminho de reserva. Uma correção que mudaria esses bytes não foi aplicada:
ficou só descrita (marcada **só descrito**), com o motivo.

## Gravidade

| Nível | Critério |
|---|---|
| **Crítica** | grava, sobrescreve ou apaga arquivo fora do que o usuário pediu |
| **Alta** | trava, esgota a memória ou aborta com uma entrada pequena; deixa lixo de centenas de MB num caso comum; deixa um pacote com cara de pronto que está corrompido |
| **Média** | erro que não explica o que falhou; comportamento errado em caso raro; ganho de desempenho claro |
| **Baixa** | robustez em caso artificial, ganho pequeno, cosmético |

Nenhum achado é crítico: o destino de toda gravação é montado só com o
Title ID (hexadecimal conferido), o tipo de conteúdo e o SHA-1 do nome
único — nada que venha da imagem vira caminho. O que chega mais perto é o
S-4 (link simbólico plantado no destino).

## Resumo

| # | Gravidade | Onde | Problema | Decisão |
|---|---|---|---|---|
| B-1 | Alta | `gdf/mod.rs:270-327`, `gdf/diretorio.rs:95-152` | tabelas de diretório compartilhadas: 9 MB de imagem viram mais de 4 GB de memória e o processo aborta | a corrigir |
| B-2 | Alta | `xbe/mod.rs:161-196, 202-206` | busca da seção da miniatura quadrática: XBE de 8 MB trava por 239 s | a corrigir |
| S-1 | Alta | `progresso.rs:52-56` e 111 `println!`/`eprintln!` | stdout ou stderr fechado vira pânico e aborto, sem limpeza | a corrigir |
| S-2 | Alta | `sistema.rs:42-108` | SIGHUP (terminal fechado) e fechar a janela no Windows matam sem limpar | a corrigir |
| S-3 | Alta | `god/mod.rs:422-430, 1042` | ao converter de novo, o cabeçalho antigo fica ao lado do `.data` novo pela metade | a corrigir |
| B-3 | Média | `xex/xdbf.rs:65-75` | XDBF de 88 bytes declara 4 bilhões de entradas: 15 s de laço | a corrigir |
| B-4 | Média | `gdf/mod.rs:153-190` | `default.xex`/`.xbe` lido inteiro: entrada corrompida pede até 4 GiB | a corrigir |
| B-5 | Média | `gdf/mod.rs:270-327` | Ctrl+C e SIGTERM ignorados durante a leitura da árvore | a corrigir |
| B-6 | Média | `god/cabecalho.rs:307-320`, `god/mod.rs:403` | `--title-id` com acento: pânico; `+1+2+3+4` aceito e vira nome de pasta | a corrigir |
| B-7 | Média | `god/mod.rs:176-192` | `--icone` lido inteiro antes de conferir o tamanho (`/dev/zero`, a ISO por engano) | a corrigir |
| E-1 | Média | `erro.rs:6-7` e todo `?` em E/S | erro de E/S sem arquivo nem operação, em inglês | a corrigir |
| E-2 | Média | `main.rs`, `Cargo.toml` (`panic = "abort"`) | pânico sai em inglês, sem evento `erro` e sem apagar a saída | a corrigir |
| S-4 | Média | `god/cabecalho.rs:299`, `god/reconstrucao.rs:57`, `assistente.rs:1064-1068` | link simbólico no destino redireciona a gravação | a corrigir |
| P-1 | Média | `Cargo.toml` | `opt-level = "z"` deixa o SHA-1 por software 44% mais lento | a corrigir |
| P-2 | Média | `god/reconstrucao.rs:253-286` | `--padding completa` copia 2 KiB por chamada: 1,05 milhão de chamadas por GiB | a corrigir |
| B-8 | Baixa | `xex/recursos.rs:232-243` | tabela de recursos declarando 4 GiB: 268 milhões de voltas | a corrigir |
| B-9 | Baixa | `xbe/xpr.rs:46-60` | miniatura XPR declarando 32768×32768 aloca 4 GiB por nada | a corrigir |
| T-1 | Baixa | `analise.rs:120-218`, `gdf/mod.rs:314-317`, `assistente.rs` | textos da imagem (nome do jogo, nomes de arquivo) vão crus ao terminal | a corrigir |
| P-3 | Baixa | `god/mod.rs:771-828` | uma thread que falha não para as outras | a corrigir |
| P-4 | Baixa | `god/mod.rs:846, 873-876, 905-907` | uma mensagem de canal por bloco de 4 KiB; SHT serializada duas vezes | a corrigir |
| L-1 | Baixa | `Cargo.toml`, `terminal.rs:299-303` | `terminal_size` puxa `rustix` e `linux-raw-sys` para ler a largura | a corrigir |
| L-2 | Baixa | `Cargo.toml`, `analise.rs:79-81, 100-102` | `base64` inteiro para codificar um PNG | a corrigir |
| N-1 | Alta | `gdf/diretorio.rs:51, 73-78` | `--padding completa` grava nomes não-ASCII como `?` | **só descrito** (muda bytes) |
| N-2 | Média | `god/reconstrucao.rs:203-210, 229-237` | `--padding completa` casa entradas pelo nome: nomes repetidos copiam o arquivo errado | **só descrito** (muda bytes) |
| N-3 | Baixa | `god/mod.rs:403`, `1052-1061` | `--title-id` em minúsculas: pasta em maiúsculas, nome único calculado com minúsculas | **só descrito** (muda bytes) |
| N-4 | Baixa | `gdf/diretorio.rs:95-152` | varredura linear conta entradas de lixo no fim da tabela | **só descrito** (pode mudar o corte) |
| N-5 | Baixa | `xex/recursos.rs:139` | limite da compressão básica é o maior entre o declarado e 256 MiB | **só descrito** (pode recusar XEX aceito hoje) |
| S-5 | Baixa | `god/mod.rs:602-616` | ISO reconstruída esquecida depois de SIGKILL ou queda de energia | **só descrito** |
| D-1 | Baixa | `sistema.rs:49-51` | segundo Ctrl+C sai na hora e deixa a saída pela metade | **só descrito** (proposital) |
| C-1 | Baixa | `cli.rs`, `god/mod.rs:685-694` | `-j 1` é o padrão; `-j N` lê N regiões ao mesmo tempo | **só descrito** |
| L-3 | Baixa | `Cargo.toml` | `clap` é a maior dependência; `thiserror 1` traz um segundo `syn` | **só descrito** |

Os itens estão detalhados abaixo com o cenário e a correção proposta.

---

## 1. Bugs ocultos com entradas corrompidas, truncadas ou malformadas

O que já estava bem protegido e foi conferido: limites de toda leitura do
XEX, do XDBF e do XBE (`get` com `checked_add`), a profundidade da árvore
(`PROFUNDIDADE_MAXIMA = 64`), entradas que apontam além do volume
(`validar_ultimo_setor`), estouro em `setor + setores` (saturado), tamanho de
textura XPR (`checked_mul`), janela LZX inválida e imagem LZX maior que a
declarada. O `lzxd` foi submetido a 260 mil entradas aleatórias e mutadas a
partir de frames válidos: nenhum pânico, nenhum laço sem fim, saída sempre
limitada ao tamanho pedido. A descompressão inteira é limitada ao tamanho
de imagem declarado, que por sua vez é limitado a 256 MiB.

### B-1 (Alta) — tabelas de diretório compartilhadas: memória explode

`gdf/mod.rs:270-327` (`processar_diretorio`) e `gdf/diretorio.rs:95-152`.
O cache de subdiretórios é por entrada, não por tabela: duas entradas que
apontam para a mesma tabela a carregam duas vezes, com tudo que há abaixo
dela. A leitura é uma varredura linear do bloco, e uma região zerada vira
uma entrada a cada 16 bytes; com o atributo de diretório, cada uma delas
carrega de novo a tabela seguinte. O limite de profundidade (64) não ajuda:
o número de cópias multiplica a cada nível.

*Cenário (reproduzido):* uma imagem de 9 MB com tabelas em cadeia, cada uma
com várias entradas de diretório apontando para a próxima, faz `info`,
`converter` e o assistente passarem de 4 GB de memória em 22 s; o processo
aborta (`memory allocation failed`), sem mensagem em português.

*Correção, sem mudar o resultado de nenhuma imagem:* um orçamento por
imagem, guardado no `Gdf`: no máximo 1 milhão de entradas carregadas e
1 GiB de tabelas lidas, e uma tabela sozinha de no máximo 16 MiB (uma
tabela real tem poucos KB). Estourar o orçamento é erro da imagem inteira,
não um aviso que pula a subpasta. Um disco real tem alguns milhares de
entradas e lê poucos MB de tabelas, então nada muda para ele.

### B-2 (Alta) — XBE: busca da miniatura quadrática

`xbe/mod.rs:161-196` e `202-206`. Para cada um dos dois nomes procurados,
para cada seção declarada (até 4 bilhões, limitado só pelo fim do arquivo),
`ler_string_ascii_ate_nul` percorre o arquivo do endereço do nome até o
primeiro zero, montando uma `String` de tudo que leu.

*Cenário (reproduzido):* um `default.xbe` de 8 MB com 75 mil seções cujo
nome aponta para uma cauda sem zero trava a conversão (e o `info`, e o
assistente) por **239 s** em release, sem resposta ao Ctrl+C.

*Correção:* comparar só os `len + 1` bytes necessários (`$$XSIMAGE\0`). É a
mesma decisão de antes (só um nome igual, seguido de zero, casa), em tempo
linear.

### B-3 (Média) — XDBF: laço de 4 bilhões de voltas

`xex/xdbf.rs:65-75`. `entrada` percorre `0..n`, com `n` vindo do arquivo
(até 2³²), e cada volta só devolve `None` quando a posição já passou do fim.

*Cenário (reproduzido):* um XDBF de 88 bytes declarando `0xFFFFFFFF`
entradas leva 15 s em release a cada leitura (uma na conversão, outra no
`info`, as duas no assistente).

*Correção:* limitar `n` às entradas que cabem nos bytes (`(len - 24) / 18`).
Além disso nenhuma volta acharia nada, então o resultado é o mesmo.

### B-4 (Média) — executável lido inteiro, de qualquer tamanho

`gdf/mod.rs:153-190`. `ler_arquivo` só confere que o arquivo cabe no
volume. Numa imagem de 7 GB, uma entrada `default.xex` corrompida com
tamanho `0xFFFFFFFF` pede 4 GiB de uma vez; sob limite de memória
(contêiner, VM) o Rust aborta o processo.

*Correção:* recusar, para a detecção, executável maior que 512 MiB. Um XEX
precisa caber nos 512 MB de memória do Xbox 360 (e um XBE nos 64 MB do
Xbox), então nenhum jogo real passa disso; a detecção cai no aviso de
sempre ("não foi possível ler o default.xex").

### B-5 (Média) — cancelamento ignorado durante a leitura da árvore

`gdf/mod.rs:270-327`. A leitura da árvore não consulta
`sistema::cancelado()`. Numa imagem como a do B-1 o primeiro Ctrl+C e o
SIGTERM não fazem nada. *Correção:* consultar o cancelamento a cada tabela
lida.

### B-6 (Média) — `--title-id` e `--media-id` mal validados

`god/cabecalho.rs:307-320`. `hex_para_bytes` fatia a string por índice de
byte (`&hex[i..i + 2]`): com um caractere de mais de um byte, a fatia cai no
meio dele e o programa entra em pânico (`--title-id '€1'`, reproduzido). E
`u8::from_str_radix` aceita sinal: `--title-id +1+2+3+4` passa pela
validação, grava os bytes `01 02 03 04` no cabeçalho e cria a pasta
`+1+2+3+4` (`god/mod.rs:403`), que o console não reconhece.

*Correção:* aceitar só dígitos hexadecimais ASCII. Um ID válido continua
gravando exatamente os mesmos bytes e a mesma pasta.

### B-7 (Média) — `--icone` lido inteiro antes de conferir o tamanho

`god/mod.rs:176-192`. `fs::read(caminho)` vem antes da checagem de
`MAX_ICONE` (16 KiB). `--icone /dev/zero` nunca termina e esgota a memória
(reproduzido); passar a ISO por engano carrega gigabytes. *Correção:* ler no
máximo `MAX_ICONE + 1` bytes e recusar se passar, com a mesma mensagem de
hoje.

### B-8 (Baixa) — tabela de recursos do XEX sem limite

`xex/recursos.rs:232-243`. O laço vai até `tam_tabela / 16`, com
`tam_tabela` vindo do arquivo: 268 milhões de voltas (0,3 s em release,
dezenas de segundos em debug) para uma tabela declarando 4 GiB num XEX de
1 KB. *Correção:* limitar às entradas que cabem no arquivo; o resultado é o
mesmo.

### B-9 (Baixa) — miniatura XPR declarando dimensões absurdas

`xbe/xpr.rs:46-60`. O lado da textura é `1 << dados[27]`. Os dados precisam
existir (DXT1 exige `lado² / 2` bytes), mas a saída é 8 vezes maior que a
entrada: um XBE de 512 MiB pede 4 GiB de RGBA, e o PNG resultante ainda
passa pela compressão inteira para ser descartado em seguida por não caber
nos 16 KiB do cabeçalho.

*Correção:* recusar lado acima de 2048. Um PNG RGBA de lado 4096 tem pelo
menos `4096 × (1 + 4 × 4096) / 1032 ≈ 65 KB` (1032:1 é a maior compressão
possível do deflate), quatro vezes o campo do cabeçalho: hoje essa
miniatura já é descartada com aviso na conversão, e continua sendo. Nenhum
byte do pacote muda; o `info --json` deixa de trazer uma miniatura desse
tamanho (nenhum jogo real tem: as do Xbox são de 64×64 a 256×256).

## 2. Segurança de arquivos

Conferido e sem problema: nenhum nome vindo da imagem vira caminho (o
destino é `<destino>/<Title ID>/<tipo>/<SHA-1>` e `.data/DataNNNN`); a
pasta `.data` de uma conversão anterior é apagada com `remove_dir_all`, que
não segue link simbólico; uma falha ou o Ctrl+C apagam a saída incompleta e
as pastas que a conversão criou, sem tocar no destino do usuário. A
ferramenta não usa arquivos `.parcial`: as partes são gravadas direto em
`.data`, e o cabeçalho, por último, é o que faz o pacote aparecer no
console.

### S-1 (Alta) — saída padrão fechada: pânico e aborto sem limpeza

`progresso.rs:52-56` (`emitir`) e os 111 `println!`/`eprintln!`/`print!`.
O `println!` entra em pânico quando a escrita falha (pipe fechado, terminal
que sumiu), e o perfil de release usa `panic = "abort"`: nada é limpo.

*Cenário (reproduzido):* `iso2god converter jogo.iso destino
--progresso-json | head -1` — o caso de um programa que lê o progresso e
fecha o pipe (o xiso-manager, se cair) — aborta com
`failed printing to stdout` e deixa um `Data0000` de 170 MB.

*Correção:* `saida!` e `saida_erro!`, que escrevem e ignoram o erro de
escrita (o mesmo usado no extract-xiso-pt). A conversão continua até o fim
ou até um cancelamento, e a limpeza acontece como sempre.

### S-2 (Alta) — SIGHUP e fechar a janela no Windows: morte sem limpeza

`sistema.rs:42-108`. Só SIGINT e SIGTERM têm tratador. Fechar o terminal
manda SIGHUP, que mata o processo no meio (reproduzido: `Data0000` de
170 MB deixado para trás). No Windows, fechar a janela do console, sair da
sessão ou desligar chegam como `CTRL_CLOSE_EVENT`, `CTRL_LOGOFF_EVENT` e
`CTRL_SHUTDOWN_EVENT`; o tratador devolve `FALSE` e o processo morre na
hora.

*Correção:* SIGHUP cancela como SIGTERM, a não ser que já chegue ignorado
(`nohup`: quem rodou assim quer que continue). No Windows, os três eventos
marcam o cancelamento e esperam até 4,5 s — o Windows encerra o processo
uns 5 s depois — enquanto a thread principal apaga a saída e sai.

### S-3 (Alta) — conversão por cima de outra: cabeçalho velho com dados novos pela metade

`god/mod.rs:422-430` e `1042`. Ao converter o mesmo jogo de novo para o
mesmo destino, a pasta `.data` antiga é apagada e regravada, mas o
cabeçalho antigo (mesmo nome) fica no lugar até ser sobrescrito no fim. Se
o processo morrer no meio (SIGKILL, queda de energia, segundo Ctrl+C), sobra
um cabeçalho válido apontando para partes que faltam ou estão pela metade:
o pacote aparece no console e o jogo quebra ao ler a parte que falta.

*Correção:* apagar o cabeçalho antigo antes de mexer na pasta `.data`. Ele
seria sobrescrito de qualquer jeito; assim, o pacote só aparece pronto
quando estiver pronto.

### S-4 (Média) — link simbólico no destino redireciona a gravação

`god/cabecalho.rs:299` (`fs::write` do cabeçalho),
`god/reconstrucao.rs:57` (`File::create` da ISO reconstruída, de nome
previsível `.iso2god-<pid>-<nome>.reconstruida.iso`) e
`assistente.rs:1064-1068` (arquivo de teste de escrita
`.iso2god-escrita-<pid>`). Os três seguem link simbólico: quem puder
escrever no destino (pasta compartilhada) planta um link com o nome
esperado e a ferramenta sobrescreve — ou, no teste de escrita, **zera** — o
arquivo para onde ele aponta.

*Correção:* apagar o que houver no caminho e criar com `create_new`, que não
segue link (`remove_file` apaga o link, não o alvo). Os bytes gravados são
os mesmos.

### S-5 (Baixa, só descrito) — ISO reconstruída esquecida depois de SIGKILL

`god/mod.rs:602-616`. Com `--padding completa`, a ISO reconstruída fica no
destino durante a conversão e é apagada no fim, na falha e no cancelamento
(e, depois do S-2, no SIGHUP e ao fechar a janela). Um SIGKILL ou uma queda
de energia a deixam para trás, com o PID no nome. Apagar as de execuções
mortas exigiria conferir se o PID ainda existe, o que não é confiável (PIDs
são reutilizados). Fica descrito; o nome começa com ponto e diz o que é.

### D-1 (Baixa, só descrito) — segundo Ctrl+C

`sistema.rs:49-51`. O segundo Ctrl+C chama `_exit(130)` e deixa a saída
pela metade. É proposital ("para quem não quer esperar") e fica como está;
com o S-3, o que sobra nunca tem cabeçalho, então não aparece no console.

## 3. Tratamento de erros

### E-1 (Média) — erro de E/S sem arquivo nem operação

`erro.rs:6-7`. `Erro::Io(#[from] io::Error)` converte qualquer falha de E/S
com `?`, e a mensagem fica só com a causa, em inglês:

```
❌ erro de E/S: No such file or directory (os error 2)
```

(reproduzido com `iso2god converter /nao/existe.iso destino`). Não diz se o
que falhou foi abrir a ISO, criar a pasta, gravar a parte 3 ou o cabeçalho.

*Correção:* o modelo do extract-xiso-pt. `Erro::Arquivo { operacao,
caminho, fonte }` sem conversão automática: cada chamada dá o contexto
(`.ctx(Operacao::Abrir, caminho)`), e a causa sai em português para os
casos comuns (não existe, sem permissão, disco cheio, só leitura, arquivo
grande demais para FAT32...). O formato do evento `erro` não muda; só o
texto da `mensagem` fica útil.

### E-2 (Média) — pânico em inglês, sem evento `erro`, sem limpeza

`main.rs` (sem gancho de pânico) e `Cargo.toml` (`panic = "abort"`). Um
pânico (como o do B-6) imprime a mensagem padrão do Rust em inglês e aborta;
em `--progresso-json` quem lê o progresso não recebe o evento `erro`, e a
saída incompleta fica no disco.

*Correção:* um gancho de pânico que escreve a mensagem em português (com o
local, para o relato de defeito), emite o evento `erro` em
`--progresso-json` e apaga a saída que a conversão registrou como "em
andamento" antes do aborto. O código de saída de um pânico continua o do
aborto, como hoje.

## 4. Memória e velocidade

Linha de base (Linux x86_64, 4 núcleos, CPU com SHA-NI, imagem de 1 GiB no
cache, melhor de 3):

| Medida | 0.1.4 |
|---|---|
| `converter -j 1` | 2,22 s |
| `converter -j 4` | 0,61 s |
| `converter -j 4 --padding completa` | 1,77 s |
| chamadas `read`/`write` no `--padding completa` | 525.620 / 526.929 |
| pico de memória (RSS) | 10,4 MiB |
| binário (release) | 999.768 bytes |

O consumo de memória é baixo e constante: cada thread tem um buffer de
816 KiB, a parte guarda 41.412 hashes de 20 bytes, e a ISO nunca é carregada
inteira. Não há clones ou `String`s no caminho quente.

### P-1 (Média) — `opt-level = "z"` e o SHA-1

| Perfil | Binário | `-j 1` com SHA-NI | `-j 1` com SHA-1 por software |
|---|---|---|---|
| `z` (atual) | 999.768 B | 2,21 s | **8,71 s** |
| `z` + `opt-level = 3` em `sha1`, `digest`, `block-buffer`, `md-5` | 1.000.512 B | 2,24 s | **4,89 s** |

Com SHA-NI, igual. Sem SHA-NI (boa parte dos Intel de desktop até a 10ª
geração e vários AMD antigos), o SHA-1 compilado para tamanho é o gargalo:
44% mais lento. *Correção:* `opt-level = 3` só nesses pacotes (+744 bytes).
O `aes` e o `lzxd` só processam o `default.xex` (milissegundos) e ficam
como estão.

### P-2 (Média) — `--padding completa` copia 2 KiB por chamada

`god/reconstrucao.rs:253-286`. `copiar_arquivo` lê e grava um setor de cada
vez: 1,05 milhão de chamadas ao sistema por GiB. *Correção:* buffer de
1 MiB (múltiplo do setor), o último trecho completado com zero até o fim do
setor, como hoje. Mesmos bytes.

### P-3 (Baixa) — uma thread que falha não para as outras

`god/mod.rs:771-828`. Se uma thread falha (erro de leitura), as outras
continuam lendo e gravando até o fim da parte — até 170 MB de trabalho
jogado fora antes de a falha aparecer. *Correção:* um sinalizador de parada
compartilhado, consultado a cada grupo de 204 blocos.

### P-4 (Baixa) — canal por bloco, SHT serializada duas vezes

`god/mod.rs:873-876`: um envio pelo canal a cada bloco de 4 KiB (262 mil
por GiB; 14 mil a 56 mil `futex` medidos). `god/mod.rs:905-907`: cada Sub
Hash Table é serializada duas vezes (para gravar e para o hash).
`god/mod.rs:846`: o buffer de 816 KiB é alocado de novo a cada parte.
*Correção:* enviar os hashes do grupo inteiro numa mensagem e serializar a
SHT uma vez só.

## 5. Concorrência e E/S

O desenho atual já é bom: cada thread lê uma faixa contígua da parte e grava
os blocos direto na posição final (sem placeholder nem regravação), lendo e
gravando grupos inteiros de 204 blocos (816 KiB) por chamada. As tabelas de
hash são montadas depois, em ordem, na thread principal. Com 4 threads a
conversão de 1 GiB cai de 2,2 s para 0,6 s.

### C-1 (Baixa, só descrito)

- O padrão é `-j 1` (`cli.rs`). Mudar o padrão mudaria o comportamento de
  uma opção da linha de comando, o que esta auditoria não faz. Para SSD,
  `-j 0` (automático) é o recomendado.
- Com `-j N`, cada thread lê uma região diferente da ISO ao mesmo tempo. Num
  SSD isso é o que dá o ganho; num HD ou DVD vira busca de cabeça e pode ser
  mais lento que `-j 1`. Uma fila leitura → hash → gravação com uma única
  leitora sequencial resolveria os dois casos, mas é uma reescrita do
  núcleo da conversão; fica para depois, medida num HD de verdade.
- A troca de parte é uma barreira (todas as threads terminam antes da
  próxima parte começar); o custo medido é pequeno (uma parte tem 170 MB).

## 6. Binário leve

O perfil de release já é o certo para tamanho (`opt-level = "z"`, LTO,
`codegen-units = 1`, `panic = "abort"`, `strip`). Fica como está, com a
exceção do P-1.

### L-1 (Baixa) — `terminal_size`

`terminal.rs:299-303`. Puxa `rustix` e `linux-raw-sys` para fazer um
`ioctl(TIOCGWINSZ)` — e, no Windows, um `GetConsoleScreenBufferInfo` —, as
duas coisas disponíveis em `libc` e `windows-sys`, que já são dependências.
*Correção:* as duas chamadas diretas, como no extract-xiso-pt.

### L-2 (Baixa) — `base64`

`analise.rs:79-81, 100-102`. O pacote inteiro para codificar o PNG do
`info --json`. Um codificador de 15 linhas tira 6 KB do binário; o texto
gerado é o mesmo (conferido pelo golden do `info --json`).

### L-3 (Baixa, só descrito)

O `clap` é a maior dependência; trocá-lo mudaria a ajuda e as mensagens de
uso. O `thiserror 1` traz o `syn 2` além do `syn 3` usado por `clap` e
`serde` (só tempo de compilação, não binário). O `png` precisa ficar
exatamente na mesma versão e configuração: o PNG da miniatura do XBE vai
para o cabeçalho.

## 7. Só descritos: mudariam os bytes gravados

### N-1 (Alta, só descrito) — `--padding completa` grava nomes não-ASCII como `?`

`gdf/diretorio.rs:73-78` decodifica os nomes como o `Encoding.ASCII` do
.NET (byte ≥ 0x80 vira `?`), e `diretorio.rs:51` grava esse texto de volta
na GDF reconstruída. Um arquivo `Canção.ogg` (Latin-1) vira `Can??o.ogg` no
pacote: o jogo procura o nome original e não acha. Os goldens registram
exatamente isso. A correção (guardar e gravar os bytes originais do nome)
muda os bytes do pacote para essas imagens, então **não foi aplicada**. Só
afeta `--padding completa` com nomes fora do ASCII; `parcial` e `nenhuma`
copiam os setores como estão.

### N-2 (Média, só descrito) — reconstrução casa entradas pelo nome

`god/reconstrucao.rs:203-210, 229-237`. A árvore remapeada é um clone da
original, mas as duas são casadas por nome (`encontrar`, sem diferenciar
maiúsculas), não por posição. Dois nomes iguais na mesma pasta — por
diferença só de maiúsculas, ou por virarem o mesmo texto depois do N-1 —
fazem o segundo receber os dados do primeiro. Casar por posição corrige,
mas muda os bytes para essas imagens.

### N-3 (Baixa, só descrito) — Title ID em minúsculas

`god/mod.rs:403` usa o Title ID em maiúsculas para a pasta, mas
`nome_unico` (`god/mod.rs:1052-1061`) usa o texto como foi digitado.
`--title-id 4d5308bf` gera outro nome de pacote que `--title-id 4D5308BF`
(está no golden das opções manuais). Normalizar mudaria o nome gravado.

### N-4 (Baixa, só descrito) — varredura linear das tabelas

`gdf/diretorio.rs:95-152`. A tabela é lida de ponta a ponta, não seguindo
os ponteiros da árvore: uma região zerada no fim da tabela vira entradas
vazias que entram no cálculo do último setor. Seguir a árvore é o certo,
mas pode mudar o ponto de corte do `--padding parcial` em alguma imagem.

### N-5 (Baixa, só descrito) — limite da compressão básica

`xex/recursos.rs:139` compara com `tamanho_imagem.max(MAX_IMAGEM)`, ou
seja, sempre 256 MiB. O limite certo seria o tamanho declarado; apertar
pode recusar um XEX aceito hoje (e trocar o nome e o ícone gravados), e o
teto de 256 MiB já impede o abuso.
