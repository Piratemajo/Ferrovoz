# Ferrovoz

Chat de voz de proximidad para **Minecraft 26.3** y **1.21.1** con el motor escrito en **Rust**. Por **Piratemajo**.
Un solo `.jar` para todo: mod de **Fabric**, **NeoForge** o **Forge** (cliente y servidor), plugin de **Paper / Purpur / Folia / Spigot** y plugin de proxy para **Velocity / BungeeCord** y derivados.
Java solo hace de pegamento con el juego; captura, códec, red, cifrado, mezcla y efectos viven en Rust.

## Funciones

**Para jugar**
- **Audio 3D para cascos**: la voz baja con la distancia y llega desde donde está quien habla: izquierda o derecha, **delante o detrás** y **arriba o abajo**, también según hacia dónde mires en vertical. Usa un modelo de cabeza y oreja (Brown y Duda) sin tablas de medidas: diferencia de tiempo entre oídos (hasta 0,65 ms), sombra de la cabeza, sombra trasera de la oreja (−4 dB de agudos detrás) y los ecos de la oreja que mueven una muesca de agudos con la altura. Cuesta un 0,4 % de un núcleo por cada persona que habla. Con altavoces se puede volver al estéreo simple (J → Audio 3D).
- **Susurrar (B) y gritar (G)**: alcance de 8 y 96 bloques (configurable) además del normal de 48.
- **Grupos**: `/voz grupo crear <nombre> [contraseña]`. Los miembros se oyen a cualquier distancia y dimensión, como una radio; el resto no los oye. Tienen **dueño** (puede expulsar, fijar y borrar), avisan por chat cuando alguien entra o sale, y se ven en el HUD (⚑, ★ si es permanente). Los **permanentes** (`/voz grupo fijar`) sobreviven a reinicios y sus miembros vuelven solos al entrar.
- **El sonido rodea**: si la línea recta está tapada pero hay un camino abierto (una puerta, una ventana, doblar una esquina), la voz llega también por ahí, más clara que a través del muro y **desde el hueco**. Cada esquina la baja y le quita agudos (los graves rodean mejor). Se busca con A* por los bloques donde pasa el sonido (puertas y trampillas abiertas, plantas, agua…) y después se "tira de la cuerda" para quedarse con las esquinas de verdad; como mucho 2000 bloques y un camino por tick, así que cuesta 1–2 ms y no se nota. Las puertas cerradas, el cristal y las vallas tapan la mitad que un bloque macizo.
- **Paredes que amortiguan**: con bloques sólidos entre medias la voz suena más baja y apagada (se filtran los agudos). Si quien habla está fuera de tu distancia de renderizado, las calcula el servidor (Fabric, Paper y Spigot; en Folia solo el cliente), con un coste fijo por tick: como mucho 300 líneas, las parejas más cercanas primero, y sin recalcular si nadie se ha movido de bloque. Con 250 personas juntas y 100 hablando: 0,2 ms por tick de media.
- **Multitudes y eventos**: a partir de unas 20 voces a la vez ya no se distingue a nadie, y mandarlas todas a todos satura el servidor. Si a una zona (cubos de 8 bloques) le llegan más de `max_voices` voces (24 por defecto), sus jugadores reciben por separado las 24 más cercanas, con audio 3D, paredes y eco, y el resto **mezclado en el servidor como el murmullo de la multitud**, que suena desde donde está el grueso de la gente: no desaparece nadie. La mezcla se hace por zona, no por jugador, así que cuesta poco aunque haya cientos. Quien tenga `ferrovoz.priority` (por defecto, operadores) **se oye siempre por separado**, aunque hable toda la plaza: para quien presenta el evento. Con menos voces que el límite no cambia nada, y funciona con clientes de cualquier versión.
- **Eco según la forma del sitio**: la voz resuena si tú **o quien habla** estáis en una cueva o sala cerrada. 42 rayos miden la sala: la duración del eco sale de su tamaño y sus materiales (Sabine: un cuarto de piedra ~1,1 s, una caverna hasta 3,5 s, una sala de lana 0,25 s), la lana y la madera además se comen los agudos, y los **primeros ecos** llegan de las paredes cercanas, desde su lado y con su retardo de ida y vuelta (una pared a 6 bloques: 35 ms). Al aire libre, sin eco. Y al hablar tú dentro, **oyes tu propio eco** (solo el rebote, no tu voz directa). Cada jugador lo puede desactivar en Ajustes (J), y el servidor para todos (`cave_echo=false`).
- **Voz bajo el agua**: si quien habla tiene la cabeza sumergida, o la tienes tú, la voz llega como bajo el agua: sin agudos, hueca, con el tono ondulando como el agua en movimiento y un leve burbujeo. Con los dos sumergidos, aún más tapada. Entra y sale poco a poco, sin chasquidos. También se puede desactivar en Ajustes (J). Lo de quien habla lo calcula el servidor con el juego, así que funciona aunque esté fuera de tu vista.
- **Volumen por jugador**: baja o silencia a alguien solo para ti (J → Jugadores…). Se guarda.
- **Volumen de grupo** aparte del de proximidad (J): la "radio" del grupo y quien tienes al lado, cada uno a su volumen.
- **Indicadores sobre la cabeza**: ♪ en quien habla y, en los demás, **✖ micro** (silenciado por él, por un admin o sin permiso), **✖ ensordecido** o **sin voz** (sin Ferrovoz, o con la voz desactivada). El servidor reparte cada segundo el estado de todos; cada cliente avisa del suyo al cambiar y cada 5 s. Los de estado solo se ven a menos de 24 bloques y solo en jugadores de la lista de jugadores (no en NPC de plugins). Con servidores anteriores a la 0.10 solo sale el ♪.
- **Cancelación de eco**: para quien juega con altavoces. Quita del micro lo que suena por ellos (las voces de los demás, tu eco en cuevas), así nadie se oye a sí mismo repetido. AEC3 de WebRTC portado a Rust (`sonora`). Medido: −46 a −62 dB de eco tras aprender ~1 s, tu voz pierde menos de 1,5 dB aunque hables a la vez, y con altavoces la activación por voz deja de dispararse con las voces de los demás (de 137 tramas reenviadas a 0). La prueba de micro no se cancela.
- **Ganancia automática**: sube un micro bajo y baja uno alto (±12 dB) para que tu voz llegue a un nivel normal. Va después de la supresión de ruido, mide tu voz como lo más fuerte que dices (percentil 95 de los últimos 15 s, así lo que se cuela de los cascos o el ruido no cambia la medida) y no se aplica en las pausas, así que no sube la música del juego que se cuela por los cascos ni el ruido de fondo.
- **No usar chat de voz** (J): ni oyes ni te oyen. El servidor deja de mandarte voz al momento; se recuerda para todos los servidores y se reactiva con el mismo botón.
- **Grupos de red** tras un proxy: `/vozred crear <nombre>` (o `/voicenet create`). Sus miembros se oyen aunque estén en servidores distintos de la red.
- **Icono ♪** fijo sobre la cabeza de quien habla, siempre mirando a la cámara (azul si es voz de grupo).

