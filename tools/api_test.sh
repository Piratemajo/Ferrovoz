#!/usr/bin/env bash
# Prueba la API de Java (dev.ferrovoz.api) contra la librería nativa real, sin
# Minecraft. Necesita Java 21+ y Rust.
#
#   tools/api_test.sh                    compila native/ sin audio y prueba
#   FERROVOZ_LIB=ruta tools/api_test.sh  con una librería ya compilada
#   SLF4J_JAR=ruta                       sin descargar slf4j-api
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

lib="${FERROVOZ_LIB:-}"
if [ -z "$lib" ]; then
    (cd "$root/native" && cargo build --no-default-features --target-dir target/api-test)
    for f in libferrovoz.so libferrovoz.dylib ferrovoz.dll; do
        if [ -f "$root/native/target/api-test/debug/$f" ]; then lib="$root/native/target/api-test/debug/$f"; break; fi
    done
fi
[ -f "$lib" ] || { echo "No encuentro la librería nativa ($lib)"; exit 2; }

slf4j="${SLF4J_JAR:-}"
if [ -z "$slf4j" ]; then
    slf4j="$work/slf4j-api.jar"
    curl -fsSL -o "$slf4j" https://repo1.maven.org/maven2/org/slf4j/slf4j-api/2.0.17/slf4j-api-2.0.17.jar
fi

# La API y el código común que no toca Minecraft ni las APIs de las plataformas.
shared=$(grep -L -E '^import (net\.|org\.bukkit|com\.velocitypowered|com\.mojang|net\.md_5)' "$root"/mod/src/main/java/dev/ferrovoz/*.java)
javac --release 21 -Xlint:all,-serial -Werror -encoding UTF-8 -cp "$slf4j" -d "$work/classes" \
    $(find "$root/mod/src/api/java" -name '*.java') $shared \
    "$root/tools/api-test/dev/ferrovoz/ApiSelfTest.java"
java -cp "$work/classes:$root/mod/src/main/resources:$slf4j" -Dferrovoz.native.path="$lib" dev.ferrovoz.ApiSelfTest
