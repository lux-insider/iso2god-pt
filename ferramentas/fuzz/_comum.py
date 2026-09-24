"""Peças compartilhadas pelos dois fuzzers.

O alvo padrão é o binário de **debug**, e isso é deliberado: o build de
release faz a aritmética dar a volta em silêncio, enquanto o de debug aborta
com "attempt to multiply with overflow". Foi exatamente assim que apareceu o
estouro no decodificador de thumbnail — em release ele viraria um acesso
fora dos limites bem mais adiante, muito mais difícil de ligar à causa.
"""

import os
import shutil
import struct
import subprocess
import sys

SETOR = 2048
BASE_ASSINATURA = 32 * SETOR
DESLOCAMENTO_XGD3 = 34_078_720
ASSINATURA = b"MICROSOFT*XBOX*MEDIA"

RAIZ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BINARIO_PADRAO = os.path.join(RAIZ, "target", "debug", "iso2god")


def binario(escolhido=None):
    caminho = escolhido or BINARIO_PADRAO
    if not os.path.exists(caminho):
        sys.exit(
            f"binário não encontrado: {caminho}\n"
            "compile antes com `cargo build` (debug, para pegar estouro aritmético)"
        )
    return caminho


def entrada_diretorio(setor, tamanho, atributos, nome):
    """Uma entrada de diretório GDF serializada, com o alinhamento de 4 bytes."""
    if isinstance(nome, str):
        nome = nome.encode()
    b = bytearray()
    b += struct.pack("<HH", 0, 1)          # ponteiros de subárvore
    b += struct.pack("<II", setor, tamanho)
    b.append(atributos)
    b.append(len(nome))
    b += nome
    while len(b) % 4:
        b.append(0xFF)
    return bytes(b)


def escrever_iso(caminho, bloco_raiz, setor_raiz, tamanho_raiz, conteudos, tamanho_total,
                 deslocamento=DESLOCAMENTO_XGD3):
    """Monta uma imagem com descritor de volume válido e o conteúdo informado.

    `conteudos` é uma lista de (setor, bytes) gravados dentro do volume.
    """
    descritor = ASSINATURA + struct.pack("<II", setor_raiz, tamanho_raiz) + b"TESTDATA"
    with open(caminho, "wb") as f:
        f.truncate(tamanho_total)
        f.seek(BASE_ASSINATURA + deslocamento)
        f.write(descritor)
        f.seek(deslocamento + setor_raiz * SETOR)
        f.write(bytes(bloco_raiz))
        for setor, dados in conteudos:
            f.seek(deslocamento + setor * SETOR)
            f.write(dados)


def rodar(binario_alvo, argumentos, limite_segundos=120):
    """Devolve (código, saída). O código vira "timeout" quando estoura o limite."""
    try:
        p = subprocess.run(
            [binario_alvo] + argumentos,
            capture_output=True, timeout=limite_segundos, text=True, errors="replace",
        )
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired:
        return "timeout", ""


def e_achado(codigo, saida):
    """Pânico (101), morte por sinal, ou travamento — tudo que não é erro tratado.

    Um erro de verdade sai com 1 e uma mensagem em português; é o
    comportamento esperado para entrada corrompida e NÃO é achado.
    """
    return codigo == "timeout" or codigo == 101 or (isinstance(codigo, int) and codigo < 0) \
        or "panicked" in saida


def detalhe(saida):
    for linha in saida.splitlines():
        limpa = linha.strip()
        if limpa.startswith(("attempt", "range", "index", "capacity", "memory", "slice")):
            return limpa
    for linha in saida.splitlines():
        if "panicked" in linha:
            return linha.strip()
    return ""


def guardar_achado(iso, pasta_achados, numero):
    os.makedirs(pasta_achados, exist_ok=True)
    destino = os.path.join(pasta_achados, f"achado_{numero:03}.iso")
    shutil.copy(iso, destino)
    return destino


def relatar(casos, achados):
    print(f"\ncasos executados: {casos}")
    print(f"achados (pânico/travamento): {len(achados)}")
    vistos = set()
    for numero, descricao, codigo, det, arquivo in achados:
        chave = (det, descricao)
        if chave in vistos:
            continue
        vistos.add(chave)
        print(f"  - caso {numero}: {descricao}  [{codigo}]")
        if det:
            print(f"      {det}")
        print(f"      guardado em {arquivo}")
    return 1 if achados else 0
