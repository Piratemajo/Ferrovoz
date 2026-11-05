# Ferrovoz: Proximity Voice Chat

**Talk to the players around you, hear them from the right direction, and let the world shape the sound.**
Ferrovoz is a proximity voice chat for **Minecraft 26.3 and 1.21.1** with a high-performance engine written in **Rust**.

It runs as a mod on **Fabric, NeoForge and Forge**, as a plugin on **Paper, Purpur, Folia and Spigot**, and on **Velocity and BungeeCord** proxies. For 26.3 a single `.jar` covers everything; for 1.21.1 there is one for Fabric and one for NeoForge and Forge (both also work as a plugin).

---

## Features

### Voice that feels real
- **3D positional audio**: voices get quieter with distance and come from where the speaker is. With headphones you can also tell whether they're **in front or behind, above or below**.
- **Whisper and shout**: whisper (8 blocks) or shout (96 blocks) besides normal talking (48 blocks). All ranges are configurable.
- **Walls muffle voices**: solid blocks between you and the speaker make the voice quieter and duller. The server works this out too, even for players outside your render distance.
- **Sound goes around walls**: through an open door, a window or around a corner, the voice reaches you clearer and **from the opening**. Closed doors muffle; open ones let sound through.
- **Echo shaped by the place**: every cave and room sounds different depending on its size and materials (stone rings, wool deadens), with early reflections coming from nearby walls. You also **hear your own echo** when you talk inside.
- **Underwater voice**: with your head (or theirs) underwater, voices sound muffled and hollow, with a gentle warble and bubbles.
- **Indicators above heads**: a music note (♪) above whoever is talking, plus who has their mic muted, who is deafened and who has no voice chat.

### A mic that sounds right
- **Echo cancellation**: if you play on speakers, other players won't hear themselves through your mic.
- **Automatic mic level**: your voice arrives at a normal volume even if your mic is very quiet or very loud.
- **Noise suppression** (RNNoise) removes fans, hum and hiss.

### For content creators
- **Voice is saved in replays** from **Replay Mod** and **Flashback**: everyone's voice and yours, with nothing to configure.
- When watching, each voice comes from its player and is heard from the camera, with 3D audio, walls and echo.
- With **Flashback**, voice is **included in the videos you export**.

### Groups
- Create a group with an optional password: members hear each other **at any distance and in any dimension**, like a radio.
- Groups have an **owner** who can kick members, pin the group or delete it.
- **Permanent groups** survive restarts, and their members rejoin automatically.
- Chat notices when someone joins or leaves, and your group is shown in the HUD.

### Easy settings (press **J**)
- Pick your **microphone and output device**, with sliders for volume, mic gain and voice-activation threshold.
- **Test mic** to hear yourself.
- **Per-player volume**: turn someone down or mute them, just for you. Plus a **group volume** separate from proximity voice.
- Turn cave echo, the underwater effect, echo cancellation, automatic mic level and headphone 3D audio on or off.
- **Don't use voice chat**: hear no one and be heard by no one, with a single button.
- **Change your keys** right there (Keys…), with clashes shown in red.
- Ping, packet loss and audio quality at a glance.

### For server owners
- **Moderation**: mute players in voice chat. Mutes are saved and persist across restarts.
- **Spectators and dead players**: by default, spectators can't be heard by the players in the game (they talk among themselves and hear the game). You can change it: let them be heard, only listen… (`spectators` and `dead`).
- **Permissions**, compatible with **LuckPerms** and identical on every platform:

  | Node | Default | What it does |
  |---|---|---|
  | `ferrovoz.speak` | everyone | talk |
  | `ferrovoz.shout` | everyone | shout (without it, shouting is normal voice) |
  | `ferrovoz.groups` | everyone | use groups |
  | `ferrovoz.admin` | operators | `/voice status`, mute, and manage any group |

- **No open port? No problem.** If UDP can't get through, voice automatically goes through the game connection.
- **Proxy support**: on Velocity or BungeeCord, a single public UDP port on the proxy forwards voice to every backend server. Switching servers just works.
- **Network groups**: with `/voicenet create <name>`, members hear each other even when they're on different servers of the network.
- **Five languages**: English, Spanish, Portuguese, French and German, following each player's game language (and easy to add more).
- **Clear warnings**: players are told if their Ferrovoz version doesn't match the server's, if voice isn't available on their system, or if their microphone is blocked (e.g. missing macOS permission).

