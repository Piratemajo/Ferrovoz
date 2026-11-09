# Cambios

Formato: una sección `## [versión]` por versión. Al publicar, el CI copia la
sección de esa versión como notas de la release.

## [Sin publicar]

- **Contraseñas de grupo protegidas**: ya no se guardan tal cual ni en memoria ni en `ferrovoz-data.txt`, sino como PBKDF2-HMAC-SHA256 con una sal por grupo. Las de versiones anteriores se convierten solas al arrancar. En Paper, además, la contraseña de `/voz grupo crear|unirse` ya no queda en el log de la consola: se escribe tapada.
- **Aviso al borrar un grupo**: sus miembros reciben un mensaje en su idioma (antes se quedaban sin grupo sin saber por qué).
- **Los nombres de grupo no pueden llevar `§`**, que servía para meter colores o romper el formato de los mensajes del chat.
- **Guardar no frena la voz**: `ferrovoz-data.txt` se escribe en un hilo propio en vez de con el estado del relé bloqueado (en un disco lento, la voz de todos se paraba un momento al guardar). Al apagar se escribe lo pendiente.
- **Un fallo no tira la voz entera**: si un paquete raro o un error hace fallar a un hilo del relé, se descarta ese paquete y los demás hilos siguen; antes, el lock quedaba envenenado y se caía la voz de todo el servidor.
- **Relé más ligero**: descifra fuera del candado del estado y prepara la copia de la trama para cada oyente en los hilos que cifran y envían. Cada paquete cuesta un 15-20 % menos: el caso extremo de 100 personas juntas hablando todas pasa del 89 % al 100 % de voz entregada con menos CPU. Sin cambios para los jugadores ni el protocolo.
- **No se pierde voz a la entrada en eventos grandes**: el hilo que recibe ya no cifra ni envía (lo hacen todo los hilos de envío, salvo que vayan llenos), y el socket pide al sistema 4 MB de búfer en vez de los 64 KB de Windows, para aguantar las ráfagas de cientos de personas hablando a la vez. Importa sobre todo en Windows, donde cada envío cuesta varios µs.
- **Repartir cada trama cuesta la mitad en multitudes**: cada jugador guarda su zona y solo se recalcula al moverse, y quién la oye por separado y quién en el murmullo se decide en la misma vuelta, consultando el plan una vez por zona en vez de una por oyente. Con 500 personas juntas, de 47-75 µs a 23-33 µs por trama: un hilo atiende unas 40 000 por segundo (antes, unas 21 000), de sobra para un evento de 500 con 300 hablando.
- **Paredes sin picos en eventos grandes**: para decidir a quién calcular paredes ya no se recorren todas las parejas posibles, solo las voces que cada uno oye por separado. Con 500 personas juntas y 300 hablando, esa búsqueda (cada 4 ticks) baja de unos 13 ms a menos de 1 ms.

## [0.12.0]

