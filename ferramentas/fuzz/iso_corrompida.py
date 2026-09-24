#!/usr/bin/env python3
"""Fuzzer da estrutura GDF: imagens com árvore de diretórios corrompida.

Gera imagens com descritor de volume válido (para passar da porta de entrada)
e uma árvore de diretórios cheia de valores extremos — setores fora da
imagem, tamanhos de 2 GiB, nomes de 255 caracteres, entradas marcadas como
diretório apontando para qualquer lugar —, e roda `info` e `converter` nos
três modos de padding em cima de cada uma.

Foi assim que apareceram dois problemas reais: o estouro ao reescrever a
tabela de diretório (`--padding completa`) e a conversão que gerava 32 GB de
zeros a partir de uma imagem de 2 MiB.

Uso:
    python3 iso_corrompida.py [casos] [--semente N] [--forcar] [--bin CAMINHO]

`--forcar` passa `--plataforma/--title-id/--media-id` na linha de comando, o
que faz a conversão passar da detecção de metadados e exercitar a escrita de
verdade — sem isso, imagens sem `default.xbe` param cedo demais.
"""

import argparse
import os
import random
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _comum import (DESLOCAMENTO_XGD3, SETOR, binario, detalhe, e_achado, entrada_diretorio,
                    escrever_iso, guardar_achado, relatar, rodar)


def xbe_minimo(rnd):
    """XBE válido o bastante para a detecção de metadados funcionar."""
    import struct
    base = 0x0001_0000
    cabecalho = 284
    b = bytearray(cabecalho)
    b[0:4] = struct.pack("<I", 1_212_498_520)  # "XBEH"
    b[260:264] = struct.pack("<I", base)
    b[280:284] = struct.pack("<I", base + cabecalho)
    cert = bytearray(172)
    cert[8:12] = struct.pack("<I", rnd.randrange(0, 1 << 32))
    nome = "Jogo de Teste".encode("utf-16-le")
    cert[12:12 + len(nome)] = nome
    cert[168:172] = struct.pack("<I", 1)
    return bytes(b + cert)


def gerar(caminho, rnd):
    """Uma imagem com árvore de diretórios deliberadamente hostil."""
    setor_raiz = rnd.choice([100, 100, 100, rnd.randrange(0, 5000)])
    setor_xbe = 200
    xbe = xbe_minimo(rnd)

    raiz = bytearray(entrada_diretorio(setor_xbe, len(xbe), 0x00, "default.xbe"))
    for _ in range(rnd.randrange(0, 20)):
        raiz += entrada_diretorio(
            rnd.choice([setor_xbe, rnd.randrange(0, 10_000), rnd.randrange(0, 1 << 31)]),
            rnd.choice([len(xbe), 2048, rnd.randrange(0, 1 << 31)]),
            rnd.choice([0x00, 0x10]),  # 0x10 = diretório: força a travessia recursiva
            bytes(rnd.randrange(65, 91) for _ in range(rnd.choice([8, rnd.randrange(1, 256)]))),
        )
    tamanho_raiz = rnd.choice([len(raiz), 2048, 4096, rnd.randrange(0, 8192)])
    while len(raiz) % SETOR:
        raiz.append(0xFF)

    total = DESLOCAMENTO_XGD3 + setor_xbe * SETOR + len(xbe) + rnd.randrange(0, 4 * 1024 * 1024)
    escrever_iso(caminho, raiz, setor_raiz, tamanho_raiz, [(setor_xbe, xbe)], total)


