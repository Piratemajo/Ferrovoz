package dev.ferrovoz.client;

import com.mojang.blaze3d.platform.InputConstants;
import dev.ferrovoz.Ferrovoz;
import dev.ferrovoz.FerrovozNative;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceSetupPayload;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import org.lwjgl.glfw.GLFW;

import java.net.InetSocketAddress;
import java.net.SocketAddress;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Set;

/**
 * Lado cliente, común a Fabric, NeoForge y Forge: teclas, conexión (UDP o túnel),
 * sensor del mundo, avisos y HUD.
 *
 * <p>Cada loader implementa {@link Platform}, registra las teclas de
 * {@link #keyMappings()}, el HUD ({@link VoiceHud#render}) y los iconos
 * ({@link SpeakerIcons}), y llama a los {@code on*} desde sus eventos.
 */
public final class FerrovozClient {
    /** Lo que depende del loader. */
    public interface Platform {
        Path configDir();

        /** Envía un paquete por la conexión del juego. Puede lanzar IllegalStateException si ya no hay partida. */
        void sendTunnel(byte[] packet);

        /** Canales que anuncia el servidor, para detectar versiones antiguas. Vacío si el loader no lo sabe. */
        Set<String> serverChannels();
    }

    /** Categoría de las teclas (en 1.21.1 es la clave de traducción). */
    public static final String CATEGORY = "key.category." + Ferrovoz.MOD_ID + ".voice";
    public static final ResourceLocation HUD_ID = ResourceLocation.fromNamespaceAndPath(Ferrovoz.MOD_ID, "hud");

    private static Platform platform;
    private static final List<KeyMapping> keys = new ArrayList<>();
    private static KeyMapping pushToTalk;
    private static KeyMapping whisper;
    private static KeyMapping shout;
    private static KeyMapping toggleMode;
    private static KeyMapping toggleMute;
    private static KeyMapping toggleDeafen;
    private static KeyMapping openSettings;

    private static ClientConfig config;

    private static volatile long client; // handle del VoiceClient (0 = desconectado)
    private static boolean muted;
    private static boolean deafened;
    private static int talkMode = FerrovozNative.TALK_NORMAL;
    private static long[] speaking = new long[0];
    private static final WorldSensor sensor = new WorldSensor();

    // --- Avisos: versión distinta, sin librería nativa, micro bloqueado ---
    /** Ticks tras entrar antes de comprobar la versión del servidor. */
    private static final int LEGACY_CHECK_TICKS = 100;
    private static boolean nativeLoaded;
    /** Ticks en el servidor actual (-1 = ninguno). */
    private static int ticksSinceJoin = -1;
    /** Lo recibido por {@code ferrovoz:version}, o null. */
    private static volatile VersionPayload serverVersion;
    private static volatile boolean setupReceived;
    /** Últimos datos de conexión, para reactivar la voz desde los ajustes. */
    private static volatile VoiceSetupPayload lastSetup;
    private static boolean nativeWarned;
    private static boolean micWarned;
    /** Clave de traducción del problema actual para el HUD (null = ninguno). */
    private static volatile String problem;

    private FerrovozClient() {}

    /** Al cargar el mod. Si falta la librería nativa el juego funciona y se avisa al entrar. */
    public static void init(Platform p) {
        platform = p;
        nativeLoaded = FerrovozNative.load();
        if (!nativeLoaded) return;
        config = ClientConfig.load(p.configDir());

        // En 1.21.1 las teclas son códigos de GLFW.
        pushToTalk = key("push_to_talk", GLFW.GLFW_KEY_V);
        whisper = key("whisper", GLFW.GLFW_KEY_B);
        shout = key("shout", GLFW.GLFW_KEY_G);
        toggleMode = key("toggle_mode", GLFW.GLFW_KEY_UNKNOWN);
        toggleMute = key("toggle_mute", GLFW.GLFW_KEY_M);
        toggleDeafen = key("toggle_deafen", GLFW.GLFW_KEY_UNKNOWN);
        openSettings = key("open_settings", GLFW.GLFW_KEY_J);
    }