- **API para plugins y mods** (`dev.ferrovoz.api`): la misma en Fabric, NeoForge, Forge y Paper/Spigot/Folia, en 26.3 y 1.21.1, y sin clases de Minecraft. Permite controlar la voz (silencio de moderación, forzar permisos, alcance propio para un megáfono o un escenario, **silencio general** en el que solo se oye a quien tiene prioridad, grupos sin contraseña, alcances, multitudes y eco en marcha), suscribirse a eventos (conectar, empezar y dejar de hablar, cambios en los grupos) y cancelar lo que piden los jugadores con `/voz` (crear un grupo, unirse, silenciar). También permite **poner sonido**: canales de audio que suenan desde un punto, siguen a un jugador o van a unos jugadores o a un grupo, con el mismo 3D que la voz, a partir de WAV, PCM u Opus. Y **escuchar la voz** de los jugadores para grabarla, transcribirla o hacer bots. El sonido de los plugins lo oyen todos los clientes, también los anteriores; estos lo muestran en la lista de quién habla. Se compila contra `ferrovoz-api-<versión>.jar`, que va en cada release. Ver [docs/api-es.md](docs/api-es.md) (en inglés: [docs/api.md](docs/api.md)). El javadoc de la API está en inglés.
- **Multitudes y eventos**: si a una zona le llegan más voces de las que se pueden distinguir (`max_voices` en `ferrovoz.properties`, 24 por defecto), sus jugadores reciben por separado las más cercanas y el resto **mezclado en el servidor como el murmullo de la multitud**, que suena desde donde está la gente. Nadie desaparece y el servidor aguanta: en una prueba con 250 personas juntas y 150 hablando, sin límite el relé solo entregaba el 31 % de la voz; con el límite la recibe toda y envía 24 voces y el murmullo a cada uno. Con menos voces que el límite no cambia nada. Funciona con clientes de cualquier versión (los anteriores a esta muestran el murmullo en el HUD como un hablante más).
- **Prioridad para el anfitrión** (`ferrovoz.priority`, por defecto operadores): su voz llega siempre por separado, aunque hable toda la plaza.
- **Paredes del servidor con coste fijo**: como mucho 300 cálculos por tick, las parejas más cercanas primero, y sin recalcular si nadie se ha movido de bloque. Con 250 personas juntas y 100 hablando pasa de 2,9 ms por tick (con picos de 24 ms) a 0,2 ms (picos de 2 ms).
- El relé monta cada trama de voz una vez y solo cambia lo de cada oyente: el caso extremo de 100 personas juntas hablando todas pasa del 94 % al 100 % de voz entregada.
- **El servidor puede quitar el efecto bajo el agua** a todos: `underwater_effect=false` en `ferrovoz.properties`. Funciona igual que con el eco: en Ajustes (J) sale «Desactivado por el servidor», y los clientes anteriores dejan de oír tapado a quien habla bajo el agua, pero no su propio efecto al estar sumergidos. También desde la API (`settings().setUnderwaterEffect`).
- **El servidor puede quitar el eco** de cuevas y salas a todos: `cave_echo=false` en `ferrovoz.properties`. Nadie lo oye aunque lo tenga activado, y en Ajustes (J) sale «Desactivado por el servidor». También funciona con clientes anteriores, salvo que en ellos sigue sonando el eco del sitio donde está quien escucha.
- **Retraso de la voz según la red de cada uno**: el colchón antes de reproducir cada frase ya no es fijo (60 ms), se ajusta a la conexión de quien habla. Con buena red baja a 40 ms (la conversación se siente más inmediata) y con Wi-Fi malo o datos móviles sube lo necesario, hasta 160 ms, para que la voz no se corte. Si un paquete llega tarde a media frase se espera por él en vez de tirarlo. En una red simulada muy inestable los cortes bajan del 14 % al 0 %. Solo cambia el cliente: funciona con servidores de cualquier versión.

## [0.11.0]

- **Minecraft 1.21.1**: Ferrovoz funciona también en 1.21.1, con las mismas funciones. Un solo jar, como en 26.3, para Fabric, NeoForge 21.1, Forge 52, Paper/Spigot y Velocity/BungeeCord. En Fabric 1.21.1 los permisos usan `fabric-permissions-api` (LuckPerms), con los mismos nodos `ferrovoz.*`.
- Arreglado: el juego o el servidor podían cerrarse de golpe (fallo de memoria) al parar la voz mientras llegaba voz por el túnel: al apagar el servidor, al cambiar de servidor o al desactivar la voz en J (sobre todo en NeoForge y Forge).
- Arreglado: tras morir y reaparecer, las paredes que calcula el servidor se medían desde donde moriste hasta volver a entrar (Fabric, NeoForge y Forge).
- Arreglado: un valor mal escrito en `ferrovoz.properties` o `ferrovoz-proxy.properties` (por ejemplo `max_distance=48,5`) devolvía a sus valores por defecto los ajustes que venían detrás y reescribía el archivo. Ahora solo ese valor usa el de por defecto, se avisa en el log y el archivo no se toca. Se admite la coma decimal.

## [0.10.0]

