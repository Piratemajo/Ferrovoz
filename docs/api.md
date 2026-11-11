# Ferrovoz API for plugins and mods

*[Español](api-es.md)*

With the API, another plugin or mod on the server can:

- **Control voice**: mute players, override permissions, give someone their own range (a megaphone, a stage), hush everyone for a speech, manage groups without passwords, and change ranges and crowd limits while the server runs.
- **Know what's going on**: who connects, who starts and stops talking, group changes… and stop a player from creating a group, joining one or muting someone.
- **Play sound**: audio channels that play from a point, follow a player, or go to specific players or a group, in 3D like voice. WAV, PCM or Opus.
- **Listen to voice**: receive each player's audio to record it, transcribe it or build a bot.

It is the same on **Fabric, Quilt, NeoForge, Forge and Paper/Spigot/Folia**, and on **26.3 and 1.21.1**: it uses no Minecraft classes. It only works on the server.

Everything on this page is in the `dev.ferrovoz.api` package and has javadoc.

## Adding it to your project

Every [release](https://github.com/Piratemajo/Ferrovoz/releases) has a `ferrovoz-api-<version>.jar` (plus `-sources` and `-javadoc`). It targets Java 21 and depends on nothing. Put it in `libs/` and compile against it without shading it into your jar, because Ferrovoz already provides it on the server.

```groovy
dependencies {
    compileOnly files('libs/ferrovoz-api-0.13.1.jar')
}
```

Then declare that you use Ferrovoz so it loads first:

| Platform | Where | What to add |
|---|---|---|
| Paper/Spigot/Folia | `plugin.yml` | `depend: [Ferrovoz]` (or `softdepend` if optional) |
| Fabric (and Quilt) | `fabric.mod.json` | `"depends": { "ferrovoz": "*" }` (or `"suggests"`) |
| Quilt | `quilt.mod.json` | `"depends": [ "ferrovoz" ]` inside `quilt_loader` |
| NeoForge / Forge | `mods.toml` | a `[[dependencies.yourmod]]` with `modId="ferrovoz"` |

## Getting the API

```java
FerrovozApi.whenReady(api -> {
    // ready to use here
});
```

`whenReady` runs right away if Ferrovoz has already loaded, or as soon as it does, so the load order of mods and plugins doesn't matter. There's also `FerrovozApi.get()`, which throws if Ferrovoz isn't there, and `FerrovozApi.find()`, which returns an `Optional`. On Paper it is also registered in the `ServicesManager`.

If the server has no native library for its system, Ferrovoz has no voice and the API isn't registered, so `find()` is empty.

### When things work

Ferrovoz loads when the server starts, but the **voice relay** (what moves the audio) only starts once the server is up, and stops when it shuts down. `api.isRunning()` tells you whether it is running, and there are events for both moments (`VoiceServerStartedEvent` and `VoiceServerStoppingEvent`).

While the relay is stopped:

- queries return empty results and changes do nothing;
- creating a channel throws `IllegalStateException`;
- group operations throw `VoiceGroupException` with reason `NOT_RUNNING`.

### Threads

- You can call the API **from any thread**.
- **Events** are delivered on the server thread. On Folia, they run on the global region scheduler.
- **Received voice** arrives on a thread of its own (`ferrovoz-microphone`).
- **`play` sources** are read on another thread of its own (`ferrovoz-audio`).

## Players

```java
api.player(uuid).ifPresent(p -> {
    p.isConnected();       // their voice is flowing
    p.isSpeaking();        // they can be heard right now
    p.isMicrophoneMuted(); // they muted their own mic
    p.group();             // Optional<VoiceGroup>
    p.setMuted(true);      // moderator mute: saved to disk, like /voice mute
});
```

`api.player(uuid)` is empty if the player doesn't have Ferrovoz. The `VoicePlayer` it returns is a view: every call reads the current state.

### Permissions and range

```java
VoicePlayer host = api.player(uuid).orElseThrow();
host.setPermission(VoicePermission.PRIORITY, PermissionOverride.ALLOW); // always heard
host.setRange(300);                                                     // megaphone: 300 blocks
// ...
host.resetRange();
host.setPermission(VoicePermission.PRIORITY, PermissionOverride.INHERIT); // LuckPerms decides again
```

What a plugin sets takes precedence over the server's permissions (`ferrovoz.speak`, `ferrovoz.shout`, `ferrovoz.priority`) and applies immediately. It lasts while the server is running, including when the player leaves and comes back, but it is **not saved to disk**.

## Runtime settings

```java
VoiceSettings s = api.settings();
s.setHushed(true);                  // hush: only players with priority can be heard
s.setRange(TalkMode.SHOUT, 128);
s.setMaxVoices(16);                 // separate voices in a crowd
s.setCaveEcho(false);
s.setUnderwaterEffect(false);
```

These start from `ferrovoz.properties`, and anything changed here lasts until the server restarts.

## Groups

```java
VoiceGroup team = api.groups().create("Red", null, captain); // captain joins and owns it
team.add(otherPlayer);              // no password; if they were in another group, they leave it
team.setPersistent(true);           // saved to disk, remembers its members
team.members();                     // the ones online now
team.delete();
```

Names are case-insensitive, up to 24 characters long and can't contain `|` or `§`. Failures throw `VoiceGroupException`; `reason()` says why.

## Events

```java
Subscription sub = api.events().subscribe(PlayerStartedSpeakingEvent.class, e ->
        log.info(e.player().name() + (e.group() ? " talks to their group" : " talks (" + e.mode() + ")")));
// ...
sub.unsubscribe();
```

| Event | When |
|---|---|
| `VoiceServerStartedEvent` / `VoiceServerStoppingEvent` | the relay starts / is about to stop |
| `PlayerConnectedEvent` / `PlayerDisconnectedEvent` | a player's voice starts flowing / they left voice chat |
| `PlayerStartedSpeakingEvent` / `PlayerStoppedSpeakingEvent` | they start being heard / haven't been heard for about 400 ms |
| `GroupMemberEvent` | someone joins, leaves, gets kicked, is put back in their persistent group or becomes owner |
| `GroupDeletedEvent` | a group is deleted |
| `GroupCreateEvent` ✋ | a player is about to create a group |
| `GroupJoinEvent` ✋ | a player is about to join a group (before the password is checked) |
| `PlayerMuteEvent` ✋ | a moderator is about to mute or unmute someone |

Events marked ✋ can be **cancelled**. They are only fired when a player asks with `/voice`. What a plugin does through the API never fires them, so two plugins can't fight or loop. When you cancel one, Ferrovoz tells the player nothing, so tell them yourself:

```java
api.events().subscribe(GroupCreateEvent.class, e -> {
    if (!e.group().matches("[A-Za-z0-9 ]+")) {
        e.setCancelled(true);
        tellPlayer(e.player(), "Group names can only have letters and numbers.");
    }
});
```

Subscribing to a class also gets its subclasses, so `VoiceEvent.class` receives everything. `EventPriority` orders subscribers: `EARLY` decides first and `LATE` sees the final outcome. If a subscriber throws, the exception is logged and the rest still run.

## Sound: audio channels

A channel is a server-side speaker. Clients hear it the same way they hear someone talking, with the same 3D, so **it works with any Ferrovoz client**. Clients older than 0.12 list it among the people talking under an odd name, and newer ones hide it.

```java
AudioService audio = api.audio();
VoiceWorld world = api.world(bukkitWorld);   // or a ServerLevel on mod loaders

AudioChannel speaker  = audio.channel().music().at(new Position(world, 100, 64, -20), 32); // from a point
AudioChannel radio    = audio.channel().following(playerId, 8);                            // follows them
AudioChannel intercom = audio.channel().to(List.of(a, b));                                 // only them, no 3D
AudioChannel teamRadio = audio.channel().toGroup("Red");                                   // a group, no 3D
```

To play a whole clip, `play` sends it at the right pace for you:

```java
short[] song;
try (InputStream in = Files.newInputStream(path)) {
    song = Wav.read(in);                     // any WAV: converted to 48 kHz mono
}
Playback playback = speaker.play(FrameSource.of(song));
playback.finished().thenRun(speaker::close);
```

If you generate the audio yourself, send one frame every 20 ms with `send(short[960])` (48 kHz, mono, 16-bit), or with `sendOpus(byte[])` if it's already Opus.

- `music()` uses Opus's mode for music and sound effects, and `bitrate(…)` changes quality (48 kbps by default).
- `setVolume` goes from 0 to 4, and `moveTo`, `follow`, `sendTo` and `setRange` change a channel while it plays.
- Up to 1024 channels can be open at once, and they close by themselves when the relay stops. **Close the ones you no longer need.**
- Server-side wall occlusion is not applied to channels.

## Listening to voice

```java
Map<UUID, OpusDecoder> decoders = new ConcurrentHashMap<>();
Subscription listening = api.audio().listen(frame -> {
    OpusDecoder d = decoders.computeIfAbsent(frame.player(), id -> api.audio().createDecoder());
    short[] pcm = d.decode(frame.opus());    // 960 samples
    recorder.write(frame.player(), pcm);     // hand it to another thread if it's slow
});
```

You get every 20 ms frame, as Opus, from every player who can be heard. A muted player's voice doesn't arrive. `sequence()` goes up by one each frame: a jump means a lost frame, and `decoder.decode(null)` fills the gap.

The listener runs about 50 times per second for each player talking, on the `ferrovoz-microphone` thread, so return quickly and move heavy work to another thread. While nobody is listening, the relay copies nothing. Close decoders when a player leaves (`PlayerDisconnectedEvent`).

Recording or analysing people's voices is sensitive: **tell your players** if your plugin does it.

## Full example: stage mode

This is a command for whoever hosts an event. Their voice reaches 200 blocks, they're always heard, and the audience is hushed until they step down. It uses the Bukkit API, but the Ferrovoz part would be the same in a mod.

```java
public final class StageCommand implements CommandExecutor {
    private UUID onStage;

    @Override
    public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
        if (!(sender instanceof Player player)) return false;
        FerrovozApi api = FerrovozApi.find().orElse(null);
        if (api == null || !api.isRunning()) {
            player.sendMessage("Voice chat isn't running.");
            return true;
        }
        VoicePlayer voice = api.player(player.getUniqueId()).orElse(null);
        if (voice == null) {
            player.sendMessage("You need Ferrovoz to go on stage.");
            return true;
        }
        if (player.getUniqueId().equals(onStage)) {
            voice.resetRange();
            voice.setPermission(VoicePermission.PRIORITY, PermissionOverride.INHERIT);
            api.settings().setHushed(false);
            onStage = null;
            player.sendMessage("You left the stage.");
        } else {
            voice.setRange(200);
            voice.setPermission(VoicePermission.PRIORITY, PermissionOverride.ALLOW);
            api.settings().setHushed(true);
            onStage = player.getUniqueId();
            player.sendMessage("You're on stage: heard up to 200 blocks away, and the audience is hushed.");
        }
        return true;
    }
}
```

## Compatibility

- `FerrovozApi.API_VERSION` goes up if something changes incompatibly. Within one version, things are only added.
- Classes in `dev.ferrovoz.api.internal` are internal, so don't use them.
- The API is tested by `tools/api_test.sh`, against the real native library, and by `native/tests/api.rs`.
