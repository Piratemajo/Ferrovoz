#!/usr/bin/env bash
# Compila los binarios de Linux x64 (cliente con audio y servidor sin audio) y los deja
# en mod/natives-prebuilt/natives/linux-x86_64/, para meterlos en el jar.
#
# En Windows se ejecuta dentro de WSL (lo hace tools/build-windows.ps1). La primera vez
# hace falta, dentro de WSL (Ubuntu):
#   sudo apt update && sudo apt install -y build-essential cmake pkg-config libasound2-dev
#   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/native"

missing=()
command -v cargo >/dev/null || missing+=("Rust (https://rustup.rs)")
command -v cmake >/dev/null || missing+=("cmake")
command -v cc >/dev/null || missing+=("build-essential")
pkg-config --exists alsa 2>/dev/null || missing+=("libasound2-dev (y pkg-config)")
if (( ${#missing[@]} )); then
    echo "Falta: ${missing[*]}"
    echo "En Ubuntu/WSL: sudo apt install -y build-essential cmake pkg-config libasound2-dev"
    echo "y Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

# Desde WSL, compilar dentro de /mnt/c es muy lento: usamos una carpeta de Linux para
# lo compilado (se reutiliza entre compilaciones).
target="${FERROVOZ_TARGET_DIR:-$HOME/.cache/ferrovoz-target}"
echo "== Linux x64: cliente (con audio)"
cargo build --release --target-dir "$target/client"
echo "== Linux x64: servidor (sin audio)"
cargo build --release --no-default-features --target-dir "$target/server"

out="$root/mod/natives-prebuilt/natives/linux-x86_64"
mkdir -p "$out"
cp "$target/client/release/libferrovoz.so" "$out/libferrovoz.so"
cp "$target/server/release/libferrovoz.so" "$out/libferrovoz_server.so"
# El de servidor no puede depender de ALSA (los servidores no tienen tarjeta de sonido).
if ldd "$out/libferrovoz_server.so" | grep -q asound; then
    echo "ERROR: libferrovoz_server.so depende de ALSA"; exit 1
fi
echo "Listo: $out"
ls -la "$out"
