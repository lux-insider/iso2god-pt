# Fuzzers do iso2god

Duas ferramentas que jogam entrada corrompida contra o binário e procuram por
**pânico ou travamento** — não por erro. Um erro tratado (código 1 com
mensagem em português) é o comportamento certo para uma imagem quebrada e não
conta como achado.

Sempre rodam contra o **binário de debug**. Isso é deliberado: o release faz
a aritmética dar a volta em silêncio, enquanto o debug aborta com "attempt to
multiply with overflow". Foi assim que apareceu o estouro no decodificador de
thumbnail — em release ele viraria um acesso fora dos limites bem adiante,
difícil de ligar à causa.

```bash
cargo build            # precisa ser debug
python3 ferramentas/fuzz/iso_corrompida.py --bordas
python3 ferramentas/fuzz/xbe_forjado.py --bordas
```

## Os dois modos

**`--bordas`** enumera combinações hostis conhecidas — uma passada de poucos
segundos que pega regressão sempre. É o modo para rodar antes de publicar.

**Sem `--bordas`** sorteia casos aleatórios, procurando o que ninguém previu.
É o modo para deixar rodando com um número alto de casos.

A diferença importa mais do que parece: o estouro do decodificador ARGB só
acontece com expoente de tamanho 31 (textura de 2³¹×2³¹), um ponto único num
espaço enorme. Medido, a busca aleatória precisava de centenas de casos para
cair nele; a enumeração pega em segundos, sempre. Busca aleatória serve para
o desconhecido, não para o que já se sabe onde dói.

## `--cobertura`: o fuzzer está mesmo chegando lá?

```bash
python3 ferramentas/fuzz/xbe_forjado.py 200 --cobertura
python3 ferramentas/fuzz/iso_corrompida.py 50 --cobertura
```

Em vez de caçar pânico, classifica **onde cada caso parou**. Rode isto sempre
que mexer num gerador.

O motivo é concreto: o gerador do `xbe_forjado.py` já esteve quebrado sem
ninguém notar. A área de pixel era menor que a textura declarada no
cabeçalho, então toda capa era recusada por "dados insuficientes" e o
decodificador nunca rodava — foram 6.000 execuções que pareciam cobrir o
decodificador e não cobriam nada. A medição mostrou 2,7% de cobertura; depois
do conserto, 37%.

Um fuzzer que não chega na função não protege ela, e "rodou 10.000 casos sem
achar nada" pode significar as duas coisas opostas. Os dois modos avisam
quando a cobertura cai abaixo de 10%.

Referência do que é saudável hoje:

| Ferramenta | Alvo da medição | Esperado |
|---|---|---|
| `xbe_forjado.py` | casos que decodificam a textura | ~37% |
| `iso_corrompida.py` | conversões que gravam o pacote inteiro | ~40% |

## O que cada um ataca

| Ferramenta | Alvo |
|---|---|
| `iso_corrompida.py` | estrutura GDF: árvore de diretórios, setores, tamanhos, ciclos, profundidade |
| `xbe_forjado.py` | `default.xbe` embutido: cabeçalho, certificado, tabela de seções e a textura XPR do thumbnail |

O gerador aleatório do `xbe_forjado.py` parte de um XBE **válido** e aplica
uma ou duas mutações por caso. A versão anterior sorteava todos os campos de
uma vez, e quase toda imagem morria na primeira validação sem nunca chegar ao
decodificador.

## Opções

```
casos                quantos casos gerar (modo aleatório)
--bordas             enumera as combinações conhecidas
--cobertura          mede onde os casos param, em vez de caçar pânico
--semente N          repete uma rodada anterior exatamente
--forcar             passa --plataforma/--title-id/--media-id, para a conversão
                     passar da detecção de metadados e exercitar a escrita
                     (só em iso_corrompida.py)
--bin CAMINHO        outro binário alvo
--achados PASTA      onde guardar as imagens que quebraram (padrão: ./achados)
```

Toda rodada imprime a semente. Achou algo, `--semente N` reproduz a rodada
inteira, e a imagem culpada fica guardada em `achados/`.

## O que estas ferramentas já encontraram

- Conversão que gerava **32 GB de zeros** a partir de uma imagem de 2 MiB
  (achado pelo modo aleatório, como um travamento que na verdade era uma
  conversão real e absurda).
- Estouro de soma em `processar_diretorio` com setor e tamanho no limite do
  `u32` — em release dava a volta e podia truncar a conversão em silêncio
  (achado pelo modo `--bordas`, na primeira execução dele).

Os outros defeitos corrigidos no projeto vieram de **leitura do código**, não
destas ferramentas — vale registrar para não superestimar o que elas fazem.
Cada um deles virou também um teste em `cargo test`, que é onde a proteção de
verdade mora; os fuzzers existem para achar o que ninguém pensou em testar.