- Arreglado: en un hueco pequeño (3×3) la voz sonaba metálica y entrecortada. Las cuatro paredes a la misma distancia devolvían su eco a la vez, más fuerte que la propia voz, y se saturaba. Ahora los primeros ecos de paredes a la misma distancia no se suman, entre todos suenan como mucho a la mitad que la voz (−6 dB), los cuartos muy pequeños tienen menos eco, y tu propio eco solo se oye en sitios grandes (en un cuarto pequeño sonaba como un doble de ti, con retraso).
- Arreglado: había eco en los bosques. Ahora el eco de sala necesita techo; las hojas no lo son (el sonido se escapa entre ellas), así que en un bosque los troncos ya no hacen eco.
- **Indicadores sobre la cabeza**: además del ♪ de quien habla, se ve quién tiene el **micro silenciado** (por él, por un admin o sin permiso), quién está **ensordecido** y quién va **sin voz** (sin Ferrovoz, o con la voz desactivada). Los de estado son más pequeños y solo se ven de cerca (24 bloques). Necesita servidor 0.10.
- **Volumen de grupo** aparte del de proximidad (J): baja la "radio" del grupo sin bajar a quien tienes al lado, o al revés.
- Ajustes (J) reordenados para que quepa todo: volumen de voces y de grupo juntos; ganancia y umbral juntos; los interruptores de dos en dos.
- **La voz queda en las repeticiones de Replay Mod y Flashback** (Fabric). Se graba la voz de todos los que oyes y la tuya, sin configurar nada. Al ver la repetición, cada voz sale de su personaje y suena desde donde esté la cámara, con todo el audio 3D, las paredes, las puertas y el eco. Con Flashback, además, **la voz sale en los vídeos que exportas**. Solo hace falta Ferrovoz en quien graba y en quien ve la repetición; el servidor no cambia.
- **El sonido rodea las paredes**: si hay un muro entre tú y quien habla pero una puerta abierta, una ventana o una esquina por la que pasar, su voz llega también por ahí: más clara que a través del muro y desde el hueco, como en la realidad. Cuantas más esquinas dobla, más baja y más apagada. Las puertas, trampillas y puertas de valla cerradas tapan, y menos que la piedra; abiertas, dejan pasar.
- **Eco según la forma real del sitio**: rayos en todas direcciones miden cada sala o cueva. La duración del eco sale de su tamaño y de sus materiales (la piedra resuena, la lana y la madera apagan), y los **primeros ecos** llegan de las paredes cercanas, desde su lado y con su retardo de ida y vuelta. Ahora también resuenan las casas y salas, no solo las cuevas (y menos que una cueva).
- **Audio 3D para cascos**: además de izquierda y derecha, ahora se nota si quien habla está **delante o detrás** (lo de detrás suena más apagado, como en la realidad) y **arriba o abajo**. Sigue también hacia dónde miras en vertical: si miras a alguien que está encima, lo oyes de frente. Se puede desactivar en J (con altavoces suena mejor el estéreo simple).
- Ajustes (J) más compactos: volumen y ganancia, y modo y umbral, van de dos en dos para que todo quepa en pantallas bajas.
- La marca del umbral se ve siempre en el medidor del micro (en gris con «pulsar para hablar», cuando no se usa), para poder ajustarla antes de cambiar de modo.
- **Cancelación de eco**: para quien juega con altavoces. Ferrovoz quita del micro lo que suena por ellos (las voces de los demás y tu propio eco en cuevas), así nadie se oye repetido. Basado en el cancelador de eco de WebRTC (AEC3). Se activa o desactiva en J.
- **Ganancia automática del micro**: sube los micros bajos y baja los altos (±12 dB) hasta un nivel normal. Va después de la supresión de ruido y no actúa en las pausas, así que no sube lo que se cuela de los cascos (la música del juego) ni el ruido de fondo. En J.
- **No usar chat de voz** (J): ni oyes ni te oyen. El servidor deja de mandarte voz al momento. Se recuerda y se reactiva con el mismo botón, sin salir del servidor.
- **Espectadores y muertos**: nuevas opciones `spectators` y `dead` en `ferrovoz.properties`: `normal` (como cualquiera), `listen` (oyen pero no se les oye), `ghost` (hablan entre ellos y oyen a los vivos), `own` (solo entre ellos) u `off`. **Por defecto, los espectadores son `ghost`**: los que juegan ya no les oyen. Los muertos, `normal`.
- **Grupos de red** en Velocity y BungeeCord: `/vozred crear <nombre>` (o `/voicenet create`) crea un grupo cuyos miembros se oyen aunque estén en servidores distintos de la red. El HUD muestra el nombre de quien habla aunque esté en otro servidor.
- **Portugués, francés y alemán**, en el juego y en los mensajes del servidor y del proxy. El español del juego ahora también sale con `es_mx`, `es_ar` y el resto de variantes.
- Mismo protocolo que la 0.9: compatible con clientes y servidores 0.6 a 0.9 (los grupos de red y el aviso al desactivar la voz solo funcionan con 0.10 en los dos lados).

## [0.9.0]

**Soporte para proxies: Velocity, BungeeCord y derivados**, con el mismo jar.

