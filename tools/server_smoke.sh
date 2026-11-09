#!/usr/bin/env bash
# Arranca un servidor dedicado de verdad con el jar de Ferrovoz y Fabric API, espera a
# que esté listo, lo para y comprueba en el log que la voz ha arrancado.
#
#   tools/server_smoke.sh <fabric|quilt> <versión de Minecraft> <versión de Fabric API> <jar>
#
# Necesita Java y conexión (descarga el instalador, Minecraft y Fabric API). Lo usa el CI.
set -euo pipefail

LOADER=$1 MC=$2 FAPI=$3 JAR=$(realpath "$4")
DIR=${SMOKE_DIR:-smoke-$LOADER-$MC}
TIMEOUT=${SMOKE_TIMEOUT:-600}

latest() { # última versión publicada en un maven-metadata.xml
    curl -fsSL "$1/maven-metadata.xml" | sed -n 's:.*<release>\(.*\)</release>.*:\1:p'
}

rm -rf "$DIR" && mkdir -p "$DIR/mods"
cd "$DIR"

case "$LOADER" in
    fabric)
        base=https://maven.fabricmc.net/net/fabricmc/fabric-installer
        v=$(latest "$base")
        curl -fsSL -o installer.jar "$base/$v/fabric-installer-$v.jar"
        java -jar installer.jar server -mcversion "$MC" -downloadMinecraft -dir .
        launch=fabric-server-launch.jar
        ;;
    quilt)
        base=https://maven.quiltmc.org/repository/release/org/quiltmc/quilt-installer
        v=$(latest "$base")
        curl -fsSL -o installer.jar "$base/$v/quilt-installer-$v.jar"
        java -jar installer.jar install server "$MC" --download-server --install-dir=.
        launch=quilt-server-launch.jar
        ;;
    *) echo "Loader desconocido: $LOADER (fabric o quilt)"; exit 2 ;;
esac
[ -f "$launch" ] || { echo "El instalador no ha dejado $launch"; ls -la; exit 1; }

curl -fsSL -o "mods/fabric-api-$FAPI.jar" \
    "https://maven.fabricmc.net/net/fabricmc/fabric-api/fabric-api/$FAPI/fabric-api-$FAPI.jar"
cp "$JAR" mods/
echo "eula=true" > eula.txt
printf 'online-mode=false\nlevel-type=minecraft\\:flat\ngenerate-structures=false\nspawn-protection=0\n' > server.properties

# La consola lee de una tubería que dejamos abierta para poder mandarle "stop".
mkfifo console
exec 3<>console
java -Xmx2G -jar "$launch" nogui <console > server.log 2>&1 &
pid=$!

ready=
for _ in $(seq "$TIMEOUT"); do
    if grep -q 'Done (' server.log; then ready=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 1
done
if [ -n "$ready" ]; then
    # El relé arranca justo después de "Done".
    for _ in $(seq 30); do grep -q 'voice chat running' server.log && break; sleep 1; done
    sleep 2
    echo stop >&3
    for _ in $(seq 120); do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
fi
kill -0 "$pid" 2>/dev/null && { kill "$pid"; sleep 5; kill -9 "$pid" 2>/dev/null || true; }
exec 3>&-

fail=
check() { grep -q -- "$1" server.log || { echo "No aparece en el log: $1"; fail=1; }; }
[ -n "$ready" ] || { echo "El servidor no ha llegado a arrancar"; fail=1; }
check 'Ferrovoz native core loaded'
check "voice chat running ($( [ "$LOADER" = quilt ] && echo Quilt || echo Fabric ))"
check 'voice relay: UDP buffers'
if grep -nE 'Exception|Error' server.log | grep -i ferrovoz; then
    echo "Errores de Ferrovoz en el log"; fail=1
fi

if [ -n "$fail" ]; then
    echo "----- server.log -----"; cat server.log; exit 1
fi
grep -E 'Ferrovoz|ferrovoz|Done \(' server.log
echo "OK: $LOADER $MC"
