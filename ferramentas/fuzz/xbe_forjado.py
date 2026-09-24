#!/usr/bin/env python3
"""Fuzzer do executável embutido: `default.xbe` com cabeçalho e textura forjados.

Ataca a outra metade do programa — a que lê metadados de dentro da imagem.
Randomiza endereço base, endereço do certificado, tabela de seções e, o mais
produtivo, o cabeçalho XPR do thumbnail: assinatura, tamanhos declarados,
formato (DXT1/ARGB/inválido) e o **expoente** de tamanho da textura.

Esse expoente foi onde apareceu o segundo estouro: a dimensão vem de
`1 << dados[27]`, então uma textura de 2^31 x 2^31 é trivial de forjar e a
conta de bytes não cabia em `usize`. Por isso os expoentes extremos (30, 31,
32) entram na roleta de propósito.

Uso:
    python3 xbe_forjado.py [casos] [--semente N] [--bin CAMINHO]
"""

import argparse
import os
import random
import shutil
import struct
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from _comum import (DESLOCAMENTO_XGD3, SETOR, binario, detalhe, e_achado, entrada_diretorio,
                    escrever_iso, guardar_achado, relatar, rodar)

ASSINATURA_XPR0 = 810_700_888
NOMES_SECAO = [b"$$XSIMAGE\x00", b"$$XTIMAGE\x00", b"OUTRA\x00"]


def xbe_valido(expoente=8, formato=12):
    """Um XBE completo e coerente: é daqui que cada caso parte."""
    base = 0x0001_0000
    cabecalho = 284

    cert = bytearray(172)
    cert[8:12] = struct.pack("<I", 0x4D53_08BF)
    nome = "Jogo de Teste".encode("utf-16-le")
    cert[12:12 + len(nome)] = nome
    cert[168:172] = struct.pack("<I", 1)

    off_secoes = cabecalho + 172
    off_nome = off_secoes + 56
    nome_secao = b"$$XSIMAGE\x00"
    off_xpr = off_nome + len(nome_secao)

    # A área de pixel precisa caber a textura de verdade, senão o
    # decodificador nunca roda: ARGB gasta 4 bytes por pixel (o pior caso),
    # DXT1 gasta 8 bytes por bloco de 4x4. Reservar de menos aqui fazia
    # TODA capa ser recusada por "dados insuficientes" antes da decodificação
    # — o fuzzer parecia rodar e não exercitava nada.
    # Teto de 256x256: acima disso a área só cresceria sem exercitar nada de
    # novo, e um expoente extremo (30, 31, 255) tem que ser recusado pelo
    # programa justamente por declarar mais do que os dados comportam.
    lado = 1 << expoente if expoente <= 8 else 64
    tam_xpr = 28 + lado * lado * 4

    xpr = bytearray(tam_xpr)
    xpr[0:4] = struct.pack("<I", ASSINATURA_XPR0)
    xpr[4:8] = struct.pack("<I", tam_xpr)
    xpr[8:12] = struct.pack("<I", 28)
    xpr[25] = formato
    xpr[27] = expoente

    b = bytearray(off_xpr + tam_xpr)
    b[0:4] = struct.pack("<I", 1_212_498_520)  # "XBEH"
    b[260:264] = struct.pack("<I", base)
    b[280:284] = struct.pack("<I", base + cabecalho)
    b[cabecalho:cabecalho + 172] = cert
    b[0x11C:0x120] = struct.pack("<I", 1)                    # NumberOfSections
    b[0x120:0x124] = struct.pack("<I", base + off_secoes)    # SectionHeadersAddress

    secao = bytearray(56)
    secao[12:16] = struct.pack("<I", off_xpr)      # RawAddress
    secao[16:20] = struct.pack("<I", tam_xpr)      # RawSize
    secao[20:24] = struct.pack("<I", base + off_nome)
    b[off_secoes:off_secoes + 56] = secao
    b[off_nome:off_nome + len(nome_secao)] = nome_secao
    b[off_xpr:off_xpr + tam_xpr] = xpr
    return bytearray(b), {"base": base, "off_secoes": off_secoes, "off_nome": off_nome,
                          "off_xpr": off_xpr, "tam_xpr": tam_xpr}


