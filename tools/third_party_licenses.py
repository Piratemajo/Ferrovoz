#!/usr/bin/env python3
"""Genera THIRD_PARTY_LICENSES.txt con todo el código de terceros que va dentro del jar.

Usa `cargo metadata` (todas las plataformas: Windows, macOS y Linux) para saber
qué crates de Rust acaban en las librerías nativas, y copia el texto de licencia
que trae cada uno. Añade además las librerías en C que se compilan dentro:
libopus (vía audiopus_sys) y RNNoise (portado en nnnoiseless).

Uso:  python3 tools/third_party_licenses.py   (necesita `cargo fetch` antes)
"""
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NATIVE = os.path.join(ROOT, "native")
OUT = os.path.join(ROOT, "mod", "src", "main", "resources", "META-INF", "THIRD_PARTY_LICENSES.txt")
LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)([-_.].*)?$", re.I)


# Las cinco plataformas que se compilan en el CI (ver .github/workflows/build.yml).
TARGETS = [
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
]

MIT_TEXT = """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE."""


def metadata(target):
    out = subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=NATIVE, text=True)
    return json.loads(out)


def shipped_packages(meta):
    """Paquetes alcanzables desde ferrovoz-native por dependencias normales (no build ni dev)."""
    pkgs = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, stack = set(), [root]
    while stack:
        pid = stack.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]["deps"]:
            # kind None = dependencia normal (las de build.rs y tests no van en el binario).
            if any(k["kind"] is None for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    seen.discard(root)
    return sorted((pkgs[i] for i in seen), key=lambda p: (p["name"], p["version"]))


def license_texts(pkg_dir):
    texts = []
    for name in sorted(os.listdir(pkg_dir)):
        path = os.path.join(pkg_dir, name)
        if os.path.isfile(path) and LICENSE_FILE.match(name):
            with open(path, encoding="utf-8", errors="replace") as f:
                texts.append((name, f.read().strip()))
    return texts


def main():
    found = {}
    for target in TARGETS:
        for p in shipped_packages(metadata(target)):
            found[p["id"]] = p
    packages = sorted(found.values(), key=lambda p: (p["name"], p["version"]))
    # Agrupamos los crates que traen exactamente el mismo texto (p. ej. MIT idénticos)
    # para que el archivo no tenga 80 copias de lo mismo.
    groups = {}
    missing = []
    for p in packages:
        texts = license_texts(os.path.dirname(p["manifest_path"]))
        if not texts:
            # Crates de un mismo repositorio (p. ej. sonora y sonora-aec3) comparten
            # licencia y copyright: si otro de ese repo trae el archivo, se usa ese.
            repo = p.get("repository")
            for other in packages:
                if other is not p and repo and other.get("repository") == repo and other.get("license") == p.get("license"):
                    texts = license_texts(os.path.dirname(other["manifest_path"]))
                    if texts:
                        break
        if not texts:
            missing.append(p)
            continue
        key = "\n\n".join(f"--- {n} ---\n{t}" for n, t in texts)
        groups.setdefault(key, []).append(p)

    lines = []
    w = lines.append
    w("Ferrovoz — licencias de terceros / third-party licenses")
    w("=" * 60)
    w("")
    w("Ferrovoz (c) Piratemajo, licencia MIT (ver LICENSE_ferrovoz).")
    w("Las librerías nativas de este jar incluyen el siguiente código de terceros.")
    w("The native libraries in this jar include the following third-party code.")
    w("")

    # Librerías en C compiladas dentro del binario.
    cargo_home = os.environ.get("CARGO_HOME", os.path.expanduser("~/.cargo"))
    by_name = {p["name"]: p for p in packages}
    w("-" * 60)
    w("Opus audio codec (libopus), compilado dentro de audiopus_sys")
    w("https://opus-codec.org/  —  BSD-3-Clause")
    w("-" * 60)
    apus = by_name.get("audiopus_sys")
    copying = os.path.join(os.path.dirname(apus["manifest_path"]), "opus", "COPYING") if apus else None
    if not copying or not os.path.exists(copying):
        sys.exit("No encuentro opus/COPYING de audiopus_sys: ejecuta `cargo fetch` en native/")
    w(open(copying, encoding="utf-8").read().strip())
    w("")
    w("-" * 60)
    w("RNNoise (Xiph.Org / Mozilla), red neuronal y algoritmo portados a Rust por nnnoiseless")
    w("https://github.com/xiph/rnnoise  —  BSD-3-Clause (texto en el bloque de nnnoiseless, abajo)")
    w("-" * 60)
    w("")

    w("=" * 60)
    w(f"Crates de Rust ({len(packages)})")
    w("=" * 60)
    w("")
    for key, pkgs in sorted(groups.items(), key=lambda kv: kv[1][0]["name"]):
        w("-" * 60)
        for p in pkgs:
            repo = p.get("repository") or p.get("homepage") or ""
            w(f"{p['name']} {p['version']}  ({p.get('license') or 'ver texto'})  {repo}".rstrip())
        w("-" * 60)
        w(key)
        w("")
    for p in missing:
        w("-" * 60)
        w(f"{p['name']} {p['version']}  ({p.get('license')})  {p.get('repository') or ''}".rstrip())
        w("-" * 60)
        lic = p.get("license") or ""
        if "MIT" in lic:
            authors = ", ".join(a.split("<")[0].strip() for a in p.get("authors") or []) or f"the {p['name']} authors"
            w(f"El crate no trae archivo de licencia; se usa bajo MIT ({lic}):")
            w("")
            w(f"Copyright (c) {authors}")
            w("")
            w(MIT_TEXT)
        else:
            w(f"El crate no trae archivo de licencia; se distribuye bajo {lic}.")
        w("")

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8") as f:
        f.write("\n".join(lines).rstrip() + "\n")
    licenses = sorted({p.get("license") or "?" for p in packages})
    print(f"{OUT}: {len(packages)} crates, {len(groups)} textos distintos, sin archivo: {[p['name'] for p in missing]}")
    print("Licencias:", "; ".join(licenses))


if __name__ == "__main__":
    main()
