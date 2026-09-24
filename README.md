# iso2god

Ferramenta para converter imagens ISO de Xbox e Xbox 360 em containers **GOD**
(Games on Demand), reescrita do zero em **Rust**, com uma interface de
terminal rica (cores, gradiente, barra de progresso animada, emojis) e um
assistente interativo passo a passo.

Este projeto é uma reimplementação independente, em Rust, do formato e da
lógica de conversão usados pelo Iso2God original (Chilano's Iso2God) — nenhum
código-fonte do projeto original foi copiado; o comportamento foi reproduzido
a partir da análise do formato de arquivo e de uma versão descompilada usada
como referência.

## Recursos

- Detecção automática da plataforma (Xbox Original via `default.xbe` / Xbox
  360 via `default.xex`) — Title ID, Media ID, título e número de disco lidos
  direto da imagem.
- Extração e conversão do thumbnail do jogo (XPR/DXT1 e ARGB) para PNG, no
  caso de Xbox Original.
- Três estratégias de padding: nenhuma remoção, remoção parcial (padrão) ou
  reconstrução completa da estrutura GDF.
- Leitura e cálculo de hash multi-thread, configurável (`-j`/`--threads`).
- Duas formas de usar o CLI:
  - **Direto por flags** (`converter`/`info`), com saída visual rica.
  - **Menu interativo** (rodando sem nenhum argumento): navegador de pastas
    para escolher a imagem, assistente passo a passo com resumo antes de
    converter, e conversão em lote de várias ISOs de uma vez.
- Protocolo de progresso estruturado em JSON (`--progresso-json` / `info
  --json`), pensado para outros programas consumirem sem precisar entender a
  saída colorida do terminal.
- Comportamento correto quando a saída não é um terminal (ex: redirecionada
  para um arquivo): sem códigos ANSI, sem barra "ao vivo", só uma linha de
  resumo limpa por evento. Erros e avisos vão para stderr, então redirecionar
  stdout não esconde falha nenhuma.
- Verificações antes de escrever qualquer byte: a imagem é mesmo de
  Xbox/Xbox 360, a pasta de destino aceita escrita, e há espaço livre para o
  pacote (a conversão é recusada de cara em vez de falhar a 90% do caminho).
- Ctrl+C cancela de forma limpa (Linux): a conversão para num ponto conhecido e a
  saída incompleta é descartada, em vez de ficar uma pasta pela metade com
  cara de pacote pronto. O mesmo vale para qualquer falha no meio.

## Instalação / build

Requer o [Rust](https://rustup.rs) instalado (edição 2024, toolchain estável
recente).

Baixe um binário pronto na [página de Releases](https://github.com/lux-insider/iso2god-pt/releases), ou compile a partir do código-fonte:

```bash
git clone https://github.com/lux-insider/iso2god-pt.git
cd iso2god-pt
cargo build --release
```

O binário fica em `target/release/iso2god`. Para poder chamá-lo de
qualquer lugar:

```bash
cp target/release/iso2god ~/.local/bin/
# garanta que ~/.local/bin está no seu PATH
```

### Windows

O `iso2god.exe` da página de Releases roda no Windows 10/11 (64 bits) sem
instalar nada. No Windows Terminal aparece com cores e emoji; no console
clássico (cmd e PowerShell antigos) aparece com cores e símbolos simples no
lugar dos emoji, que ele não desenha. No Windows o Ctrl+C encerra na hora —
o cancelamento limpo ainda é só no Linux.

Para gerar o `.exe` a partir do Linux:

```bash
sudo apt install clang lld llvm
rustup target add x86_64-pc-windows-msvc
cargo install --locked cargo-xwin
cargo xwin build --release --target x86_64-pc-windows-msvc
```

O `.exe` fica em `target/x86_64-pc-windows-msvc/release/iso2god.exe`. O
`.cargo/config.toml` do repositório liga o CRT estático, então ele não
depende do Visual C++ Redistributable.

## Uso

### Assistente interativo (recomendado para uso manual)

```bash
iso2god
```

Sem nenhum argumento, abre um menu:

```
  ── Converter ──────────────────────────────────────────── GOD ──
   [1]  🎮  ISO para GOD             assistente guiado
   [2]  🚀  Lote: uma pasta inteira  todas as ISOs de uma vez

  ── Inspecionar ─────────────────────────────────────────────────
   [3]  🔍  Analisar ISO             sem converter
```

- **Navegador de pastas**: escolha a imagem navegando, sem digitar caminho.
  A listagem mostra tamanho e o layout detectado (Xsf/XGD1/XGD2/XGD3, ou `?`
  quando o arquivo não é uma imagem de Xbox), então dá para ver de relance o
  que vale a pena abrir. `..` sobe, `c` digita um caminho, `t` leva todas as
  ISOs da pasta.
- **Assistente em quatro etapas** depois da escolha: destino, padding,
  threads e opções avançadas (título, ícone e numeração de disco — as mesmas
  da CLI), com resumo e confirmação antes de converter.
- **Lote**: `2`, `1,3-5` ou `todas`. A falha de uma imagem não interrompe as
  outras, e no fim sai um resumo do lote.
- **Caminhos**: `~/Jogos/x.iso` funciona, assim como caminhos colados com
  aspas ou com espaços escapados (arrastar o arquivo do gerenciador de
  arquivos para o terminal).
- **Navegação**: `v` volta uma etapa, `q` sai, Enter aceita o padrão
  mostrado entre colchetes. Ctrl+D encerra e Ctrl+C cancela a conversão em
  andamento sem deixar lixo.
- **Antes de confirmar** ele mostra quanto o pacote deve ocupar e quanto há
  livre no destino.

O processo sai com código 1 se alguma imagem do lote não converteu, para dar
para usar o assistente em cima de um script.

### Direto por flags

```bash
# conversão
iso2god converter "/caminho/do/jogo.iso" "/pasta/destino" -j 4

# só ver informações da ISO (plataforma, Title ID, Media ID, título...)
iso2god info "/caminho/do/jogo.iso"
```

Outras flags úteis de `converter` (veja `iso2god converter --help` para a
lista completa): `--padding <nenhuma|parcial|completa>`, `--title-id`,
`--media-id`, `--titulo`, `--disco`, `--total-discos`, `--icone`.

### Onde o pacote é gravado

```
<destino>/<Title ID>/<tipo de conteúdo>/<pacote>
                                        <pacote>.data/Data0000, Data0001, ...
```

Por exemplo, `iso2god converter jogo.iso /mnt/usb/` grava em
`/mnt/usb/545400A3/00005000/…`. O tipo de conteúdo é `00005000` para jogos de
Xbox original e `00007000` para Xbox 360 — é o mesmo layout que o console
procura dentro de `Content/0000000000000000/`, então dá para copiar a pasta
do Title ID direto para lá.

### Modo máquina (para integrar com outro programa)

```bash
iso2god converter origem.iso destino/ --progresso-json
iso2god info origem.iso --json
```

Emite uma linha JSON por evento em stdout (fases, progresso com bytes/
velocidade/ETA, conclusão ou erro) em vez da saída colorida — é o protocolo
pensado para integração com outros programas.


## Estrutura do repositório

```
src/            código-fonte do CLI/engine de conversão (Rust)
tests/          testes de integração (conversão real, leitura de GDF, etc.)
```

## Créditos

- Baseado no formato e comportamento do **Iso2God** original (Chilano's
  Iso2God) — este repositório não inclui nem redistribui o código-fonte
  original.
- Decodificação de XPR/DXT1 inspirada no
  [UIXTool](https://github.com/Ernegien/UIXTool).

## Licença

[MIT](LICENSE).