    private static KeyMapping key(String name, int defaultKey) {
        KeyMapping k = new KeyMapping("key.ferrovoz." + name, InputConstants.Type.KEYSYM, defaultKey, CATEGORY);
        keys.add(k);
        return k;
    }

    /** Teclas a registrar (vacío sin librería nativa). */
    public static List<KeyMapping> keyMappings() {
        return keys;
    }

    // --- Eventos (los llama cada loader) ---

    /** Datos de conexión del servidor. */
    public static void onSetup(VoiceSetupPayload payload) {
        // En una repetición llega el mensaje grabado: no hay que conectar.
        if (ReplayCompat.inReplay()) return;
        setupReceived = true;
        lastSetup = payload;
        // Voz desactivada: no se conecta y el servidor nos ve sin voz.
        if (nativeLoaded && !config.voiceDisabled) connect(payload, Minecraft.getInstance());
    }

    /** Hay voz en el servidor pero el jugador la ha desactivado (para el HUD). */
    static boolean voiceOffHere() {
        return nativeLoaded && config.voiceDisabled && lastSetup != null;
    }

    /**
     * Activar o desactivar la voz desde los ajustes. Al desactivar se avisa al
     * servidor y se liberan los dispositivos; al activar se reconecta con los últimos
     * datos recibidos.
     */
    static void setVoiceEnabled(boolean enabled) {
        if (!nativeLoaded || config.voiceDisabled == !enabled) return;
        config.voiceDisabled = !enabled;
        config.save();
        if (!enabled) {
            disconnect(true);
        } else if (lastSetup != null && Minecraft.getInstance().getConnection() != null) {
            connect(lastSetup, Minecraft.getInstance());
        }
    }

    /** Voz por la conexión del juego (sin UDP). */
    public static void onTunnel(byte[] packet) {
        long h = client;
        if (h != 0 && packet != null) FerrovozNative.clientHandleTunnel(h, packet);
    }

    /** Versión de Ferrovoz del servidor. */
    public static void onVersion(VersionPayload payload) {
        if (ReplayCompat.inReplay()) return;
        onServerVersion(payload, Minecraft.getInstance());
    }

    // --- Repeticiones (Replay Mod, Flashback) ---

    /** Mezclador de la repetición actual (0 = ninguna). */
    private static long replay;
    private static final ReplayAudioOut replayOut = new ReplayAudioOut();
    /** Quién ha hablado hace poco en la repetición (UUID -> último instante, ms). */
    private static final java.util.Map<java.util.UUID, Long> replaySpeakers = new java.util.HashMap<>();
    private static boolean recordingNow;
    /** 50 ms de audio por tick. */
    private static final int REPLAY_TICK_FRAMES = 2_400;

    /** Trama de voz grabada en la repetición. */
    public static void onReplayVoice(dev.ferrovoz.ReplayVoicePayload payload) {
        if (!nativeLoaded || !ReplayCompat.inReplay()) return;
        byte[] frame = payload.frame();
        if (frame == null || frame.length < 16) return; // formato desconocido
        if (replay == 0) {
            replay = FerrovozNative.replayStart();
            if (replay == 0) return;
            config.applyTo(replay); // volumen, HRTF, reverb... (los dispositivos no se usan)
        }
        if (FerrovozNative.replayFeed(replay, frame)) {
            java.nio.ByteBuffer id = java.nio.ByteBuffer.wrap(frame, 0, 16);
            replaySpeakers.put(new java.util.UUID(id.getLong(), id.getLong()), System.currentTimeMillis());
        }
    }