**Repeticiones (Replay Mod y Flashback, en Fabric)**
- Si grabas con **Replay Mod** o **Flashback**, la voz queda guardada en la repetición: la de todos los que oyes y la tuya. No hay que configurar nada.
- Al verla, cada voz sale **de su personaje** y suena **desde la cámara**: audio 3D, paredes, puertas y eco se calculan otra vez con el mundo de la repetición. Y los iconos ♪ salen sobre la cabeza de quien habla.
- Con **Flashback**, la voz **sale en los vídeos exportados**: Ferrovoz suena por el motor de sonido de Minecraft al ver repeticiones, que es lo que Flashback graba. (Replay Mod no exporta el sonido del juego en sus vídeos.)
- Cómo funciona: cada trama de voz (el audio Opus, sin cifrar, con quién habla) se mete en la grabación como un paquete más (`ferrovoz:replay_voice`), como si hubiera llegado del servidor. Ocupa poco (~5 KB/s por persona hablando). Ferrovoz se engancha a los dos mods por reflexión, así que no depende de ellos: si no están, no pasa nada, y si una versión suya cambia por dentro, el puente se desactiva con un aviso en el log, sin romper nada. Las repeticiones grabadas se ven igual sin Ferrovoz (solo que sin voz).

**Para que nadie se quede sin voz sin saber por qué**
- **Versión distinta**: si el cliente y el servidor tienen versiones de Ferrovoz incompatibles, el jugador recibe un mensaje claro («Este servidor usa Ferrovoz 0.7.0 y tú tienes 0.6.0: actualiza el mod») y el HUD lo indica. Funciona también con clientes y servidores anteriores a la 0.6. Versiones distintas con el mismo protocolo (0.5 y 0.6) son compatibles y funcionan sin aviso.
- **Plataforma sin soporte** (Linux de 32 bits, Alpine/musl…): el juego y el servidor arrancan con normalidad, solo que sin voz, y se avisa al jugador una vez.
- **Micrófono bloqueado**: si el micro manda silencio puro durante 3 s (lo típico cuando macOS no da permiso al launcher, o un cascos silenciado por hardware), sale un aviso con los pasos para arreglarlo.
- **Idiomas**: español, inglés, **portugués, francés y alemán**, en el juego (ajustes, HUD, avisos) y en los mensajes del servidor y del proxy, según el idioma de Minecraft de cada jugador (cualquier variante: `pt_br`, `fr_ca`, `de_at`…). El resto, en inglés. Comandos: `/voz grupo crear` (subcomandos en español) y `/voice group create` (en inglés, el que usan los demás idiomas). Para añadir otro idioma basta con un `messages_<idioma>.properties` y un `<idioma>.json`.

