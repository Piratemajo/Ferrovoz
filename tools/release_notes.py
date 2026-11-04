#!/usr/bin/env python3
"""Saca de CHANGELOG.md la sección de una versión (para las notas de la release).

Uso: python3 tools/release_notes.py 0.7.0 > notas.md
Falla si esa versión no tiene sección: así no se publica nada sin notas.
"""
import re
import sys

version = sys.argv[1]
text = open("CHANGELOG.md", encoding="utf-8").read()
m = re.search(r"^## \[" + re.escape(version) + r"\][^\n]*\n(.*?)(?=^## \[|\Z)", text, re.S | re.M)
if not m or not m.group(1).strip():
    sys.exit(f"CHANGELOG.md no tiene la sección «## [{version}]». Añádela antes de publicar.")
print(m.group(1).strip())