    /** Cada tick de repetición: la cámara es el oyente y el audio sale por Minecraft. */
    private static void replayTick(Minecraft mc) {
        if (replay == 0 || mc.level == null) return;
        net.minecraft.world.entity.Entity camera = mc.getCameraEntity() != null ? mc.getCameraEntity() : mc.player;
        if (camera == null) return;
        net.minecraft.world.phys.Vec3 eye = camera.getEyePosition();
        FerrovozNative.clientTick(replay, camera.getYRot(), camera.getXRot(), false, FerrovozNative.TALK_NORMAL);
        long now = System.currentTimeMillis();
        replaySpeakers.values().removeIf(t -> now - t > 3_000);
        for (java.util.UUID id : replaySpeakers.keySet()) {
            for (net.minecraft.world.entity.player.Player p : mc.level.players()) {
                if (!p.getUUID().equals(id)) continue;
                net.minecraft.world.phys.Vec3 e = p.getEyePosition();
                FerrovozNative.replaySetPosition(replay, id.getMostSignificantBits(), id.getLeastSignificantBits(),
                        (float) (e.x - eye.x), (float) (e.y - eye.y), (float) (e.z - eye.z));
            }
        }
        long[] who = FerrovozNative.clientSpeakingPlayers(replay);
        speaking = who == null ? new long[0] : who;
        sensor.tick(mc, replay, speaking, eye, camera);
        if (replayOut.wantsPrebuffer()) {
            replayOut.push(FerrovozNative.replayRender(replay, REPLAY_TICK_FRAMES));
        }
        replayOut.push(FerrovozNative.replayRender(replay, REPLAY_TICK_FRAMES));
    }

    private static void stopReplay() {
        if (replay != 0) {
            FerrovozNative.clientStop(replay);
            replay = 0;
            replayOut.close();
            replaySpeakers.clear();
            speaking = new long[0];
        }
    }

    /** Si Replay Mod o Flashback están grabando, les pasa la voz de este tick. */
    private static void recordTick() {
        boolean rec = ReplayCompat.recording();
        if (rec != recordingNow) {
            FerrovozNative.clientSetRecording(client, rec);
            recordingNow = rec;
        }
        if (!rec) return;
        byte[][] frames = FerrovozNative.clientPollRecorded(client);
        if (frames == null) return;
        for (byte[] f : frames) {
            ReplayCompat.record(new net.minecraft.network.protocol.common.ClientboundCustomPayloadPacket(dev.ferrovoz.ReplayVoicePayload.of(f)));
        }
    }

    /** Al entrar en un servidor o mundo local. */
    public static void onJoin() {
        onJoinServer(Minecraft.getInstance());
    }

    /** Al salir del servidor. */
    public static void onDisconnect() {
        ticksSinceJoin = -1;
        serverVersion = null;
        setupReceived = false;
        lastSetup = null;
        problem = null;
        if (nativeLoaded) {
            disconnect();
            stopReplay();
        }
    }

    /** Al final de cada tick del cliente. */
    public static void onTick(Minecraft mc) {
        noticeTick(mc);
        if (nativeLoaded) tick(mc);
    }

    // --- Avisos ---

    static boolean isMac() {
        return System.getProperty("os.name", "").toLowerCase(Locale.ROOT).contains("mac");
    }

    /** Problema actual para el HUD (clave de traducción), o null. */
    static String problem() {
        return problem;
    }

    private static void onJoinServer(Minecraft mc) {
        // Detrás de un proxy, cambiar de servidor es volver a entrar sin desconectar:
        // se suelta la voz del anterior. Si el nuevo tiene Ferrovoz enviará sus datos.
        if (nativeLoaded) disconnect();
        serverVersion = null;
        setupReceived = false;
        lastSetup = null;
        problem = null;
        ticksSinceJoin = 0;
        if (!nativeLoaded && !nativeWarned) {
            nativeWarned = true;
            chat(mc, Component.translatable("ferrovoz.notice.native_missing", FerrovozNative.platform()));
        }
        if (!nativeLoaded) problem = "ferrovoz.hud.unavailable";
    }

    private static void onServerVersion(VersionPayload v, Minecraft mc) {
        serverVersion = v;
        String mine = Protocol.modVersion();
        if (v.protocol() != Protocol.VERSION) {
            problem = "ferrovoz.hud.incompatible";
            chat(mc, Component.translatable(v.protocol() > Protocol.VERSION
                    ? "ferrovoz.notice.server_newer" : "ferrovoz.notice.server_older", v.version(), mine));
        } else if (!v.voiceAvailable()) {
            problem = "ferrovoz.hud.server_down";
            chat(mc, Component.translatable("ferrovoz.notice.server_no_voice"));
        }
    }