**Para administrar**
- **API para plugins y mods** (`dev.ferrovoz.api`), la misma en Fabric, NeoForge, Forge y Paper, en 26.3 y 1.21.1. Sirve para controlar la voz (silenciar, permisos, megáfono, silencio general, grupos, ajustes en marcha), recibir eventos (quién conecta, habla, calla o cambia de grupo, y cancelar crear grupos, unirse o silenciar), poner sonido con canales de audio (desde un punto, siguiendo a alguien, a unos jugadores o a un grupo, con 3D; WAV, PCM u Opus) y escuchar la voz para grabarla o hacer bots. Ver [docs/api-es.md](docs/api-es.md) (en inglés: [docs/api.md](docs/api.md)). El javadoc de la API está en inglés.
- **Moderación**: `/voz silenciar <jugador>`, `/voz desilenciar <jugador>` y `/voz estado`. Los silencios se **guardan en disco**: siguen aunque el jugador salga o el servidor se reinicie.
- **Espectadores y muertos**: cada servidor elige qué oyen y a quién se oye (`spectators` y `dead` en `ferrovoz.properties`):

  | Regla | Les oyen | Oyen |
  |---|---|---|
  | `normal` (por defecto para los muertos) | todos | a todos |
  | `listen` | nadie | a los vivos |
  | `ghost` (por defecto para los espectadores) | solo otros espectadores/muertos con `ghost` u `own` | a los vivos y a esos "fantasmas" |
  | `own` | solo los "fantasmas" | solo a los "fantasmas" |
  | `off` | nadie | nadie |

  Por defecto, **los espectadores no se oyen desde la partida**: hablan entre ellos y oyen a los que juegan, sin molestarles. Con `spectators=normal` se les oye como a cualquiera, con `spectators=listen` solo escuchan, o `dead=ghost` para que los muertos hablen entre ellos mientras siguen la partida. La distancia sigue contando, y la regla se aplica también a los grupos.
- **Permisos** compatibles con LuckPerms, **iguales en Fabric y Paper**:

  | Nodo | Por defecto | Para qué |
  |---|---|---|
  | `ferrovoz.speak` | todos | hablar |
  | `ferrovoz.shout` | todos | gritar (sin él, el grito es voz normal) |
  | `ferrovoz.groups` | todos | usar grupos |
  | `ferrovoz.admin` | operadores | `/voz estado`, silenciar, y expulsar/fijar/borrar en cualquier grupo |
  | `ferrovoz.priority` | operadores | en una multitud, que se le oiga siempre por separado (el anfitrión de un evento) |

  En Fabric se usa la API de permisos de Fabric con `ferrovoz:speak`…, y LuckPerms lo traduce a `ferrovoz.speak` (comprobado en su código fuente). En NeoForge y Forge, sus API de permisos con los mismos nodos. Se reconsultan cada 5 s: los cambios se aplican en caliente.