def casos_de_borda(rnd):
    """Estruturas hostis conhecidas, enumeradas em vez de sorteadas.

    Cada uma reproduz uma classe de defeito que já apareceu neste projeto (ou
    que a leitura do código apontou como possível). Rodam em segundos e pegam
    a regressão sempre — busca aleatória levaria centenas de casos para cair
    em qualquer uma delas.

    Devolve (bloco da raiz, setor da raiz, tamanho declarado da raiz,
    conteúdos, tamanho total, descrição).
    """
    xbe = xbe_minimo(rnd)
    setor_xbe = 200
    volume = DESLOCAMENTO_XGD3 + 400 * SETOR

    # 1. Entradas que, ao serem REESCRITAS, não cabem no tamanho declarado:
    #    a leitura é linear e aceita, a escrita impõe fronteira de setor.
    #    (estouro em TabelaDiretorio::para_bytes, via --padding completa)
    raiz = bytearray()
    for i in range(15):
        raiz += entrada_diretorio(300, 2048, 0x00, bytes([65 + i % 26]) * 255)
    raiz_1 = bytearray(raiz)
    while len(raiz_1) % SETOR:
        raiz_1.append(0xFF)
    yield raiz_1, 100, 4096, [(setor_xbe, xbe)], volume, "15 entradas de nome 255 numa tabela de 4096"

    # 2. Diretório que aponta para a própria raiz: travessia infinita.
    raiz_2 = bytearray(entrada_diretorio(100, SETOR, 0x10, "EU_MESMO"))
    raiz_2 += entrada_diretorio(100, SETOR, 0x10, "DE_NOVO")
    while len(raiz_2) % SETOR:
        raiz_2.append(0xFF)
    yield raiz_2, 100, len(raiz_2), [], volume, "diretório apontando para si mesmo"

    # 3. Arquivo que termina muito além do fim da imagem: convertia isso em
    #    dezenas de GB de zeros a partir de uma imagem de poucos MB.
    raiz_3 = bytearray(entrada_diretorio(setor_xbe, 2 << 30, 0x00, "gigante.bin"))
    raiz_3 += entrada_diretorio(setor_xbe, len(xbe), 0x00, "default.xbe")
    while len(raiz_3) % SETOR:
        raiz_3.append(0xFF)
    yield raiz_3, 100, len(raiz_3), [(setor_xbe, xbe)], volume, "entrada de 2 GiB num volume de 800 KiB"

    # 4. Nenhum setor ocupado: a conversão calculava zero blocos e subtraía 1.
    raiz_4 = bytearray(b"\xFF" * SETOR)
    yield raiz_4, 100, SETOR, [], volume, "raiz sem nenhuma entrada válida"

    # 5. Raiz apontando para fora da imagem.
    yield bytearray(b"\xFF" * SETOR), 900_000, SETOR, [], volume, "diretório raiz fora da imagem"

    # 6. Tamanhos no limite de u32.
    raiz_6 = bytearray(entrada_diretorio(0xFFFF_FFFF, 0xFFFF_FFFF, 0x00, "limite.bin"))
    while len(raiz_6) % SETOR:
        raiz_6.append(0xFF)
    yield raiz_6, 100, len(raiz_6), [], volume, "entrada com setor e tamanho 0xFFFFFFFF"

    # 7. Tamanhos declarados que não cabem no volume: antes de serem
    #    recusados, viravam um `vec![0u8; 4 GiB]` — que passa despercebido em
    #    máquina folgada e ABORTA o processo sob limite de memória.
    raiz_a = bytearray(b"\xFF" * SETOR)
    yield raiz_a, 100, 0xFFFF_FFFF, [], volume, "tabela de diretório raiz declarando 4 GiB"

    raiz_b = bytearray(entrada_diretorio(setor_xbe, 0xFFFF_FFFF, 0x00, "default.xbe"))
    while len(raiz_b) % SETOR:
        raiz_b.append(0xFF)
    yield raiz_b, 100, len(raiz_b), [], volume, "default.xbe declarando 4 GiB"

    raiz_c = bytearray(entrada_diretorio(300, 0xFFFF_FFFF, 0x10, "PASTA"))
    while len(raiz_c) % SETOR:
        raiz_c.append(0xFF)
    yield raiz_c, 100, len(raiz_c), [], volume, "subdiretório declarando 4 GiB"

    # 8. Cadeia profunda de diretórios, cada um apontando para o seguinte.
    conteudos = []
    for nivel in range(80):
        bloco = bytearray(entrada_diretorio(300 + nivel + 1, SETOR, 0x10, f"N{nivel}"))
        while len(bloco) % SETOR:
            bloco.append(0xFF)
        conteudos.append((300 + nivel, bytes(bloco)))
    raiz_7 = bytearray(entrada_diretorio(300, SETOR, 0x10, "FUNDO"))
    while len(raiz_7) % SETOR:
        raiz_7.append(0xFF)
    yield raiz_7, 100, len(raiz_7), conteudos, volume, "80 níveis de diretório aninhados"


