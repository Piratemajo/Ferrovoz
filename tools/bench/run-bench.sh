#!/usr/bin/env bash
# Benchmark de Ferrovoz: tres pasadas seguidas en un servidor de Fabric de pruebas.
# Pon este archivo y bench_clients en la carpeta del servidor y ejecútalo con
# `bash run-bench.sh`. Cada pasada carga chunks y apaga el servidor al terminar: no
# lo uses en el de verdad.
set -u

SERVER_JAR="${SERVER_JAR:-server.jar}"
MEMORY="${MEMORY:-4G}"
SECONDS_MEASURED="${SECONDS_MEASURED:-120}"

cd "$(dirname "$0")"
[ -f "$SERVER_JAR" ] || { echo "No encuentro $SERVER_JAR (SERVER_JAR=... bash run-bench.sh)"; exit 1; }
[ -f bench_clients ] || { echo "Falta bench_clients junto a este archivo (viene en bench-clients-linux-* del CI)"; exit 1; }
chmod +x bench_clients
grep -q '^pause-when-empty-seconds=0' server.properties 2>/dev/null || {
    echo "Pon pause-when-empty-seconds=0 en server.properties: si no, el servidor deja de hacer ticks sin jugadores."
    exit 1
}
mkdir -p ferrovoz-bench-results

run() { # nombre jugadores colocación %hablando [hilos que reciben]
    local name=$1 players=$2 layout=$3 talking=$4 threads=${5:-}
    echo
    echo "=== $name: $players jugadores, $talking % hablando ==="
    rm -rf ferrovoz-bench
    ./bench_clients ferrovoz-bench &
    local clients=$!
    if [ -n "$threads" ]; then export FERROVOZ_RELAY_THREADS=$threads; else unset FERROVOZ_RELAY_THREADS; fi
    java "-Xms$MEMORY" "-Xmx$MEMORY" "-Dferrovoz.bench=$players" "-Dferrovoz.bench.layout=$layout" \
        "-Dferrovoz.bench.talking=$talking" "-Dferrovoz.bench.seconds=$SECONDS_MEASURED" -jar "$SERVER_JAR" nogui
    wait "$clients" # guardan lo recibido justo después de que pare el servidor
    mkdir -p "ferrovoz-bench-results/$name"
    for f in report.md result.properties clients.properties; do
        [ -f "ferrovoz-bench/$f" ] && cp "ferrovoz-bench/$f" "ferrovoz-bench-results/$name/"
    done
}

run normal       100 spread 20
run evento       250 crowd  60
run evento-1hilo 250 crowd  60 1

echo
echo "Listo. Los informes están en ferrovoz-bench-results: mándalos."