**Técnicas**
- **Sin puerto UDP**: si el UDP no llega en 3 s (host que no deja abrir puertos), la voz pasa sola por la conexión del juego (*túnel*). Se envía desde un hilo propio cada 5 ms (no al ritmo de los ticks), así que solo añade la espera de recepción del servidor. Se puede forzar con `transport=tunnel`.
- **Seguridad**: cada paquete lleva un contador y los repetidos se rechazan (*anti-replay*, como WireGuard): nadie puede reenviar tu voz ni desviarla. Límite de tráfico por jugador y por IP contra inundaciones.
- **Calidad automática**: el servidor mide la pérdida de paquetes de cada jugador y el cliente ajusta Opus: 32 kbps en buena red, hasta 14 kbps con más protección si se pierden paquetes.
- **Retraso según tu red**: antes de reproducir cada frase se guarda un pequeño colchón, y su tamaño se ajusta a la conexión de cada persona. Se mide con qué retraso llegan sus paquetes y se espera lo justo para cubrir el 97 %: con fibra o cable, 40 ms; con Wi-Fi malo o datos móviles, hasta 160 ms para que no se corte. Si un paquete llega tarde a media frase se espera por él en vez de tirarlo. En una red simulada con ±120 ms de variación los cortes bajan del 14 % al 0 %, y con buena red el retraso baja de ~60 a ~40 ms.
- **Opus** con FEC, **ChaCha20-Poly1305** por jugador, **jitter buffer** de 60 ms.
- **Limpieza del micro**, en este orden: cancelación de eco (AEC3 de WebRTC, en Rust) → paso alto a 80 Hz + RNNoise (Rust puro) + puerta de ruido → ganancia automática. Medido: ventilador −63 dB, zumbido de 50 Hz −38 dB, siseo −26 dB en la trama típica.
- El servidor **no decodifica audio**: reenvía bytes Opus, así que aguanta muchos jugadores con muy poca CPU. La excepción son las multitudes: ahí decodifica las voces lejanas una vez y mezcla un murmullo por zona.

## Cómo funciona

```
 CLIENTE (Rust)                                          SERVIDOR (Rust)
 micro ─► limpieza ─► PTT/VAD ─► Opus (bitrate adapt.) ─┐
                                                        ├─ UDP cifrado ─► relé: proximidad (susurro/normal/grito)
 altavoz ◄─ eco ◄─ 3D/grupo ◄─ paredes ◄─ vol. jugador ◄┤   o túnel        o grupo; silencios y permisos;
                                         ▲               │  por el juego    mide la pérdida de cada jugador
 Java cliente: teclas, HUD, pantallas, ──┘ bloques entre jugadores, forma de la sala, iconos ♪
 Java servidor (Fabric o plugin): entrar/salir, posiciones, permisos, /voz, mover el túnel
```

## Estructura

