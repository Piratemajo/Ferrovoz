# Benchmark en un servidor real

Mide lo que Ferrovoz cuesta a un servidor de **Fabric** de verdad: tiempo de tick, CPU del relé, memoria y voz entregada. Hace tres pasadas seguidas, de unos 3-4 minutos cada una:

| Pasada | Jugadores | Colocación | Hablando | Para qué |
|---|---|---|---|---|
| `normal` | 100 | repartidos en grupos de 5 | 20 % | un servidor normal |
| `evento` | 250 | todos juntos | 60 % | un evento grande |
| `evento-1hilo` | 250 | todos juntos | 60 % | lo mismo con un solo hilo recibiendo, para ver cuánto aportan los demás |

Los jugadores son virtuales: un programa aparte (`bench_clients`) hace de clientes y manda voz cifrada por UDP, como un cliente de verdad.

## Qué hace falta

1. Un **servidor de Fabric de pruebas** con Ferrovoz, no el de verdad: carga chunks y se apaga solo al terminar cada pasada.
2. En su `server.properties`, `pause-when-empty-seconds=0`.
3. **Java 21+** y unos 4 GB de RAM libres para el servidor.
4. El programa de los clientes para el sistema del servidor. Lo compila el CI: en la ejecución de GitHub Actions, descarga `bench-clients-windows-x86_64`, `bench-clients-linux-x86_64` o `bench-clients-linux-aarch64`. Lleva el programa y estos scripts.

## Cómo lanzarlo

Copia el contenido del artefacto en la carpeta del servidor (junto a `server.jar`) y:

- **Windows**: doble clic en `run-bench.bat`. Si el jar no se llama `server.jar`, cambia `SERVER_JAR` al principio.
- **Linux**: `bash run-bench.sh`. Otro nombre de jar: `SERVER_JAR=fabric-server-launch.jar bash run-bench.sh`.

Los clientes y el servidor corren en la misma máquina a propósito: el evento mueve unos 300 000 paquetes por segundo, y por una red de casa se mediría el router y no Ferrovoz. Los clientes gastan algo de CPU, así que los números son un poco peores que en la realidad.

Al acabar, cada pasada deja su informe en `ferrovoz-bench-results/<pasada>/report.md`.