    /** Cada tick: servidores anteriores a 0.6 y micro bloqueado. */
    private static void noticeTick(Minecraft mc) {
        if (ticksSinceJoin < 0 || mc.player == null || ReplayCompat.inReplay()) return;
        if (ticksSinceJoin < LEGACY_CHECK_TICKS && ++ticksSinceJoin == LEGACY_CHECK_TICKS) {
            checkLegacyServer(mc);
        }
        long h = client;
        if (h != 0 && !micWarned && FerrovozNative.clientMicSilent(h)) {
            micWarned = true; // una vez por sesión: arreglarlo requiere reiniciar
            chat(mc, Component.translatable(isMac() ? "ferrovoz.notice.mic_blocked_mac" : "ferrovoz.notice.mic_blocked"));
        }
    }

    /**
     * Los servidores anteriores a 0.6 no envían su versión pero anuncian sus canales
     * ({@code ferrovoz:voice_vN}). Si el número no coincide, se avisa en vez de
     * quedarse en "conectando..." indefinidamente.
     */
    private static void checkLegacyServer(Minecraft mc) {
        if (setupReceived || serverVersion != null) return;
        Set<String> channels;
        try {
            channels = platform.serverChannels();
        } catch (IllegalStateException e) {
            return; // ya desconectado
        }
        int theirs = Protocol.protocolOf(channels);
        if (theirs > 0 && theirs != Protocol.VERSION) {
            problem = "ferrovoz.hud.incompatible";
            chat(mc, Component.translatable(theirs > Protocol.VERSION
                    ? "ferrovoz.notice.server_newer_legacy" : "ferrovoz.notice.server_older_legacy", theirs, Protocol.modVersion()));
        }
    }

    private static void chat(Minecraft mc, Component message) {
        mc.gui.getChat().addMessage(
                Component.literal("[Ferrovoz] ").withStyle(net.minecraft.ChatFormatting.GOLD).append(message));
    }

    static ClientConfig config() {
        return config;
    }

    static long handle() {
        return client;
    }

    static boolean isMuted() {
        return muted;
    }

    static boolean isDeafened() {
        return deafened;
    }

    static int talkMode() {
        return talkMode;
    }

    /** Estado de voz de los demás ({@code [msb, lsb, flags, ...]}), o null si el servidor no lo envía. */
    private static volatile long[] peerStates;

    static long[] peerStates() {
        return peerStates;
    }

    /** Último {@code [msb, lsb, grupo, ...]} de quien habla (cada tick). */
    static long[] speaking() {
        return speaking;
    }

    private static void connect(VoiceSetupPayload payload, Minecraft mc) {
        disconnect();
        String host = payload.host().isEmpty() ? gameServerHost(mc) : payload.host();
        // UUID asignada por el servidor, no la de la cuenta: en modo offline o detrás
        // de un proxy pueden ser distintas.
        client = FerrovozNative.clientStart(host, payload.port(), payload.idMsb(), payload.idLsb(),
                payload.key(), 48f, payload.transport());
        if (client != 0) {
            startTunnelSender(client);
            config.applyTo(client); // dispositivos, volúmenes, modo, denoiser
            FerrovozNative.clientSetMuted(client, muted);
            FerrovozNative.clientSetDeafened(client, deafened);
        }
    }

    /** Host con el que se entró; en un mundo local, localhost. */
    private static String gameServerHost(Minecraft mc) {
        if (mc.getConnection() != null) {
            SocketAddress addr = mc.getConnection().getConnection().getRemoteAddress();
            if (addr instanceof InetSocketAddress inet) {
                return inet.getHostString();
            }
        }
        return "127.0.0.1";
    }

    private static void disconnect() {
        disconnect(false);
    }