| Ruta | Qué hay |
|---|---|
| `native/src/protocol.rs` | Formato de paquetes y cifrado |
| `native/src/server.rs` | Relé: alcances, grupos, silencios, permisos, túnel, medición de pérdida |
| `native/src/client.rs` | Cliente: transporte UDP/túnel, bitrate adaptativo, mezcla con paredes, eco y volumen por jugador |
| `native/src/codec.rs` · `denoise.rs` · `reverb.rs` · `underwater.rs` | Opus · limpieza del micro · eco · voz bajo el agua |
| `native/src/jitter.rs` · `spatial.rs` · `resample.rs` | Jitter buffer · audio 3D · remuestreo |
| `native/src/audio.rs` | Micro y altavoces con cpal (WASAPI / CoreAudio / ALSA) |
| `native/src/forwarder.rs` | Reenviador de voz para proxies |
| `native/src/lib.rs` | Frontera JNI (68 funciones) |
| `native/tests/loopback.rs` | Pruebas de extremo a extremo por red real |
| `native/tests/load.rs` | Prueba de carga del relé con 50-100 clientes falsos |
| `native/examples/bench_clients.rs` + `mod/.../Bench.java` | Benchmark en un servidor de Fabric real |
| `tools/third_party_licenses.py` | Genera el archivo de licencias de terceros |
| `mod/src/api` | API para plugins y mods (`dev.ferrovoz.api`, Java 21, sin Minecraft); su implementación es `ApiCore` en `mod/src/main` |
| `tools/api_test.sh` | Prueba la API de Java contra la librería nativa real |
| `mod/src/main` | Lado servidor común a Fabric, NeoForge y Forge (`Ferrovoz`, `/voz`, `Messages`, `TunnelPump`, `WallTracer`, protocolo y versión, configuración) |
| `mod/src/main/resources/ferrovoz/lang` | Mensajes del servidor por idioma |
| `mod/src/client` | Cliente común a Fabric, NeoForge y Forge: teclas, HUD, pantallas, iconos, sensor del mundo |
| `mod/src/main/.../fabric` · `mod/src/client/.../fabric` | Fabric: solo conecta sus eventos, red y teclas con el núcleo |
| `mod/src/neoforge` | NeoForge (mismo jar) |
| `mod/src/forge` | Forge (mismo jar) |
| `mod/src/bukkit` | Plugin de Paper/Purpur/Folia/Spigot (mismo jar) |
| `mod/src/proxy` | Plugin de Velocity y BungeeCord (mismo jar) |
| `mod-1.21.1/` | Minecraft 1.21.1: solo la capa que habla con el juego; el resto lo coge de `mod/` (ver [Minecraft 1.21.1](#minecraft-1211)) |

## Compilar

Necesitas **Rust** (stable), **JDK 25** y, en Linux, `libasound2-dev` y `cmake`.

```bash
cd native && cargo test          # 154 tests: unitarios + extremo a extremo por red (incluidos ataques)
cd ../mod && ./gradlew build     # compila Rust (con y sin audio) y lo mete en el .jar
```

En Windows, `gradlew.bat build`. El jar sale en `mod/build/libs/ferrovoz-<Minecraft>-<versión>.jar`, y la API para otros plugins en `ferrovoz-api-<versión>.jar` (con `-sources` y `-javadoc`).

**Lo normal es dejar que lo compile GitHub:** el repo es público, así que Actions es gratis en todos los sistemas. Cada push a `main` (y cada pull request) pasa los tests en las 5 plataformas y deja los **jars** (el de 26.3 y el de 1.21.1) en la página de la ejecución (Actions → la ejecución → *Artifacts* → `ferrovoz-mod`). Los cambios que solo tocan `.md` o `docs/` no lo lanzan.

**El jar local solo lleva los binarios de tu sistema.** Si lo compilas en Windows, no tendrá voz en un servidor Linux (casi todos los hostings) ni en un Mac. Sirve para probar rápido; para meter más plataformas, pon sus dos archivos en `mod/natives-prebuilt/natives/<plataforma>/` antes de `./gradlew build`:

| Plataforma | Carpeta | Archivos |
|---|---|---|
| Windows x64 | `windows-x86_64` | `ferrovoz.dll`, `ferrovoz_server.dll` |
| Linux x64 | `linux-x86_64` | `libferrovoz.so`, `libferrovoz_server.so` |
| Linux ARM64 | `linux-aarch64` | `libferrovoz.so`, `libferrovoz_server.so` |
| macOS Apple Silicon | `macos-aarch64` | `libferrovoz.dylib`, `libferrovoz_server.dylib` |
| macOS Intel | `macos-x86_64` | `libferrovoz.dylib`, `libferrovoz_server.dylib` |

En cada sistema, desde `native/`: `cargo build --release` (cliente, con audio: es `target/release/<archivo>`) y `cargo build --release --no-default-features --target-dir target/server` (servidor, sin audio: `target/server/release/<archivo>`, renombrado con `_server`).

### Windows + Linux + Mac en tu ordenador (Windows)

1. **Una vez:** instala Rust, JDK 25 y [CMake](https://cmake.org/download/) en Windows. Para Linux, instala WSL (`wsl --install` en PowerShell como administrador, reinicia) y dentro de Ubuntu:
   ```bash
   sudo apt update && sudo apt install -y build-essential cmake pkg-config libasound2-dev
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
2. **Mac:** de cualquier ejecución de Actions, o con *Run workflow* → plataformas: **macos** (más rápido, sin tests). Descarga los artefactos de Mac y ponlos en `mod/natives-prebuilt/natives/macos-*/` (el artefacto `natives-macos` ya viene con esa estructura: descomprímelo en `mod/natives-prebuilt/`). Solo hace falta repetirlo cuando cambie el código Rust.
3. **Todo lo demás**, desde la carpeta del proyecto en PowerShell:
   ```powershell
   powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1
   ```
   Pasa los tests, compila Windows, compila Linux dentro de WSL (`tools/build-linux.sh`), y hace el jar con todo lo que haya en `mod/natives-prebuilt/`. Al final dice qué plataformas lleva y avisa de las que faltan. `-SinLinux` y `-SinTests` para ir más rápido.

### Minecraft 1.21.1

`mod-1.21.1/` es un proyecto de Gradle aparte que comparte con `mod/` todo lo que no depende de la versión del juego: el núcleo en Rust, el protocolo, la configuración, `/voz` y sus mensajes, el plugin de Paper y el de proxy. Solo tiene su propia versión de la capa que toca Minecraft (eventos, red, HUD, pantallas, iconos, teclas). La versión del mod es siempre la de `mod/gradle.properties`.

Se compila con **JDK 21** (el de Minecraft 1.21.1):

```bash
cd mod-1.21.1 && ./gradlew build
```

Sale un solo jar, `mod-1.21.1/build/libs/ferrovoz-1.21.1-<versión>.jar`, igual que en 26.3: Fabric, NeoForge 21.1, Forge 52, Paper/Spigot/Purpur/Folia y Velocity/BungeeCord. Como en 1.21.1 Fabric usa los nombres ofuscados de Minecraft y NeoForge/Forge los de Mojang, el jar lleva dos copias de las clases que tocan el juego: la de Mojang en `dev.ferrovoz` y la de Fabric en `dev.ferrovoz.intermediary` (la tarea `relocateFabric`). `FerrovozNative` y `FerrovozLog` no se duplican.

Los binarios nativos de otras plataformas se leen de la misma carpeta que la 26.3 (`mod/natives-prebuilt/`). Diferencias con la 26.3: en Fabric los permisos usan `fabric-permissions-api` (la de LuckPerms, va dentro del jar) con los mismos nodos `ferrovoz.*`; el resto funciona igual. El protocolo de voz es el mismo, así que el plugin de proxy sirve para redes con servidores de las dos versiones.

Si cambias algo de `mod/src` que toca Minecraft (lo que no está en la lista `SHARED_MAIN`/`SHARED_CLIENT` de `mod-1.21.1/build.gradle`), haz el mismo cambio en su copia de `mod-1.21.1/src`.

## Publicar una versión

Las versiones se publican en la pestaña [Releases](../../releases) de este repo, con el jar universal (5 plataformas), su `.sha256` y las notas de `CHANGELOG.md`.

1. Sube `mod_version` en `mod/gradle.properties` y `version` en `native/Cargo.toml`.
2. Añade su sección `## [X.Y.Z]` en `CHANGELOG.md`.
3. Haz commit y crea la etiqueta: `git tag vX.Y.Z && git push --tags`.

El CI compila todo, comprueba que la etiqueta coincide con `mod_version`, que el jar lleva las 5 plataformas y que hay notas, y publica la release. Si la versión lleva guion (`1.0.0-alpha`) se marca como *prerelease*. Si la release ya existe, no la pisa. No hace falta ningún token: usa el de la propia Action.

## Instalación

Hay un `.jar` por versión de Minecraft (26.3 y 1.21.1), y cada uno vale para todo:

| Dónde | Cargador | Dónde va el jar | Además | Configuración |
|---|---|---|---|---|
| Jugadores o servidor | **Fabric** 26.3 | `mods/` | Fabric API | `config/ferrovoz.properties` |
| Jugadores o servidor | **NeoForge** 26.3 | `mods/` | - | `config/ferrovoz.properties` |
| Jugadores o servidor | **Forge** 26.3 (66.1+) | `mods/` | - | `config/ferrovoz.properties` |
| Servidor | **Paper, Purpur, Folia o Spigot** 26.3 | `plugins/` | - | `plugins/Ferrovoz/ferrovoz.properties` |

| Proxy | **Velocity** 4.3+ o **BungeeCord** (y derivados) | `plugins/` | Java 25 | `plugins/ferrovoz/ferrovoz-proxy.properties` (Velocity) · `plugins/Ferrovoz/ferrovoz-proxy.properties` (BungeeCord) |

Los jugadores deben usar el **mismo cargador que el servidor**, excepto con Paper/Spigot: ahí sirve el cliente de Fabric. Fabric, NeoForge y Forge envían los mismos datos por la red, así que un cliente de un cargador con un servidor de otro también debería funcionar, pero no se ha probado.

Quien entre sin el mod juega normal, solo que sin voz. En Folia cada jugador se procesa en el hilo de su región. No uses `/reload` con Ferrovoz: reinicia el servidor.

```properties
port=24454            # puerto UDP (ábrelo en el firewall/router)
bind_host=            # interfaz (vacío = todas)
public_host=          # host que se anuncia a los clientes (vacío = el del juego). Úsalo tras un proxy o con SRV.
max_distance=48       # alcance normal
whisper_distance=8    # alcance al susurrar
shout_distance=96     # alcance al gritar
transport=auto        # auto = UDP y, si no llega, túnel · udp = solo UDP · tunnel = siempre por el juego
spectators=ghost      # espectadores: ghost (por defecto) · normal · listen · own · off (ver «Espectadores y muertos»)
dead=normal           # muertos: igual
max_voices=24         # voces por separado como mucho en cada zona; el resto, al murmullo (0 = sin límite)
cave_echo=true        # eco en cuevas y salas; false lo quita a todos aunque lo tengan activado
```

### Detrás de un proxy (Velocity, BungeeCord…)

```
 jugadores ──UDP──► proxy :24454 ──► lobby    :24455 (relé de voz)
                                 └─► survival :24456 (relé de voz)
```

1. Pon el jar en `plugins/` del **proxy** y abre en él el puerto UDP **24454**. Es el único que tiene que verse desde fuera.
2. Pon el jar en cada **servidor** (mod o plugin, como siempre). Si varios están en la misma máquina que el proxy, dale a cada uno un `port` distinto en su `ferrovoz.properties` (24455, 24456…). Esos puertos no hace falta abrirlos al exterior: solo tiene que llegar a ellos el proxy.
3. Listo. El proxy lee el mensaje de conexión que cada servidor manda al jugador, apunta a qué relé va y le dice al cliente que mande la voz al proxy. Al cambiar de servidor la voz cambia sola.

`ferrovoz-proxy.properties`: `port` (puerto público, 24454), `bind_host` y `public_host` (host que se anuncia a los clientes; vacío = el mismo con el que entran al proxy).

El reenviador está en Rust, en un solo hilo para todos los jugadores, y solo deja pasar paquetes auténticos y nuevos (anti-replay): nadie puede desviar la voz de otro. Si el UDP no llega, la voz por túnel (`transport=auto`) también funciona a través del proxy.

**Grupos de red.** Viven en el proxy, así que sus miembros se oyen aunque estén en servidores distintos (como una radio para toda la red):

| Comando | Quién |
|---|---|
| `/vozred crear <nombre> [contraseña]` · `unirse <nombre> [contraseña]` · `salir` · `lista` | todos |
| `/vozred expulsar <jugador>` · `borrar <nombre>` | dueño del grupo o `ferrovoz.admin` en el proxy |

En inglés, `/voicenet create|join|leave|list|kick|delete`. La voz de quien está en un grupo de red no pasa por su servidor: el proxy la reparte directamente a los demás miembros, cifrada con la clave de cada uno, y nadie más la oye. Al cambiar de servidor sigues en el grupo; al salir del proxy, sales. El HUD lo marca como «[red]» y muestra el nombre de quien habla aunque esté en otro servidor. Limitaciones: necesita UDP (con túnel no llega la voz del grupo) y los silencios de admin de un servidor no se aplican a los grupos de red (usa `/vozred expulsar`). Los clientes anteriores a la 0.10 no oyen los grupos de red.

## Uso en el juego

| Tecla | Acción |
|---|---|
| **V** | Pulsar para hablar |
| **B** | Susurrar (también hace de pulsar para hablar) |
| **G** | Gritar |
| **M** | Silenciar micro |
| **J** | Ajustes de voz |
| sin asignar | Cambiar modo (pulsar / voz) · Ensordecer |

**HUD:** estado (hablando / susurrando / gritando, «⇄ túnel» si aplica), tu grupo, vúmetro y quién habla.

**Comandos:** `/voz` muestra la ayuda según tus permisos.

| Comando | Quién |
|---|---|
| `/voz grupo crear <nombre> [contraseña]` · `unirse` · `salir` · `lista` | todos (`ferrovoz.groups`) |
| `/voz grupo expulsar <jugador>` · `fijar` · `desfijar` · `borrar <nombre>` | dueño del grupo o admin |
| `/voz estado` · `silenciar <jugador>` · `desilenciar <jugador>` | admin |

Todo funciona también en inglés: `/voice group create|join|leave|list|kick|pin|unpin|delete`, `/voice status|mute|unmute`. `/voz estado` muestra además la versión y la CPU que usa el relé de voz.

**Ajustes (J):** deslizadores de volumen, ganancia y umbral; micrófono y salida; supresión de ruido; probar micro; **eco en cuevas** y **voz bajo el agua** (sí/no); **cancelación de eco**, **ganancia automática** y **audio 3D (cascos)** (sí/no, activados por defecto); **volumen de grupo**; **Jugadores…**; **Teclas…** (cambiar cualquier tecla de Ferrovoz; las que chocan con otra salen en rojo); **Usar chat de voz** (sí/no); y el estado de la red.

## Rendimiento

### En un servidor de Fabric de verdad (a mano)

Un **servidor dedicado de Minecraft 26.3 con Fabric API y el jar**, arrancado en modo benchmark (`-Dferrovoz.bench=<jugadores>`):

1. **Reposo, 20 s:** mundo cargado y sin voz. Es la referencia del tick.
2. **Jugadores virtuales:** el servidor registra N jugadores repartidos sobre el terreno real. Un proceso aparte (`native/examples/bench_clients.rs`) hace de sus clientes y manda voz cifrada por UDP como el mod.
3. **Medición, 60 s:** cada tick los jugadores se mueven y pasan por **el mismo código que los reales**: posiciones al núcleo, permisos, avisos de grupo y paredes calculadas con los bloques del mundo.

Al terminar escribe `ferrovoz-bench/report.md` y apaga el servidor. El informe incluye:
- El tick de Minecraft en reposo y con Ferrovoz (media, p95, p99, máximo) y los TPS.
- **Lo que Ferrovoz añade al tick.**
- La CPU del relé y la del proceso entero.
- Paquetes por segundo y la memoria Java y del proceso.
- Cuánta voz llega a los clientes.

Resultado con 100 jugadores en una plaza y 20 hablando (2 núcleos): Ferrovoz añade **0,57 ms de media al tick** (1,1 % del presupuesto), el servidor se mantiene a 20 TPS, el relé usa el 37 % de un núcleo para 74 500 paquetes/s, la memoria no cambia y llega el 99,9 % de la voz.

No incluye el coste propio de Minecraft por cada jugador conectado (entidades, chunks, red del juego): eso existe con o sin Ferrovoz.

Para lanzarlo en tu servidor de pruebas: pon `pause-when-empty-seconds=0` en `server.properties` (si no, Minecraft deja de hacer ticks a los 60 s sin jugadores reales), arranca con `-Dferrovoz.bench=100` (opcional: `-Dferrovoz.bench.layout=crowd`, `-Dferrovoz.bench.seconds=120`, `-Dferrovoz.bench.talking=20`) y, en la misma máquina, ejecuta `cargo run --release --example bench_clients -- <carpeta del servidor>/ferrovoz-bench`. **No lo uses en un servidor con jugadores:** fuerza la carga de chunks y apaga el servidor al terminar.

### Solo el relé (`cargo test --release --test load -- --ignored --nocapture`)

El relé aislado, con clientes falsos por UDP real. Medido en una máquina de 2 núcleos (los clientes falsos comparten esos núcleos):

| Escenario | Jugadores | Hablando | CPU del relé | Paquetes enviados/s | Voz entregada |
|---|---|---|---|---|---|
| Repartidos por el mapa | 100 | 20 | **2 %** | 4 200 | 100 % |
| Evento, todos juntos | 50 | 10 | 8 % | 24 500 | 100 % |
| Plaza llena | 100 | 25 | 35 % | 124 000 | 100 % |
| Extremo: 100 juntos hablando todos a la vez | 100 | 100 | 134 % | 495 000 | 100 % |
| Evento de 250, 150 hablando, **sin límite** (`max_voices=0`) | 250 | 150 | 148 % (saturado) | 577 000 | **31 %** |
| Evento de 250, 150 hablando, **con `max_voices=24`** | 250 | 150 | 141 % | 299 000 | todas las que tocan + murmullo |

La CPU es la de todos los hilos del relé juntos (recibir, cifrar y enviar, y mezclar el murmullo); 100 % = un núcleo. Sin límite, 250 personas juntas con 150 hablando piden casi 1,9 millones de paquetes por segundo: el relé no da abasto, recibe solo un tercio de la voz y entrega un 31 %. Con el límite recibe toda la voz y envía lo que debe: 24 voces y el murmullo a cada uno.

Cada paquete cuesta unos 2 µs (cifrar y enviar). El relé no bloquea el tick de Minecraft: recibe en su propio hilo y reparte el cifrado y el envío entre hilos de ayuda (uno por núcleo libre, hasta 7). En Linux envía en lotes de 64 paquetes por llamada al sistema (`sendmmsg`). Su techo crece con los núcleos: unos 400 000 paquetes por segundo por núcleo.

Antes de la 0.9 (un solo hilo, un envío por paquete) el caso extremo se quedaba en 212 000 paquetes/s y llegaba el 43 % de la voz.

## Licencias

Ferrovoz es MIT. El jar incluye `META-INF/THIRD_PARTY_LICENSES.txt` con las licencias de todo lo que llevan dentro las librerías nativas: libopus (BSD-3), RNNoise (BSD-3), el procesado de audio de WebRTC portado a Rust (`sonora`, BSD-3) y 93 crates de Rust (MIT, Apache-2.0, BSD, ISC…). Se genera con `python3 tools/third_party_licenses.py` y el CI falla si las dependencias cambian y el archivo no se actualiza.

## Tests

```bash
cd native && cargo test --release
```

154 tests: unitarios (audio, cifrado, jitter buffer, HRTF, reverb, AEC/AGC, mezcla de multitudes...) y de extremo a extremo con un relé real por UDP en localhost: alcances, grupos, permisos, túnel, pérdida de paquetes, ataques de replay y flood, reglas de espectadores, repeticiones, multitudes y anfitrión, voz a través del proxy y lo que usa la API (canales de audio, megáfono, silencio general, escucha del micro, eventos). La prueba de carga está en [Rendimiento](#rendimiento).

La API de Java se prueba aparte contra la librería nativa real, sin Minecraft (Java 21 y Rust):

```bash
bash tools/api_test.sh
```
