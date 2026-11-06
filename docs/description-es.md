# Ferrovoz: chat de voz de proximidad

**Habla con quien tengas cerca, óyelo desde donde está y deja que el mundo cambie cómo suena.**
Ferrovoz es un chat de voz de proximidad para **Minecraft 26.3 y 1.21.1** con un motor de alto rendimiento escrito en **Rust**.

Funciona como mod de **Fabric, NeoForge y Forge**, como plugin de **Paper, Purpur, Folia y Spigot** y en proxies **Velocity y BungeeCord**. Un solo `.jar` por versión para todo.

---

## Funciones

### Una voz que se siente real
- **Audio 3D**: la voz baja con la distancia y llega desde donde está quien habla. Con cascos se distingue también si está **delante o detrás, arriba o abajo**.
- **Susurrar y gritar**: susurra (8 bloques) o grita (96 bloques) además de hablar normal (48 bloques). Todos los alcances se pueden cambiar.
- **Las paredes tapan la voz**: los bloques sólidos entre medias hacen que suene más baja y apagada. El servidor también lo calcula, incluso para jugadores fuera de tu distancia de renderizado.
- **El sonido rodea**: si hay una puerta abierta, una ventana o una esquina, la voz llega por ahí, más clara y **desde el hueco**. Las puertas cerradas tapan; abiertas, dejan pasar.
- **Eco según la forma del sitio**: cada cueva y sala suena distinta según su tamaño y sus materiales (la piedra resuena, la lana apaga), con los primeros ecos llegando de las paredes cercanas. Y al hablar dentro **oyes tu propio eco**.
- **Voz bajo el agua**: con la cabeza sumergida (la tuya o la de quien habla) la voz suena tapada y hueca, con una ondulación suave y burbujas.
- **Indicadores sobre la cabeza**: una nota musical (♪) sobre quien habla, y se ve quién tiene el micro silenciado, quién está ensordecido y quién va sin voz.

### Un micro que suena bien
- **Cancelación de eco**: si juegas con altavoces, los demás no se oyen a sí mismos a través de tu micro.
- **Ganancia automática**: tu voz llega a un volumen normal aunque tu micro esté muy bajo o muy alto.
- **Supresión de ruido** (RNNoise) que quita ventiladores, zumbidos y siseos.

### Para creadores de contenido
- **La voz queda en las repeticiones** de **Replay Mod** y **Flashback**: la de todos y la tuya, sin configurar nada.
- Al verlas, cada voz sale de su personaje y suena desde la cámara, con audio 3D, paredes y eco.
- Con **Flashback**, la voz **sale en los vídeos que exportas**.

### Grupos
- Crea un grupo, con contraseña si quieres: sus miembros se oyen **a cualquier distancia y en cualquier dimensión**, como una radio.
- Los grupos tienen **dueño**, que puede expulsar, fijar el grupo o borrarlo.
- Los **grupos permanentes** sobreviven a los reinicios y sus miembros vuelven a ellos solos.
- Avisos en el chat cuando alguien entra o sale, y tu grupo se ve en el HUD.

### Ajustes sencillos (tecla **J**)
- Elige **micrófono y salida de audio**, con deslizadores de volumen, ganancia y umbral de activación por voz.
- **Probar micro** para oírte.
- **Volumen por jugador**: baja o silencia a alguien, solo para ti. Y el **volumen de grupo**, aparte del de proximidad.
- Activa o desactiva el eco en cuevas, el efecto bajo el agua, la cancelación de eco, la ganancia automática y el audio 3D para cascos.
- **No usar chat de voz**: ni oyes ni te oyen, con un solo botón.
- **Cambia tus teclas** desde ahí mismo (Teclas…); las que chocan con otra salen en rojo.
- Ping, pérdida de paquetes y calidad de audio de un vistazo.

### Para los dueños de servidores
- **Moderación**: silencia jugadores en el chat de voz. Los silencios se guardan y se mantienen tras reiniciar.
- **Espectadores y muertos**: por defecto, a los espectadores no se les oye desde la partida (hablan entre ellos y oyen a los que juegan). Se puede cambiar: que se les oiga, que solo escuchen… (`spectators` y `dead`).
- **Permisos** compatibles con **LuckPerms** e iguales en todas las plataformas:

  | Nodo | Por defecto | Para qué |
  |---|---|---|
  | `ferrovoz.speak` | todos | hablar |
  | `ferrovoz.shout` | todos | gritar (sin él, el grito es voz normal) |
  | `ferrovoz.groups` | todos | usar grupos |
  | `ferrovoz.admin` | operadores | `/voz estado`, silenciar y gestionar cualquier grupo |

- **¿No puedes abrir puertos?** Si el UDP no pasa, la voz va sola por la conexión del juego.
- **Proxies**: en Velocity o BungeeCord, un único puerto UDP público en el proxy reparte la voz a todos los servidores. Cambiar de servidor funciona sin más.
- **Grupos de red**: con `/vozred crear <nombre>` (o `/voicenet create`), los miembros se oyen aunque estén en servidores distintos de la red.
- **Cinco idiomas**: español, inglés, portugués, francés y alemán, según el idioma del juego de cada jugador (y es fácil añadir más).
- **Avisos claros**: se avisa al jugador si su versión de Ferrovoz no coincide con la del servidor, si la voz no está disponible en su sistema o si su micrófono está bloqueado (por ejemplo, falta el permiso de macOS).