- En el proxy, Ferrovoz abre **un solo puerto UDP público** y reenvía la voz de cada jugador al servidor en el que esté. Los servidores de detrás no necesitan abrir su puerto de voz al exterior ni ningún cambio en su configuración (salvo usar puertos distintos si están en la misma máquina).
- **Cambiar de servidor** dentro del proxy: la voz pasa sola al servidor nuevo; si el nuevo no tiene Ferrovoz, se corta la del anterior.
- Seguro: el reenviador solo deja pasar paquetes auténticos y nuevos (anti-replay); nadie puede desviar la voz de otro jugador.
- Si el UDP no llega, la voz por túnel sigue funcionando a través del proxy.
- Mismo protocolo que la 0.8: compatible con clientes y servidores 0.6, 0.7 y 0.8.
- **Teclas desde los ajustes**: J → Teclas… para cambiar cualquier tecla de Ferrovoz (pulsar una acción y luego la tecla nueva; Escape la deja sin asignar). Las que chocan con otra tecla salen en rojo.
- El jar se llama ahora `ferrovoz-<Minecraft>-<versión>.jar` (p. ej. `ferrovoz-26.3-0.9.0.jar`).
- **Relé de voz más rápido, para eventos grandes**: el cifrado y el envío se reparten entre varios hilos (uno por núcleo libre), en Linux los paquetes salen en lotes (`sendmmsg`) y el cifrado es unas 3 veces más barato. Con 100 jugadores juntos hablando todos a la vez, en 2 núcleos, el relé pasa de 212 000 a 465 000 paquetes/s y llega el 94 % de la voz (antes el 43 %). En una plaza con 100 jugadores y 25 hablando, la CPU del relé baja del 62 % al 24 %.
- Ajustes (J): el texto del medidor del micro ya no se monta sobre los botones, y en Teclas… los nombres largos ya no tapan el botón de la tecla (la columna se ajusta al idioma y, si no cabe, se recorta con «...»).
- **Mensajes de consola en inglés** (log, archivos de configuración y descripciones del mod/plugin), para publicar en Modrinth, CurseForge y Hangar. Los mensajes del juego siguen en el idioma de cada jugador.

## [0.8.1]

- **Te oyes con eco en las cuevas**: al hablar dentro de una cueva o sala cerrada oyes tu propia voz rebotar, como en la realidad (solo el eco; tu voz directa no se duplica). Sigue al ajuste «Eco en cuevas» de J.

## [0.8.0]

**Ahora también en NeoForge y Forge**, con el mismo jar de siempre: Fabric, NeoForge, Forge y Paper/Purpur/Folia/Spigot.

- **NeoForge 26.3** (cliente y servidor): todo igual que en Fabric. Además, botón «Configurar» en la lista de mods que abre los ajustes de voz.
- **Forge 26.3** (66.1+) (cliente y servidor): todo igual que en Fabric.
- Permisos con los mismos nodos en todos (`ferrovoz.speak`, `ferrovoz.shout`, `ferrovoz.groups`, `ferrovoz.admin`), compatibles con LuckPerms.
- Mismo protocolo que la 0.7: compatible con clientes y servidores 0.6 y 0.7.
- El jar incluye `pack.mcmeta` (Forge lo exige para cargar los recursos del mod).

## [0.7.0]

**Compatible con 0.6** (mismo protocolo): clientes y servidores 0.6 y 0.7 se entienden.

- **Voz bajo el agua**: si quien habla tiene la cabeza sumergida, o la tienes tú, la voz suena como bajo el agua (sin agudos, hueca, con el tono ondulando y un leve burbujeo). Con los dos sumergidos, aún más. Entra y sale sin chasquidos.
- **El eco de cuevas se puede desactivar**.
- Los dos efectos se activan o desactivan en Ajustes (J) y se guardan.
- En Paper/Spigot/Folia, los bloques anegados, las algas y las columnas de burbujas cuentan como agua.

## [0.6.0]

**Compatible con 0.5** (mismo protocolo).

- **Aviso de versión distinta**: si el cliente y el servidor no son compatibles, el jugador recibe un mensaje claro y el HUD lo indica, también con versiones anteriores.
- **Sin núcleo nativo** (Linux de 32 bits, Alpine…): el juego y el servidor arrancan sin voz y se avisa una vez. Si el puerto UDP está ocupado, Paper ya no desactiva el plugin.
- **Micrófono bloqueado**: aviso con los pasos para arreglarlo (permiso de micrófono en macOS, cascos silenciado).
- **Idiomas**: mensajes del servidor en el idioma de cada jugador (español e inglés) y comandos en los dos (`/voz` y `/voice`). HUD traducido.
- `/voz estado` muestra la versión y la CPU del relé.
- El jar incluye `META-INF/THIRD_PARTY_LICENSES.txt`.

## [0.5.0]

- Icono ♪ sobre la cabeza de quien habla.
- Deslizadores en los ajustes.
- Grupos con dueño, expulsar, grupos permanentes, avisos por chat y grupo en el HUD.
- Silencios de admin persistentes.
- Túnel con menos latencia (hilo propio).
- Seguridad: anti-replay y límite de tráfico por jugador e IP.
- Eco también según dónde está quien habla.
- Paredes calculadas por el servidor a larga distancia.
- Builds para Linux ARM y macOS Intel.
