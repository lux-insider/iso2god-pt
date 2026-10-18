# Histórico de versões

Todas as mudanças relevantes do iso2god-pt, da mais nova para a mais antiga.
As versões seguem o [Versionamento Semântico](https://semver.org/lang/pt-BR/).

| Versão | Data | Destaque |
|---|---|---|
| [0.1.5](#015--2026-10-01) | 01/10/2026 | Robustez contra arquivos corrompidos e nomes com acento |
| [0.1.4](#014--2026-09-30) | 30/09/2026 | Ícone X8R8G8B8 de jogos de Xbox Original |
| [0.1.3](#013--2026-09-29) | 29/09/2026 | Nome e ícone dos jogos de Xbox 360 |
| [0.1.2](#012--2026-09-29) | 29/09/2026 | Raiz no corte parcial, cancelamento limpo e desempenho |
| [0.1.1](#011--2026-09-24) | 24/09/2026 | Primeira versão pública |

---

## 0.1.5 — 2026-10-01

### Corrigido
- **`--padding completa` com nomes acentuados:** nomes como `Canção.ogg` eram
  gravados como `Can??o.ogg`, e o jogo não achava o arquivo; dois nomes que
  viravam o mesmo texto faziam o segundo arquivo receber os dados do primeiro.
- `--title-id` com acento encerrava o programa.

### Segurança
- ISOs, `default.xex` e `default.xbe` malformados viram erro explicado em vez
  de travar ou esgotar a memória (ex.: uma ISO de 9 MB com diretórios
  compartilhados passava de 4 GB de memória).
- Fechar o terminal ou a janela no meio da conversão cancela e apaga o pacote
  incompleto, como o Ctrl+C.
- Ao converter de novo o mesmo jogo, o cabeçalho antigo é removido primeiro,
  para um pacote pela metade nunca parecer pronto.
- Um link simbólico no destino não redireciona mais a gravação.
- Ctrl+C também funciona durante a leitura da árvore de arquivos.
- Os erros de leitura e gravação informam o arquivo e a operação.

### Desempenho

| Conversão de 1 GiB | 0.1.4 | 0.1.5 |
|---|---|---|
| 1 thread (padrão) | 2,21 s | 1,17 s |
| 1 thread, processador sem instruções SHA | 7,13 s | 2,48 s |
| 4 threads | 0,55 s | 0,37 s |

---

## 0.1.4 — 2026-09-30

### Adicionado
- Ícones de jogos de Xbox Original em **X8R8G8B8** (formato 7 da textura
  XPR). Antes, esses jogos (ex.: Darkwatch) ficavam sem ícone.

---

## 0.1.3 — 2026-09-29

### Adicionado
- **Nome e ícone dos jogos de Xbox 360.** O `default.xex` é decifrado (AES) e
  descomprimido (básica ou LZX), e o recurso XDBF fornece o nome oficial, no
  idioma padrão do jogo, e o ícone. Antes, o jogo aparecia com o nome do
  arquivo da ISO e sem ícone. `--titulo` e `--icone` continuam tendo
  prioridade.

### Validação
- 89 testes, incluindo XEX sintéticos com compressão básica e LZX, truncados
  e corrompidos em todos os pontos, sem pânico.
- Resident Evil 5: nome e ícone no cabeçalho iguais aos de um protótipo
  independente; o `.exe` no Wine gera pacote idêntico byte a byte ao do Linux.

---

## 0.1.2 — 2026-09-29

### Corrigido
- **Corte parcial (padrão) perdia a tabela do diretório raiz** em imagens
  reconstruídas ou homebrew em que a raiz fica depois dos dados.
- **SIGTERM** cancela de forma limpa, apagando a saída incompleta.
- **Windows:** Ctrl+C e Ctrl+Break cancelam de forma limpa; um segundo Ctrl+C
  sai na hora.

### Desempenho
- Leitura e gravação em grupos de 204 blocos. Resident Evil 5 (7 GB, `-j 12`):
  de 16 s para 9,4 s, com a mesma saída byte a byte.

### Validação
- 79 testes, teste diferencial em imagens sintéticas e na ISO do Resident
  Evil 5, verificador independente do pacote gerado e fuzz com cerca de 2.900
  imagens corrompidas.

---

## 0.1.1 — 2026-09-24

Primeira versão pública.

- Windows: menu e barra de progresso sem caracteres quebrados; emoji no
  Windows Terminal e símbolos simples no console clássico.
- Espaço livre no destino verificado antes de converter.
- O `.exe` não depende do Visual C++ Redistributable.
- Binários pequenos: cerca de 960 KB no Linux e 830 KB no Windows.