### Rápido y seguro
- El motor de audio (captura, códec Opus, cifrado, mezcla y efectos) está escrito en **Rust**; Java solo lo conecta con el juego.
- **Voz cifrada** (ChaCha20-Poly1305, una clave por jugador) con protección **anti-replay** y límite de tráfico por jugador.
- Códec **Opus** con calidad automática: 32 kbps con buena conexión, hasta 14 kbps con más protección si se pierden paquetes.
- **Medido en un servidor de Fabric real** con 100 jugadores en el mismo sitio y 20 hablando a la vez: Ferrovoz añadió **0,57 ms por tick de media** (en torno al 1 % del tiempo de cada tick), el servidor se mantuvo a **20 TPS** y llegó el **99,9 %** de la voz.
- **Preparado para eventos grandes**: el relé reparte la voz entre varios núcleos y envía en lotes; en 2 núcleos aguanta 100 jugadores juntos hablando todos a la vez (465 000 paquetes/s).

---

## Instalación

**Minecraft 26.3**: `ferrovoz-26.3-<versión>.jar` para todo.

| Dónde | Cargador | Pon el jar en | Necesita además |
|---|---|---|---|
| Cliente o servidor | **Fabric** 26.3 | `mods/` | Fabric API |
| Cliente o servidor | **NeoForge** 26.3 | `mods/` | - |
| Cliente o servidor | **Forge** 26.3 (66.1+) | `mods/` | - |
| Servidor | **Paper / Purpur / Folia / Spigot** 26.3 | `plugins/` | - |
| Proxy | **Velocity** 4.3+ / **BungeeCord** | `plugins/` | Java 25 |

**Minecraft 1.21.1**: `ferrovoz-1.21.1-<versión>.jar` para todo.

| Dónde | Cargador | Pon el jar en | Necesita además |
|---|---|---|---|
| Cliente o servidor | **Fabric** 1.21.1 | `mods/` | Fabric API |
| Cliente o servidor | **NeoForge** 21.1 | `mods/` | - |
| Cliente o servidor | **Forge** 52 | `mods/` | - |
| Servidor | **Paper / Purpur / Folia / Spigot** 1.21.1 | `plugins/` | - |
| Proxy | **Velocity** 3.4+ / **BungeeCord** | `plugins/` | Java 21 |

1. Instala Ferrovoz en el **servidor** y en cada **jugador**.
2. Abre el **puerto UDP 24454** en el servidor (se cambia en `ferrovoz.properties`). Si no puedes, la voz funciona igual por la conexión del juego.
3. Entra y pulsa **V** para hablar.

Quien no tenga Ferrovoz puede entrar y jugar con normalidad, solo que sin voz.

**Sistemas compatibles:** Windows (x64), Linux (x64 y ARM64) y macOS (Apple Silicon e Intel).

### Detrás de un proxy (Velocity / BungeeCord)
1. Pon el jar en el **proxy** y abre allí el puerto UDP **24454**. Es el único que tiene que ser público.
2. Pon el jar en cada **servidor**. Si varios están en la misma máquina, dale a cada uno un `port` distinto en su `ferrovoz.properties` (24455, 24456…).
3. Listo. La voz sigue a los jugadores cuando cambian de servidor.

---

## Controles

| Tecla | Acción |
|---|---|
| **V** | Pulsar para hablar |
| **B** | Susurrar |
| **G** | Gritar |
| **M** | Silenciar micro |
| **J** | Ajustes de voz |
| *(sin asignar)* | Cambiar entre pulsar para hablar y activación por voz · Ensordecer |

Todas las teclas se cambian en **J → Teclas…** (pulsa una acción y luego la tecla nueva).

## Comandos

| Comando | Quién |
|---|---|
| `/voz grupo crear <nombre> [contraseña]` · `unirse` · `salir` · `lista` | todos |
| `/voz grupo expulsar <jugador>` · `fijar` · `desfijar` · `borrar <nombre>` | dueño del grupo o admin |
| `/voz estado` · `silenciar <jugador>` · `desilenciar <jugador>` | admin |
| `/vozred crear <nombre> [contraseña]` · `unirse` · `salir` · `lista` · `expulsar` · `borrar` | en el proxy (grupos de red) |

También funciona `/voice`, con los subcomandos en inglés (`/voice group create`…).

---

## Configuración

`ferrovoz.properties` (servidor):

```properties
port=24454            # puerto UDP (ábrelo en el firewall/router)
bind_host=            # interfaz donde escuchar (vacío = todas)
public_host=          # host que se anuncia a los clientes (vacío = el del juego)
max_distance=48       # alcance normal
whisper_distance=8    # alcance al susurrar
shout_distance=96     # alcance al gritar
transport=auto        # auto = UDP y, si no llega, por el juego · udp · tunnel
spectators=ghost      # espectadores: ghost (por defecto: hablan entre ellos) · normal · listen (solo escuchan) · own · off
dead=normal           # muertos: igual
```

---

## Preguntas frecuentes

**¿Los jugadores necesitan el mod?** Sí, para hablar y oír. Quien no lo tenga puede entrar y jugar con normalidad.

**¿Un cliente de Fabric puede entrar en un servidor Paper?** Sí. Los servidores Paper y Spigot funcionan con el cliente de Fabric.

**¿Funciona en un mundo local o en LAN?** Sí. El servidor integrado también tiene chat de voz, y **Probar micro** funciona sin ningún servidor.

**En macOS no se oye mi micrófono.** Abre *Ajustes del Sistema → Privacidad y seguridad → Micrófono*, activa tu launcher y reinicia el juego. Ferrovoz te avisa en el chat cuando pasa.

---

Hecho por **Piratemajo** · Licencia MIT · Las licencias de terceros van dentro del jar (`META-INF/THIRD_PARTY_LICENSES.txt`).