# Cada mutação mexe em UM campo. Sortear tudo de uma vez faz quase toda
# imagem morrer na primeira validação, sem nunca chegar ao decodificador —
# medido: o gerador antigo levaria ~6000 casos para reencontrar um estouro
# que este acha em dezenas.
def _u32(rnd):
    return rnd.choice([0, 1, 0xFFFF_FFFF, rnd.randrange(0, 1 << 32), rnd.randrange(0, 1 << 16)])


MUTACOES = [
    ("expoente da textura",   lambda b, m, r: b.__setitem__(m["off_xpr"] + 27,
                                r.choice([0, 1, 15, 16, 29, 30, 31, 32, 255]))),
    ("formato da textura",    lambda b, m, r: b.__setitem__(m["off_xpr"] + 25,
                                r.choice([0, 6, 12, 13, 255]))),
    ("file_size do XPR",      lambda b, m, r: b.__setitem__(slice(m["off_xpr"] + 4, m["off_xpr"] + 8),
                                struct.pack("<I", _u32(r)))),
    ("header_size do XPR",    lambda b, m, r: b.__setitem__(slice(m["off_xpr"] + 8, m["off_xpr"] + 12),
                                struct.pack("<I", _u32(r)))),
    ("assinatura do XPR",     lambda b, m, r: b.__setitem__(slice(m["off_xpr"], m["off_xpr"] + 4),
                                struct.pack("<I", _u32(r)))),
    ("endereço base",         lambda b, m, r: b.__setitem__(slice(260, 264), struct.pack("<I", _u32(r)))),
    ("endereço do certificado", lambda b, m, r: b.__setitem__(slice(280, 284), struct.pack("<I", _u32(r)))),
    ("número de seções",      lambda b, m, r: b.__setitem__(slice(0x11C, 0x120), struct.pack("<I", _u32(r)))),
    ("endereço da tabela de seções", lambda b, m, r: b.__setitem__(slice(0x120, 0x124), struct.pack("<I", _u32(r)))),
    ("RawAddress da seção",   lambda b, m, r: b.__setitem__(slice(m["off_secoes"] + 12, m["off_secoes"] + 16),
                                struct.pack("<I", _u32(r)))),
    ("RawSize da seção",      lambda b, m, r: b.__setitem__(slice(m["off_secoes"] + 16, m["off_secoes"] + 20),
                                struct.pack("<I", _u32(r)))),
    ("endereço do nome da seção", lambda b, m, r: b.__setitem__(slice(m["off_secoes"] + 20, m["off_secoes"] + 24),
                                struct.pack("<I", _u32(r)))),
    ("nome da seção",         lambda b, m, r: b.__setitem__(slice(m["off_nome"], m["off_nome"] + 10),
                                r.choice([b"$$XSIMAGE\x00", b"$$XTIMAGE\x00", b"QUALQUER\x00\x00"]))),
    ("byte solto do XPR",     lambda b, m, r: b.__setitem__(m["off_xpr"] + r.randrange(0, 64),
                                r.randrange(0, 256))),
    ("disco no certificado",  lambda b, m, r: b.__setitem__(slice(284 + 168, 284 + 172),
                                struct.pack("<I", _u32(r)))),

    # As de cima quase sempre quebram o cabeçalho XPR, e aí a textura é
    # recusada antes de o decodificador rodar — medido: só 3% dos casos
    # chegavam a decodificar. Estas mexem nos PIXELS, deixando o cabeçalho
    # coerente, para o DXT1 e o ARGB serem realmente exercitados.
    ("bytes de pixel",        lambda b, m, r: _sujar_pixels(b, m, r)),
    ("cauda de pixel cortada", lambda b, m, r: b.__setitem__(
                                slice(m["off_secoes"] + 16, m["off_secoes"] + 20),
                                struct.pack("<I", r.randrange(28, m["tam_xpr"] + 1)))),
    ("expoente coerente",     lambda b, m, r: b.__setitem__(m["off_xpr"] + 27,
                                r.choice([4, 5, 6, 7, 8]))),
]