def medir_cobertura(alvo, rnd, casos, tmp):
    """Classifica ONDE cada conversão parou, em vez de caçar pânico.

    Um fuzzer que não chega na função não protege ela: sem esta medição não
    dá para distinguir "rodou 10.000 casos e o programa aguentou" de "rodou
    10.000 casos que morreram todos na primeira validação".
    """
    import collections
    import re
    tally = collections.Counter()
    forcar = ["--plataforma", "xbox", "--title-id", "4D5308BF", "--media-id", "AABBCCDD"]
    total = 0
    for _ in range(casos):
        iso = os.path.join(tmp, "cobertura.iso")
        destino = os.path.join(tmp, "saida")
        gerar(iso, rnd)
        for padding in ("parcial", "nenhuma", "completa"):
            shutil.rmtree(destino, ignore_errors=True)
            codigo, saida = rodar(alvo, ["converter", iso, destino, "--padding", padding] + forcar)
            total += 1
            if codigo == 0:
                tally["converteu até o fim (gravou o pacote)"] += 1
            else:
                achado = re.search(r"\u274c (?:imagem ISO inv\u00e1lida: )?(.{0,40})", saida)
                tally[f"parou: {achado.group(1).strip() if achado else '?'}…"] += 1
        shutil.rmtree(destino, ignore_errors=True)

    print(f"\ncobertura em {total} conversões:")
    for rotulo, n in tally.most_common(8):
        print(f"  {n:5}  ({100 * n / total:4.1f}%)  {rotulo}")
    completas = tally["converteu até o fim (gravou o pacote)"]
    if completas < total // 10:
        print("\n  ATENÇÃO: menos de 10% das conversões chega ao fim — o gerador"
              "\n  provavelmente está produzindo imagens recusadas cedo demais.")
        return 1
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("casos", nargs="?", type=int, default=100)
    ap.add_argument("--semente", type=int, default=None, help="repete uma rodada anterior")
    ap.add_argument("--forcar", action="store_true", help="passa metadados por flag")
    ap.add_argument("--cobertura", action="store_true",
                    help="mede onde as conversões param, em vez de caçar pânico")
    ap.add_argument("--bordas", action="store_true",
                    help="enumera as estruturas hostis conhecidas em vez de sortear")
    ap.add_argument("--bin", dest="alvo", default=None)
    ap.add_argument("--achados", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "achados"))
    args = ap.parse_args()

    alvo = binario(args.alvo)
    semente = args.semente if args.semente is not None else random.randrange(1 << 30)
    rnd = random.Random(semente)
    print(f"alvo: {alvo}\nsemente: {semente} (repita com --semente {semente})")

    tmp = tempfile.mkdtemp(prefix="iso2god-fuzz-")
    if args.cobertura:
        try:
            return medir_cobertura(alvo, rnd, args.casos, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    achados = []
    executados = 0
    try:
        fonte = list(casos_de_borda(rnd)) if args.bordas else None
        total = len(fonte) if fonte else args.casos
        if fonte:
            print(f"modo bordas: {total} estruturas conhecidas")
        for numero in range(total):
            iso = os.path.join(tmp, "caso.iso")
            destino = os.path.join(tmp, "saida")
            if fonte:
                raiz, setor_raiz, tam_raiz, conteudos, tamanho, descricao_caso = fonte[numero]
                escrever_iso(iso, raiz, setor_raiz, tam_raiz, conteudos, tamanho)
                print(f"  · {descricao_caso}")
            else:
                gerar(iso, rnd)

            forcar = []
            if args.forcar or args.bordas:
                forcar = ["--plataforma", rnd.choice(["xbox", "xbox360"]),
                          "--title-id", "4D5308BF", "--media-id", "AABBCCDD",
                          "-j", str(rnd.choice([1, 4]))]

            for padding in (None, "parcial", "nenhuma", "completa"):
                shutil.rmtree(destino, ignore_errors=True)
                if padding is None:
                    argumentos, descricao = ["info", iso], "info"
                else:
                    argumentos = ["converter", iso, destino, "--padding", padding] + forcar
                    descricao = f"converter --padding {padding}"
                codigo, saida = rodar(alvo, argumentos)
                executados += 1
                if e_achado(codigo, saida):
                    arquivo = guardar_achado(iso, args.achados, len(achados) + 1)
                    achados.append((numero, descricao, codigo, detalhe(saida), arquivo))
                    print(f"  ! caso {numero}: {descricao} -> {codigo} {detalhe(saida)}")
            shutil.rmtree(destino, ignore_errors=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    return relatar(executados, achados)


if __name__ == "__main__":
    sys.exit(main())