### Fast and secure
- The audio engine (capture, Opus codec, encryption, mixing, effects) is written in **Rust**; Java only connects it to the game.
- **Encrypted voice** (ChaCha20-Poly1305, one key per player) with **anti-replay** protection and per-player rate limiting.
- **Opus** codec with automatic quality: 32 kbps on a good connection, down to 14 kbps with extra protection when packets are being lost.
- **Measured on a real Fabric server** with 100 players in the same spot and 20 talking at once: Ferrovoz added **0.57 ms per tick on average** (about 1% of the tick budget), the server stayed at **20 TPS**, and **99.9%** of voice packets were delivered.
- **Ready for big events**: the relay spreads voice across CPU cores and sends in batches; on 2 cores it handles 100 players in one spot all talking at once (465,000 packets/s).

---

## Installation

**Minecraft 26.3**: `ferrovoz-26.3-<version>.jar` for everything.

| Where | Loader | Put the jar in | Also needs |
|---|---|---|---|
| Client or server | **Fabric** 26.3 | `mods/` | Fabric API |
| Client or server | **NeoForge** 26.3 | `mods/` | - |
| Client or server | **Forge** 26.3 (66.1+) | `mods/` | - |
| Server | **Paper / Purpur / Folia / Spigot** 26.3 | `plugins/` | - |
| Proxy | **Velocity** 4.3+ / **BungeeCord** | `plugins/` | Java 25 |

**Minecraft 1.21.1**: `-fabric` for Fabric and `-neoforge` for NeoForge and Forge. Either one works for Paper and proxies.

| Where | Loader | Jar | Put the jar in | Also needs |
|---|---|---|---|---|
| Client or server | **Fabric** 1.21.1 | `-fabric` | `mods/` | Fabric API |
| Client or server | **NeoForge** 21.1 | `-neoforge` | `mods/` | - |
| Client or server | **Forge** 52 | `-neoforge` | `mods/` | - |
| Server | **Paper / Purpur / Folia / Spigot** 1.21.1 | either | `plugins/` | - |
| Proxy | **Velocity** / **BungeeCord** | either | `plugins/` | Java 21 |

1. Install Ferrovoz on the **server** and on every **player**.
2. Open **UDP port 24454** on the server (configurable in `ferrovoz.properties`). If you can't, voice still works through the game connection.
3. Join and press **V** to talk.

Players without Ferrovoz can still join and play normally, just without voice.

**Supported systems:** Windows (x64), Linux (x64 and ARM64) and macOS (Apple Silicon and Intel).

### Behind a proxy (Velocity / BungeeCord)
1. Put the jar on the **proxy** and open UDP port **24454** there. It's the only port that needs to be public.
2. Put the jar on each **backend server**. If several share a machine, give each a different `port` in its `ferrovoz.properties` (24455, 24456…).
3. Done. Voice follows players when they switch servers.

---

## Controls

| Key | Action |
|---|---|
| **V** | Push to talk |
| **B** | Whisper |
| **G** | Shout |
| **M** | Mute microphone |
| **J** | Voice settings |
| *(unbound)* | Toggle push-to-talk / voice activation · Deafen |

Every key can be changed in **J → Keys…** (click an action, then press the new key).

## Commands

| Command | Who |
|---|---|
| `/voice group create <name> [password]` · `join` · `leave` · `list` | everyone |
| `/voice group kick <player>` · `pin` · `unpin` · `delete <name>` | group owner or admin |
| `/voice status` · `mute <player>` · `unmute <player>` | admin |
| `/voicenet create <name> [password]` · `join` · `leave` · `list` · `kick` · `delete` | on the proxy (network groups) |

`/voz` works too, with Spanish subcommands (`/voz grupo crear`…).

---

## Configuration

`ferrovoz.properties` (server):

```properties
port=24454            # UDP port (open it in your firewall/router)
bind_host=            # interface to listen on (empty = all)
public_host=          # host announced to clients (empty = the game's address)
max_distance=48       # normal range
whisper_distance=8    # whisper range
shout_distance=96     # shout range
transport=auto        # auto = UDP, falling back to the game connection · udp · tunnel
spectators=ghost      # spectators: ghost (default: talk among themselves) · normal · listen (only listen) · own · off
dead=normal           # dead players: same options
```

---

## FAQ

**Do players need the mod?** Yes, to talk and listen. Players without it can join the server and play normally.

**Can a Fabric client join a Paper server?** Yes. Paper/Spigot servers work with the Fabric client.

**Does it work in singleplayer / LAN?** Yes. The integrated server runs voice chat too, and **Test mic** works without any server.

**My microphone isn't picked up on macOS.** Open *System Settings → Privacy & Security → Microphone*, enable your launcher, and restart the game. Ferrovoz warns you in chat when this happens.

---

Made by **Piratemajo** · MIT License · Third-party licenses are included in the jar (`META-INF/THIRD_PARTY_LICENSES.txt`).