def _sujar_pixels(b, marcos, rnd):
    """Corrompe bytes da área de dados da textura, sem tocar no cabeçalho."""
    inicio = marcos["off_xpr"] + 28
    fim = marcos["off_xpr"] + marcos["tam_xpr"]
    for _ in range(rnd.choice([1, 8, 64, fim - inicio])):
        b[rnd.randrange(inicio, fim)] = rnd.randrange(0, 256)


def xbe_forjado(rnd):
    """XBE válido com uma ou duas mutações — e o nome das mutações aplicadas."""
    b, marcos = xbe_valido(expoente=rnd.choice([6, 7, 8]), formato=rnd.choice([6, 12]))
    escolhidas = rnd.sample(MUTACOES, rnd.choice([1, 1, 2]))
    for nome, aplicar in escolhidas:
        aplicar(b, marcos, rnd)
    return bytes(b), ", ".join(nome for nome, _ in escolhidas)


def casos_de_borda():
    """Combinações hostis conhecidas, enumeradas em vez de sorteadas.

    Busca aleatória é péssima para um ponto único num espaço grande: o
    estouro do decodificador ARGB só acontece com expoente 31 (2^31 x 2^31),
    e o gerador aleatório levava centenas de casos para cair nele. Aqui cada
    combinação roda uma vez, em segundos, e uma regressão é pega sempre.

    Devolve (bytes do XBE, descrição).
    """
    for formato, nome_formato in ((6, "ARGB"), (12, "DXT1")):
        # 30 e 31 cercam o ponto onde largura*altura*4 deixa de caber em usize;
        # 32 e 255 passam do que um u32 comporta; 0 e 1 são o degenerado.
        for expoente in (0, 1, 15, 16, 29, 30, 31, 32, 255):
            xbe, _ = xbe_valido(expoente=expoente, formato=formato)
            yield bytes(xbe), f"{nome_formato} 2^{expoente}"

    xbe, marcos = xbe_valido()
    for descricao, offset, valor in (
        ("file_size do XPR = 0", marcos["off_xpr"] + 4, 0),
        ("file_size do XPR = 0xFFFFFFFF", marcos["off_xpr"] + 4, 0xFFFF_FFFF),
        ("header_size maior que file_size", marcos["off_xpr"] + 8, 0xFFFF_FFFF),
        ("endereço base = 0xFFFFFFFF", 260, 0xFFFF_FFFF),
        ("certificado além do arquivo", 280, 0xFFFF_FFFF),
        ("número de seções enorme", 0x11C, 0x000F_FFFF),
        ("tabela de seções fora do arquivo", 0x120, 0xFFFF_FFFF),
        ("RawSize da seção enorme", marcos["off_secoes"] + 16, 0xFFFF_FFFF),
        ("RawAddress da seção fora do arquivo", marcos["off_secoes"] + 12, 0xFFFF_FFFF),
    ):
        copia, _ = xbe_valido()
        copia[offset:offset + 4] = struct.pack("<I", valor)
        yield bytes(copia), descricao


