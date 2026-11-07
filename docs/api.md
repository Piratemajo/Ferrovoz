# API de Ferrovoz para plugins y mods

Con la API, otro plugin o mod del servidor puede:

- **Controlar la voz**: silenciar, forzar permisos, dar a alguien un alcance propio (megáfono, escenario), silencio general para un discurso, grupos sin contraseña, alcances y multitudes en marcha.
- **Enterarse de lo que pasa**: quién conecta, quién empieza y deja de hablar, cambios en los grupos… e impedir que un jugador cree un grupo, entre en uno o silencie a alguien.
- **Poner sonido**: canales de audio que suenan desde un punto, siguiendo a un jugador, a unos jugadores concretos o a un grupo, con 3D como la voz. WAV, PCM u Opus.
- **Escuchar la voz**: recibir el audio de cada jugador para grabarlo, transcribirlo o hacer un bot.

Es la misma en **Fabric, NeoForge, Forge y Paper/Spigot/Folia**, y en **26.3 y 1.21.1**: no usa ninguna clase de Minecraft. Solo funciona en el servidor.

Todo lo de esta página está en el paquete `dev.ferrovoz.api` y tiene javadoc.

## Añadirla al proyecto

En cada [release](https://github.com/Piratemajo/Ferrovoz/releases) hay un `ferrovoz-api-<versión>.jar` (más `-sources` y `-javadoc`). Es Java 21 y no depende de nada. Cópialo a `libs/` y compila contra él sin meterlo en tu jar: en el servidor ya lo pone Ferrovoz.

```groovy
dependencies {
    compileOnly files('libs/ferrovoz-api-0.12.0.jar')
}
```

Y declara que usas Ferrovoz, para que cargue antes:

| Plataforma | Dónde | Qué poner |
|---|---|---|
| Paper/Spigot/Folia | `plugin.yml` | `depend: [Ferrovoz]` (o `softdepend` si es opcional) |
| Fabric | `fabric.mod.json` | `"depends": { "ferrovoz": "*" }` (o `"suggests"`) |
| NeoForge / Forge | `mods.toml` | una `[[dependencies.tumod]]` con `modId="ferrovoz"` |

## Conseguir la API

```java
FerrovozApi.whenReady(api -> {
    // aquí ya se puede usar
});
```

`whenReady` se ejecuta ahora mismo si Ferrovoz ya ha cargado, o en cuanto cargue, así que no importa el orden en que arranquen los mods o plugins. También están `FerrovozApi.get()` (lanza una excepción si no está) y `FerrovozApi.find()` (devuelve un `Optional`). En Paper está además en el `ServicesManager`.

Si el servidor no tiene librería nativa para su sistema, Ferrovoz no tiene voz y la API no aparece: `find()` sale vacío.

### Cuándo funciona cada cosa

Ferrovoz carga al arrancar el servidor, pero el **relé de voz** (lo que mueve el audio) se pone en marcha cuando el servidor ya ha arrancado y se para al apagarlo. `api.isRunning()` dice si está en marcha, y hay eventos para ambos momentos (`VoiceServerStartedEvent` y `VoiceServerStoppingEvent`).

Con el relé parado, las consultas devuelven vacío, los cambios no hacen nada, crear un canal lanza `IllegalStateException` y lo de los grupos, `VoiceGroupException` con motivo `NOT_RUNNING`.

### Hilos

- Se puede llamar a la API **desde cualquier hilo**.
- Los **eventos** llegan en el hilo del servidor. En Folia, en el planificador global.
- La **voz que se escucha** llega en un hilo propio (`ferrovoz-microphone`).
- Las **fuentes de `play`** se leen en otro hilo propio (`ferrovoz-audio`).

## Jugadores

```java
api.player(uuid).ifPresent(p -> {
    p.isConnected();       // ya le llega la voz
    p.isSpeaking();        // se le oye ahora mismo
    p.isMicrophoneMuted(); // ha silenciado su micro
    p.group();             // Optional<VoiceGroup>
    p.setMuted(true);      // silencio de moderación: se guarda en disco, como /voz silenciar
});
```

`api.player(uuid)` está vacío si el jugador no tiene Ferrovoz. El `VoicePlayer` que devuelve es una vista: cada llamada mira el estado actual.

### Permisos y alcance

```java
VoicePlayer host = api.player(uuid).orElseThrow();
host.setPermission(VoicePermission.PRIORITY, PermissionOverride.ALLOW); // se le oye siempre
host.setRange(300);                                                     // megáfono: 300 bloques
// ...
host.resetRange();
host.setPermission(VoicePermission.PRIORITY, PermissionOverride.INHERIT); // vuelve a mandar LuckPerms
```

Lo que fija un plugin manda sobre los permisos del servidor (`ferrovoz.speak`, `ferrovoz.shout`, `ferrovoz.priority`) y se aplica al momento. Dura mientras el servidor esté encendido, también si el jugador sale y vuelve, pero **no se guarda en disco**.

## Ajustes en marcha

```java
VoiceSettings s = api.settings();
s.setHushed(true);                  // silencio general: solo se oye a quien tenga prioridad
s.setRange(TalkMode.SHOUT, 128);
s.setMaxVoices(16);                 // voces por separado en una multitud
s.setCaveEcho(false);
```

Empiezan con los valores de `ferrovoz.properties`, y lo que se cambie aquí dura hasta reiniciar.

## Grupos

```java
VoiceGroup team = api.groups().create("Rojos", null, captain); // captain entra y es el dueño
team.add(otherPlayer);              // sin contraseña; si estaba en otro grupo, sale de él
team.setPersistent(true);           // se guarda en disco y recuerda a sus miembros
team.members();                     // los conectados ahora
team.delete();
```

Los nombres no distinguen mayúsculas, tienen hasta 24 caracteres y no pueden llevar `|`. Si algo falla se lanza `VoiceGroupException`, con `reason()` para saber por qué.

## Eventos

```java
Subscription sub = api.events().subscribe(PlayerStartedSpeakingEvent.class, e ->
        log.info(e.player().name() + (e.group() ? " habla a su grupo" : " habla (" + e.mode() + ")")));
// ...
sub.unsubscribe();
```

| Evento | Cuándo |
|---|---|
| `VoiceServerStartedEvent` / `VoiceServerStoppingEvent` | el relé arranca / se va a parar |
| `PlayerConnectedEvent` / `PlayerDisconnectedEvent` | a un jugador ya le llega la voz / la ha dejado |
| `PlayerStartedSpeakingEvent` / `PlayerStoppedSpeakingEvent` | se le empieza a oír / hace unos 400 ms que no |
| `GroupMemberEvent` | alguien entra, sale, es expulsado, vuelve a su grupo permanente o pasa a ser el dueño |
| `GroupDeletedEvent` | se borra un grupo |
| `GroupCreateEvent` ✋ | un jugador va a crear un grupo |
| `GroupJoinEvent` ✋ | un jugador va a entrar en un grupo (antes de mirar la contraseña) |
| `PlayerMuteEvent` ✋ | un moderador va a silenciar o quitar el silencio |

Los marcados con ✋ se pueden **cancelar**. Solo se lanzan cuando la acción la pide un jugador con `/voz`. Lo que hace un plugin por la API no los lanza, para que dos plugins no se pisen ni entren en bucle. Si cancelas, Ferrovoz no le dice nada al jugador, así que díselo tú:

```java
api.events().subscribe(GroupCreateEvent.class, e -> {
    if (!e.group().matches("[A-Za-z0-9 ]+")) {
        e.setCancelled(true);
        tellPlayer(e.player(), "Solo letras y números en el nombre del grupo.");
    }
});
```

Suscribirse a una clase recibe también sus subclases, así que `VoiceEvent.class` recibe todos. Con `EventPriority` se ordenan los suscriptores: `EARLY` decide primero y `LATE` ve el resultado final. Si un suscriptor lanza una excepción, se apunta en el log y los demás siguen.

## Sonido: canales de audio

Un canal es un altavoz del servidor. Los clientes lo oyen como a alguien que habla, con el mismo 3D, así que **funciona con cualquier cliente de Ferrovoz**. Los anteriores a la 0.12 lo muestran en la lista de quién habla con un nombre raro; los nuevos no lo muestran.

```java
AudioService audio = api.audio();
VoiceWorld world = api.world(bukkitWorld);   // o un ServerLevel en los mods

AudioChannel speaker = audio.channel().music().at(new Position(world, 100, 64, -20), 32); // desde un punto
AudioChannel radio   = audio.channel().following(playerId, 8);                            // le sigue
AudioChannel intercom = audio.channel().to(List.of(a, b));                                // solo a ellos, sin 3D
AudioChannel teamRadio = audio.channel().toGroup("Rojos");                                // a un grupo, sin 3D
```

Para reproducir algo entero, `play` lo envía a ritmo él solo:

```java
short[] song;
try (InputStream in = Files.newInputStream(path)) {
    song = Wav.read(in);                     // cualquier WAV: lo pasa a 48 kHz mono
}
Playback playback = speaker.play(FrameSource.of(song));
playback.finished().thenRun(speaker::close);
```

Si el audio lo generas tú, envía una trama cada 20 ms con `send(short[960])` (48 kHz, mono, 16 bits), o con `sendOpus(byte[])` si ya lo tienes en Opus.

- `music()` usa el modo de Opus para música y efectos, y `bitrate(…)` cambia la calidad (por defecto, 48 kbps).
- `setVolume` va de 0 a 4, y `moveTo`, `follow`, `sendTo` y `setRange` cambian el canal en marcha.
- Hay hasta 1024 canales abiertos a la vez, y se cierran solos al parar el relé. **Cierra los que ya no uses.**
- A los canales no se les aplican las paredes del servidor.

## Escuchar la voz

```java
Map<UUID, OpusDecoder> decoders = new ConcurrentHashMap<>();
Subscription listening = api.audio().listen(frame -> {
    OpusDecoder d = decoders.computeIfAbsent(frame.player(), id -> api.audio().createDecoder());
    short[] pcm = d.decode(frame.opus());    // 960 muestras
    recorder.write(frame.player(), pcm);     // a otro hilo si es lento
});
```

Llega cada trama de 20 ms de cada jugador que se oye, en Opus. Lo de un jugador silenciado no llega. `sequence()` sube de uno en uno: si salta, se ha perdido una trama, y `decoder.decode(null)` rellena el hueco.

El listener se llama unas 50 veces por segundo por cada jugador que habla, en el hilo `ferrovoz-microphone`: devuelve rápido y lleva el trabajo pesado a otro hilo. Mientras no haya nadie escuchando, el relé no copia nada. Cierra los decodificadores cuando un jugador se vaya (`PlayerDisconnectedEvent`).

Grabar o analizar la voz de la gente es delicado: **avisa a tus jugadores** si tu plugin lo hace.

## Ejemplo completo: modo escenario

Un comando para quien presenta un evento: su voz llega a 200 bloques, siempre se le oye y el público calla hasta que termine. Con la API de Bukkit, pero la parte de Ferrovoz sería igual en un mod.

```java
public final class StageCommand implements CommandExecutor {
    private UUID onStage;

    @Override
    public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
        if (!(sender instanceof Player player)) return false;
        FerrovozApi api = FerrovozApi.find().orElse(null);
        if (api == null || !api.isRunning()) {
            player.sendMessage("El chat de voz no está en marcha.");
            return true;
        }
        VoicePlayer voice = api.player(player.getUniqueId()).orElse(null);
        if (voice == null) {
            player.sendMessage("Necesitas Ferrovoz para subir al escenario.");
            return true;
        }
        if (player.getUniqueId().equals(onStage)) {
            voice.resetRange();
            voice.setPermission(VoicePermission.PRIORITY, PermissionOverride.INHERIT);
            api.settings().setHushed(false);
            onStage = null;
            player.sendMessage("Has bajado del escenario.");
        } else {
            voice.setRange(200);
            voice.setPermission(VoicePermission.PRIORITY, PermissionOverride.ALLOW);
            api.settings().setHushed(true);
            onStage = player.getUniqueId();
            player.sendMessage("Estás en el escenario: te oyen a 200 bloques y el público está en silencio.");
        }
        return true;
    }
}
```

## Compatibilidad

- `FerrovozApi.API_VERSION` sube si algo cambia de forma incompatible. Dentro de una misma versión solo se añaden cosas.
- Las clases de `dev.ferrovoz.api.internal` son internas: no las uses.
- Las pruebas de la API están en `tools/api_test.sh`, contra la librería nativa real, y en `native/tests/api.rs`.