    /** @param goodbye avisar al servidor de que se deja la voz (false al salir del servidor) */
    private static void disconnect(boolean goodbye) {
        stopTunnelSender();
        recordingNow = false; // el cliente nuevo empieza sin grabar
        if (client != 0) {
            if (goodbye) {
                // Por UDP sale ya; por túnel queda en cola y se envía aquí.
                FerrovozNative.clientGoodbye(client);
                byte[][] pending = FerrovozNative.clientPollTunnel(client);
                if (pending != null) {
                    for (byte[] packet : pending) {
                        try {
                            platform.sendTunnel(packet);
                        } catch (IllegalStateException e) {
                            break; // fuera de la partida
                        }
                    }
                }
            }
            FerrovozNative.clientStop(client);
            client = 0;
        }
        speaking = new long[0];
        peerStates = null;
    }

    private static void tick(Minecraft mc) {
        if (mc.player == null) return;

        // La pantalla de ajustes funciona también sin servidor (modo prueba).
        while (openSettings.consumeClick()) {
            mc.setScreen(new VoiceSettingsScreen(null));
        }
        if (client == 0) {
            replayTick(mc);
            return;
        }
        recordTick();

        // Susurrar y gritar también funcionan como pulsar para hablar.
        boolean whispering = whisper.isDown(), shouting = shout.isDown();
        talkMode = whispering ? FerrovozNative.TALK_WHISPER
                : shouting ? FerrovozNative.TALK_SHOUT : FerrovozNative.TALK_NORMAL;
        FerrovozNative.clientTick(client, mc.player.getYRot(), mc.player.getXRot(), pushToTalk.isDown() || whispering || shouting, talkMode);
        FerrovozNative.clientSetUnderwater(client, mc.player.isUnderWater());

        long[] now = FerrovozNative.clientSpeakingPlayers(client);
        speaking = now == null ? new long[0] : now;
        peerStates = FerrovozNative.clientPeerStates(client);
        sensor.tick(mc, client, speaking, mc.player.getEyePosition(), mc.player);

        while (toggleMode.consumeClick()) {
            int mode = FerrovozNative.clientGetMode(client) == 0 ? 1 : 0;
            FerrovozNative.clientSetMode(client, mode);
            config.mode = mode;
            config.save();
            notify(mc, mode == 1 ? "ferrovoz.mode.voice_activation" : "ferrovoz.mode.push_to_talk");
        }
        while (toggleMute.consumeClick()) {
            muted = !muted;
            FerrovozNative.clientSetMuted(client, muted);
            notify(mc, muted ? "ferrovoz.muted" : "ferrovoz.unmuted");
        }
        while (toggleDeafen.consumeClick()) {
            deafened = !deafened;
            FerrovozNative.clientSetDeafened(client, deafened);
            notify(mc, deafened ? "ferrovoz.deafened" : "ferrovoz.undeafened");
        }
    }

    // --- Túnel: envío desde un hilo propio ---
    // Cada 5 ms en vez de cada tick (50 ms) para bajar la latencia. Los envíos acaban
    // en el canal de Netty, que admite llamadas desde cualquier hilo.

    private static volatile Thread tunnelSender;

    private static void startTunnelSender(long handle) {
        stopTunnelSender();
        Thread t = new Thread(() -> {
            while (!Thread.currentThread().isInterrupted() && client == handle) {
                byte[][] outgoing = FerrovozNative.clientPollTunnel(handle);
                if (outgoing != null) {
                    for (byte[] packet : outgoing) {
                        try {
                            platform.sendTunnel(packet);
                        } catch (IllegalStateException e) {
                            return; // fuera de la partida
                        }
                    }
                }
                try {
                    Thread.sleep(5);
                } catch (InterruptedException e) {
                    return;
                }
            }
        }, "ferrovoz-tunnel-client");
        t.setDaemon(true);
        tunnelSender = t;
        t.start();
    }

    private static void stopTunnelSender() {
        Thread t = tunnelSender;
        tunnelSender = null;
        if (t != null) {
            t.interrupt();
            try {
                t.join(200); // que no use el handle después de liberarlo
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
    }

    private static void notify(Minecraft mc, String key) {
        mc.gui.setOverlayMessage(Component.translatable(key), false);
    }
}