def medir_cobertura(alvo, rnd, casos, tmp):
    """Classifica ONDE cada caso parou, em vez de caçar pânico.

    Existe porque o gerador já esteve quebrado sem ninguém notar: a área de
    pixel era pequena demais para a textura declarada, então TODA capa era
    recusada por "dados insuficientes" e o decodificador nunca rodava. Foram
    6.000 execuções que pareciam cobrir o decodificador e não cobriam nada.
    Um fuzzer que não chega na função não protege ela.
    """
    import collections
    tally = collections.Counter()
    for _ in range(casos):
        xbe, _ = xbe_forjado(rnd)
        raiz = bytearray(entrada_diretorio(200, len(xbe), 0x00, "default.xbe"))
        while len(raiz) % SETOR:
            raiz.append(0xFF)
        iso = os.path.join(tmp, "cobertura.iso")
        escrever_iso(iso, raiz, 100, len(raiz), [(200, xbe)],
                     DESLOCAMENTO_XGD3 + 200 * SETOR + max(len(xbe), SETOR))
        _, saida = rodar(alvo, ["info", iso], 60)
        if "Thumbnail" in saida:
            tally["decodificou a textura"] += 1
        elif "Title ID" in saida:
            tally["leu o certificado, recusou a textura"] += 1
        else:
            tally["parou antes do certificado"] += 1

    print(f"\ncobertura em {casos} casos:")
    for rotulo, n in tally.most_common():
        print(f"  {n:5}  ({100 * n / casos:4.1f}%)  {rotulo}")
    decodificou = tally["decodificou a textura"]
    if decodificou < casos // 10:
        print("\n  ATENÇÃO: menos de 10% chega ao decodificador — o gerador provavelmente"
              "\n  está produzindo texturas que são recusadas antes da decodificação.")
        return 1
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("casos", nargs="?", type=int, default=200)
    ap.add_argument("--semente", type=int, default=None, help="repete uma rodada anterior")
    ap.add_argument("--cobertura", action="store_true",
                    help="mede onde os casos param, em vez de caçar pânico")
    ap.add_argument("--bordas", action="store_true",
                    help="enumera as combinações hostis conhecidas em vez de sortear")
    ap.add_argument("--bin", dest="alvo", default=None)
    ap.add_argument("--achados", default=os.path.join(os.path.dirname(os.path.abspath(__file__)), "achados"))
    args = ap.parse_args()

    alvo = binario(args.alvo)
    semente = args.semente if args.semente is not None else random.randrange(1 << 30)
    rnd = random.Random(semente)
    print(f"alvo: {alvo}\nsemente: {semente} (repita com --semente {semente})")

    tmp = tempfile.mkdtemp(prefix="iso2god-xbe-")
    if args.cobertura:
        try:
            return medir_cobertura(alvo, rnd, args.casos, tmp)
        finally:
            shutil.rmtree(tmp, ignore_errors=True)

    achados = []
    executados = 0
    setor_raiz, setor_xbe = 100, 200
    try:
        fonte = list(casos_de_borda()) if args.bordas else None
        total = len(fonte) if fonte else args.casos
        if fonte:
            print(f"modo bordas: {total} combinações conhecidas")
        for numero in range(total):
            xbe, mutacoes = fonte[numero] if fonte else xbe_forjado(rnd)
            raiz = bytearray(entrada_diretorio(setor_xbe, len(xbe), 0x00, "default.xbe"))
            while len(raiz) % SETOR:
                raiz.append(0xFF)

            iso = os.path.join(tmp, "caso.iso")
            destino = os.path.join(tmp, "saida")
            escrever_iso(iso, raiz, setor_raiz, len(raiz), [(setor_xbe, xbe)],
                         DESLOCAMENTO_XGD3 + setor_xbe * SETOR + max(len(xbe), SETOR))

            for argumentos, descricao in (
                (["info", iso], "info"),
                (["converter", iso, destino, "--padding", "nenhuma"], "converter"),
            ):
                shutil.rmtree(destino, ignore_errors=True)
                codigo, saida = rodar(alvo, argumentos, limite_segundos=60)
                executados += 1
                if e_achado(codigo, saida):
                    arquivo = guardar_achado(iso, args.achados, len(achados) + 1)
                    achados.append((numero, f"{descricao} [{mutacoes}]", codigo, detalhe(saida), arquivo))
                    print(f"  ! caso {numero}: {descricao} [{mutacoes}] -> {codigo} {detalhe(saida)}")
            shutil.rmtree(destino, ignore_errors=True)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    return relatar(executados, achados)


if __name__ == "__main__":
    sys.exit(main())
