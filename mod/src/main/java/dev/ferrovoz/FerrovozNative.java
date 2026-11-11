package dev.ferrovoz;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.Locale;

/**
 * Declaraciones JNI del núcleo en Rust ({@code native/src/lib.rs}).
 */
public final class FerrovozNative {
    private static boolean loaded;
    private static boolean failed;

    private FerrovozNative() {}

    /** Carga la librería una sola vez. false si no hay binario para esta plataforma. */
    public static boolean load() {
        return load(isDedicatedServer());
    }

    /**
     * @param dedicatedServer true en servidores dedicados: usa la variante sin audio,
     *                        que no necesita ALSA.
     */
    public static synchronized boolean load(boolean dedicatedServer) {
        if (loaded) return true;
        if (failed) return false; // no reintentar ni repetir el error
        failed = true;
        try {
            String override = System.getProperty("ferrovoz.native.path");
            if (override != null) {
                System.load(Path.of(override).toAbsolutePath().toString());
            } else {
                // En servidor dedicado, la variante sin audio (sin ALSA/CoreAudio/WASAPI,
                // que en una máquina sin escritorio pueden no estar). Si no está, la completa.
                String[] candidates = dedicatedServer
                        ? new String[] {"ferrovoz_server", "ferrovoz"}
                        : new String[] {"ferrovoz"};
                String file = null;
                InputStream found = null;
                for (String name : candidates) {
                    file = System.mapLibraryName(name);
                    found = FerrovozNative.class.getResourceAsStream("/natives/" + platform() + "/" + file);
                    if (found != null) break;
                }
                try (InputStream in = found) {
                    if (in == null) {
                        FerrovozLog.LOG.warn("Ferrovoz has no native library for {}: the game works, but without voice chat. "
                                + "Supported platforms: windows-x86_64, linux-x86_64, linux-aarch64, macos-aarch64, macos-x86_64.",
                                platform());
                        return false;
                    }
                    Path dir = Files.createTempDirectory("ferrovoz-");
                    Path lib = dir.resolve(file);
                    Files.copy(in, lib, StandardCopyOption.REPLACE_EXISTING);
                    lib.toFile().deleteOnExit();
                    dir.toFile().deleteOnExit();
                    System.load(lib.toAbsolutePath().toString());
                }
            }
            String nativeVersion = version(); // de paso comprueba que enlaza
            loaded = true;
            failed = false;
            FerrovozLog.LOG.info("Ferrovoz native core loaded (v{}, {})", nativeVersion, platform());
            return true;
        } catch (IOException | LinkageError | SecurityException e) {
            // LinkageError incluye UnsatisfiedLinkError: musl (Alpine) o glibc demasiado
            // antigua. El juego sigue, sin voz.
            FerrovozLog.LOG.warn("Could not load the Ferrovoz native core on {}: the game works, but without voice chat",
                    platform(), e);
            return false;
        }
    }

    /**
     * Servidor dedicado en Fabric, NeoForge o Forge. Por reflexión, para no depender de
     * ningún loader (esta clase también la usa el plugin de Bukkit).
     */
    private static boolean isDedicatedServer() {
        // Fabric: FabricLoader.getInstance().getEnvironmentType() == SERVER
        try {
            Class<?> loader = Class.forName("net.fabricmc.loader.api.FabricLoader");
            Object instance = loader.getMethod("getInstance").invoke(null);
            Object env = loader.getMethod("getEnvironmentType").invoke(instance);
            return "SERVER".equals(String.valueOf(env));
        } catch (ReflectiveOperationException | LinkageError e) {
            // no es Fabric
        }
        // NeoForge: FMLEnvironment.getDist() == DEDICATED_SERVER (en 21.1 es el campo dist)
        try {
            Class<?> env = Class.forName("net.neoforged.fml.loading.FMLEnvironment");
            Object dist;
            try {
                dist = env.getMethod("getDist").invoke(null);
            } catch (NoSuchMethodException e) {
                dist = env.getField("dist").get(null);
            }
            return "DEDICATED_SERVER".equals(String.valueOf(dist));
        } catch (ReflectiveOperationException | LinkageError e) {
            // no es NeoForge
        }
        // Forge: FMLEnvironment.dist == DEDICATED_SERVER
        try {
            Object dist = Class.forName("net.minecraftforge.fml.loading.FMLEnvironment").getField("dist").get(null);
            return "DEDICATED_SERVER".equals(String.valueOf(dist));
        } catch (ReflectiveOperationException | LinkageError e) {
            return false;
        }
    }

    public static boolean isLoaded() {
        return loaded;
    }

    /** p. ej. {@code windows-x86_64}, {@code linux-aarch64}, {@code macos-aarch64}. */
    public static String platform() {
        String os = System.getProperty("os.name").toLowerCase(Locale.ROOT);
        String arch = System.getProperty("os.arch").toLowerCase(Locale.ROOT);
        String o = os.contains("win") ? "windows" : os.contains("mac") ? "macos" : "linux";
        String a = (arch.equals("amd64") || arch.equals("x86_64")) ? "x86_64"
                : (arch.equals("aarch64") || arch.equals("arm64")) ? "aarch64" : arch;
        return o + "-" + a;
    }

    // --- General ---
    public static native String version();

    // --- Servidor ---
    public static native long serverStart(String bindHost, int port, float maxDistance);
    public static native void serverStop(long handle);
    /** Registra al jugador y devuelve su clave de 32 bytes (o null si falla). */
    public static native byte[] serverAddPlayer(long handle, long uuidMsb, long uuidLsb);
    public static native void serverRemovePlayer(long handle, long uuidMsb, long uuidLsb);
    /**
     * @param underwater cabeza bajo el agua
     * @param role {@link #ROLE_ALIVE}, {@link #ROLE_SPECTATOR} o {@link #ROLE_DEAD}
     */
    public static native void serverUpdatePlayers(long handle, long[] ids, double[] positions, int[] info, int count);

    public static native void serverUpdatePlayer(long handle, long uuidMsb, long uuidLsb,
                                                 double x, double y, double z, int dimension, boolean underwater, int role);

    public static final int ROLE_ALIVE = 0;
    public static final int ROLE_SPECTATOR = 1;
    public static final int ROLE_DEAD = 2;

    /** Reglas para espectadores y muertos: ver {@link FerrovozConfig#spectators}. */
    public static native void serverSetRoleRules(long handle, int spectators, int dead);
    public static native void serverSetMuted(long handle, long uuidMsb, long uuidLsb, boolean muted);
    public static native void serverSetMaxDistance(long handle, float maxDistance);
    public static native int serverConnectedCount(long handle);

    // --- Cliente ---
    /** @param transport ver {@link Protocol#TRANSPORT_AUTO}. */
    public static native long clientStart(String host, int port, long uuidMsb, long uuidLsb,
                                          byte[] key, float maxDistance, int transport);
    public static native void clientStop(long handle);
    /**
     * Una vez por tick: yaw y pitch del jugador (grados), tecla de hablar y modo
     * ({@link #TALK_NORMAL}, {@link #TALK_WHISPER}, {@link #TALK_SHOUT}).
     */
    public static native void clientTick(long handle, float yaw, float pitch, boolean pushToTalk, int talkMode);

    /** HRTF o paneo estéreo simple. */
    public static native void clientSetHrtf(long handle, boolean enabled);

    /** Volumen de la voz de grupo (1.0 = 100 %). */
    public static native void clientSetGroupVolume(long handle, float volume);

    public static final int PEER_MUTED = 1;
    public static final int PEER_DEAFENED = 2;
    public static final int PEER_CONNECTED = 4;

    /**
     * {@code [msb, lsb, flags, ...]} con el estado de voz de cada jugador
     * ({@link #PEER_MUTED}, {@link #PEER_DEAFENED}, {@link #PEER_CONNECTED}). Quien no
     * aparece no tiene el mod. null si el servidor no lo envía (anterior a 0.10).
     */
    public static native long[] clientPeerStates(long handle);

    // --- Repeticiones (Replay Mod, Flashback) ---
    /** Activa la grabación de tramas (propias y recibidas). */
    public static native void clientSetRecording(long handle, boolean on);
    /** Tramas grabadas desde la última llamada (ver {@link ReplayVoicePayload#of}). */
    public static native byte[][] clientPollRecorded(long handle);
    /** Mezclador para reproducir repeticiones, sin red ni dispositivos. Se libera con {@link #clientStop}. 0 si falla. */
    public static native long replayStart();
    /** Una trama grabada; false si no se reconoce. */
    public static native boolean replayFeed(long handle, byte[] frame);
    /** Posición del hablante relativa a la cámara (bloques, ejes del mundo). */
    public static native void replaySetPosition(long handle, long uuidMsb, long uuidLsb, float x, float y, float z);
    /** Mezcla {@code frames} muestras estéreo a 48 kHz (intercaladas). */
    public static native float[] replayRender(long handle, int frames);

    public static final int TALK_NORMAL = 0;
    public static final int TALK_WHISPER = 1;
    public static final int TALK_SHOUT = 2;
    /** 0 = pulsar para hablar, 1 = activación por voz. */
    public static native void clientSetMode(long handle, int mode);
    public static native int clientGetMode(long handle);
    public static native void clientSetMuted(long handle, boolean muted);
    public static native void clientSetDeafened(long handle, boolean deafened);
    public static native void clientSetVolume(long handle, float volume);
    public static native void clientSetMicGain(long handle, float gain);
    public static native void clientSetVadThreshold(long handle, float threshold);
    public static native boolean clientIsTalking(long handle);
    public static native boolean clientIsConnected(long handle);
    public static native float clientMicLevel(long handle);
    /** 3 s de ceros exactos en el micro: permiso denegado (macOS) o mute por hardware. */
    public static native boolean clientMicSilent(long handle);
    /** {@code [msb0, lsb0, grupo0, msb1, lsb1, grupo1, ...]} de quien habla ahora (grupo = 1 si es voz de grupo). */
    public static native long[] clientSpeakingPlayers(long handle);

    // --- Ajustes de audio ---
    /** Cliente sin red para probar micro y cascos. Se para con {@link #clientStop}. */
    public static native long previewStart();
    /** Micros del sistema (vacío en la variante de servidor). */
    public static native String[] listInputDevices();
    /** Salidas del sistema. */
    public static native String[] listOutputDevices();
    /** Cambia los dispositivos en caliente; cadena vacía = predeterminado. */
    public static native void clientSetDevices(long handle, String input, String output);
    /** 0 = abriendo, 1 = abiertos, 2 = error. */
    public static native int clientAudioStatus(long handle);
    /** Monitor de micro (no se envía). */
    public static native void clientSetMonitor(long handle, boolean monitor);
    /** Paso alto + RNNoise + puerta de ruido. */
    public static native void clientSetNoiseSuppression(long handle, boolean enabled);

    // --- Volumen por jugador, oclusión, reverb, túnel, red ---
    /** Volumen por jugador: 1 normal, 0 silenciado, hasta 2. */
    public static native void clientSetPlayerVolume(long handle, long uuidMsb, long uuidLsb, float volume);
    /** Oclusión de un jugador (0..1). */
    public static native void clientSetOcclusion(long handle, long uuidMsb, long uuidLsb, float amount);
    /** Reverb local: cantidad y tamaño (0..1). */
    public static native void clientSetEnvironment(long handle, float wet, float room);

    /**
     * Acústica de la sala: wet (0..1), {@code room} (0..1), absorción de agudos (0..1) y
     * reflexiones tempranas en grupos de 6
     * {@code [dx, dy, dz, retardo s, ganancia, absorción de la pared]}.
     */
    public static native void clientSetAcoustics(long handle, float wet, float room, float damp, float[] reflections);

    /**
     * Camino alternativo (puerta, esquina) para la voz de un jugador tapado: dirección
     * del hueco (mundo, unitaria), longitud en bloques y esquinas. {@code has = false} lo borra.
     */
    public static native void clientSetSoundPath(long handle, long uuidMsb, long uuidLsb, boolean has,
                                                 float dirX, float dirY, float dirZ, float length, int corners);
    /** {@code [rtt ms, pérdida %, bitrate, túnel 0/1]}. */
    public static native float[] clientNetStats(long handle);
    public static native void clientHandleTunnel(long handle, byte[] packet);
    public static native byte[][] clientPollTunnel(long handle);

    public static native void serverHandleTunnel(long handle, long uuidMsb, long uuidLsb, byte[] packet);
    public static native byte[][] serverPollTunnel(long handle, long uuidMsb, long uuidLsb);
    public static native void serverSetRanges(long handle, float normal, float whisper, float shout);
    public static native void serverSetPermissions(long handle, long uuidMsb, long uuidLsb, boolean canSpeak, boolean canShout);
    /** Su voz llega siempre por separado, aunque haya multitud ({@code ferrovoz.priority}). */
    public static native void serverSetPriority(long handle, long uuidMsb, long uuidLsb, boolean priority);
    /** Voces por separado como mucho en cada zona; las demás se mezclan en el murmullo (0 = sin límite). */
    public static native void serverSetMaxVoices(long handle, int maxVoices);

    /** Eco de cuevas y salas para todos; los clientes se enteran en 2 s como mucho. */
    public static native void serverSetCaveEcho(long handle, boolean enabled);

    /** Efecto bajo el agua para todos; los clientes se enteran en 2 s como mucho. */
    public static native void serverSetUnderwaterEffect(long handle, boolean enabled);
    /** Varias oclusiones de golpe: {@code pairs} = [hablante msb, lsb, oyente msb, lsb, ...], {@code amounts} una por pareja. */
    public static native void serverSetOcclusions(long handle, long[] pairs, float[] amounts);
    public static native boolean serverIsMuted(long handle, long uuidMsb, long uuidLsb);
    /** {@code [conectados, por túnel, grupos, in, out, rechazados, replay, limitados, ns de CPU del relé]}. */
    public static native long[] serverStatus(long handle);
    /** Pérdida (%) en la subida del jugador, o -1. */
    public static native int serverLossPercent(long handle, long uuidMsb, long uuidLsb);
    /** 0 = ok, 1 nombre no válido, 2 ya existe, 3 no existe, 4 contraseña incorrecta, 5 jugador sin voz,
     *  6 no es el dueño, 7 no está en ningún grupo. */
    public static native int serverCreateGroup(long handle, long uuidMsb, long uuidLsb, String name, String password);
    public static native int serverJoinGroup(long handle, long uuidMsb, long uuidLsb, String name, String password);
    public static native boolean serverLeaveGroup(long handle, long uuidMsb, long uuidLsb);
    /** Nombre del grupo del jugador, o null. */
    public static native String serverGroupOf(long handle, long uuidMsb, long uuidLsb);
    /** {@code ["nombre|miembros|candado(0/1)|permanente(0/1)", ...]}. */
    public static native String[] serverListGroups(long handle);

    // --- Grupos con dueño y permanentes, persistencia, oclusión del servidor ---
    /** Fichero de silencios y grupos permanentes; se carga al llamarlo. */
    public static native boolean serverSetDataFile(long handle, String path);
    public static native int serverKickFromGroup(long handle, long requesterMsb, long requesterLsb,
                                                 long targetMsb, long targetLsb, boolean admin);
    public static native int serverSetGroupPersistent(long handle, long uuidMsb, long uuidLsb, boolean persistent, boolean admin);
    public static native int serverDeleteGroup(long handle, long uuidMsb, long uuidLsb, String name, boolean admin);
    /** {@code [msb, lsb]} del dueño, o vacío. */
    public static native long[] serverGroupOwner(long handle, String name);
    /** {@code [msb0, lsb0, ...]} de los miembros conectados. */
    public static native long[] serverGroupMembers(long handle, String name);
    /**
     * {@code ["tipo|uuid|detalle", ...]}, en orden. Tipos: join, leave, kick, rejoin, owner,
     * delete, dissolve (detalle = grupo), connect, disconnect, silent y speak (detalle = normal,
     * whisper, shout o group).
     */
    public static native String[] serverPollEvents(long handle);
    /** {@code [hablaMsb, hablaLsb, oyeMsb, oyeLsb, ...]}: pares activos (para la oclusión). */
    public static native long[] serverActivePairs(long handle);
    public static native void serverSetOcclusion(long handle, long speakerMsb, long speakerLsb,
                                                 long listenerMsb, long listenerLsb, float amount);
    // --- Efectos que el jugador puede desactivar ---
    /** Reverb en cuevas y salas. */
    public static native void clientSetCaveEcho(long handle, boolean enabled);

    /** false si el servidor ha desactivado el eco para todos. */
    public static native boolean clientCaveEchoAllowed(long handle);

    /** false si el servidor ha desactivado el efecto bajo el agua para todos. */
    public static native boolean clientUnderwaterAllowed(long handle);
    /** Efecto submarino. */
    public static native void clientSetUnderwaterEffect(long handle, boolean enabled);

    /** AEC y AGC del micro. */
    public static native void clientSetMicProcessing(long handle, boolean echoCancel, boolean autoGain);

    /** Avisa al servidor de que se deja la voz (antes de {@link #clientStop}). */
    public static native void clientGoodbye(long handle);
    /** Cabeza bajo el agua (cada tick). */
    public static native void clientSetUnderwater(long handle, boolean underwater);

    // --- Reenviador para proxies (Velocity, BungeeCord) ---
    /** Abre el puerto UDP público del proxy. 0 si no se pudo. */
    public static native long proxyStart(String bindHost, int port);
    public static native void proxyStop(long handle);
    /** El jugador (UUID de voz y clave) está en el servidor con relé {@code host:port}. false si no se resuelve. */
    public static native boolean proxyRoute(long handle, long uuidMsb, long uuidLsb, byte[] key, String host, int port);
    public static native void proxyRemove(long handle, long uuidMsb, long uuidLsb);

    /**
     * Grupo de red: miembros por UUID de voz ({@code ids} = pares msb, lsb) y nombres.
     * Dueño 0,0 = ninguno. Sin miembros se borra.
     */
    public static native void proxySetNetGroup(long handle, int group, String name, long ownerMsb, long ownerLsb,
                                               long[] ids, String[] names);
    /** {@code [rutas, subidos, bajados, rechazados, replay, voz de grupos de red]}. */
    public static native long[] proxyStatus(long handle);

    // --- API para plugins (dev.ferrovoz.api) ---
    /** Bits de {@link #serverPlayerState}. */
    public static final int STATE_REGISTERED = 1, STATE_CONNECTED = 2, STATE_SPEAKING = 4,
            STATE_SELF_MUTED = 8, STATE_DEAFENED = 16;

    /** Bits {@code STATE_*}; 0 si no está registrado. */
    public static native int serverPlayerState(long handle, long uuidMsb, long uuidLsb);
    /** {@code [msb0, lsb0, ...]} de los jugadores registrados. */
    public static native long[] serverPlayers(long handle);
    /** Alcance propio en bloques; {@code <= 0} lo quita. */
    public static native void serverSetRangeOverride(long handle, long uuidMsb, long uuidLsb, float range);
    public static native void serverSetHush(long handle, boolean hush);
    public static native boolean serverIsHushed(long handle);
    /** Como {@link #serverJoinGroup}, sin contraseña. */
    public static native int serverForceJoinGroup(long handle, long uuidMsb, long uuidLsb, String name);
    /** Códigos como {@link #serverCreateGroup}. */
    public static native int serverSetGroupPersistentByName(long handle, String name, boolean persistent);
    /** {@code [dueñoMsb, dueñoLsb, flags]}: tiene dueño 1, contraseña 2, permanente 4. null si no existe. */
    public static native long[] serverGroupInfo(long handle, String name);
    public static native void serverSetMicTap(long handle, boolean on);
    /**
     * Tramas del micro pendientes, seguidas (little endian):
     * {@code uuid(16) seq(4) modo(1) flags(1) largo(2) opus}. flags 1 = grupo. null si no hay.
     */
    public static native byte[] serverPollMicrophone(long handle);
    /** Canal de audio nuevo, sin oyentes: {@code [msb, lsb]}, o null si no se pudo. */
    public static native long[] serverChannelCreate(long handle, float range, int bitrate, boolean music);
    public static native boolean serverChannelPoint(long handle, long msb, long lsb, double x, double y, double z, int dimension);
    public static native boolean serverChannelFollow(long handle, long msb, long lsb, long playerMsb, long playerLsb);
    /** {@code players} = pares msb, lsb. */
    public static native boolean serverChannelListeners(long handle, long msb, long lsb, long[] players);
    public static native boolean serverChannelGroup(long handle, long msb, long lsb, String group);
    public static native boolean serverChannelRange(long handle, long msb, long lsb, float range);
    public static native void serverChannelRemove(long handle, long msb, long lsb);
    /** 960 muestras por {@code gain}. false si el canal no existe o la trama no vale. */
    public static native boolean serverChannelSendPcm(long handle, long msb, long lsb, short[] pcm, float gain);
    public static native boolean serverChannelSendOpus(long handle, long msb, long lsb, byte[] opus);
    /** 0 si Opus falla. */
    public static native long opusEncoderCreate(int bitrate, boolean music);
    /** null si la trama no es de 960 muestras o el codificador está cerrado. */
    public static native byte[] opusEncode(long handle, short[] pcm);
    public static native void opusEncoderClose(long handle);
    public static native long opusDecoderCreate();
    /** {@code packet} null = trama perdida. null si el decodificador está cerrado. */
    public static native short[] opusDecode(long handle, byte[] packet);
    public static native void opusDecoderClose(long handle);

    /** Grupo actual (null si ninguno). */
    public static native String clientGroupName(long handle);

    /** Nombre de un miembro del grupo de red (puede estar en otro servidor), o null. */
    public static native String clientMemberName(long handle, long uuidMsb, long uuidLsb);

    /**
     * Pide al servidor los grupos que hay. La respuesta tarda un viaje de ida y vuelta
     * y se lee con {@link #clientGroupList}; hay que volver a pedirla para verla al día.
     * Los servidores anteriores a 0.13 no contestan, así que la lista queda vacía.
     */
    public static native void clientRequestGroups(long handle);

    /** {@code ["nombre|miembros|candado|permanente|propio", ...]}, los tres últimos 0 o 1. */
    public static native String[] clientGroupList(long handle);

    /** UUID de los miembros del grupo propio, en pares (mitad alta, mitad baja). */
    public static native long[] clientGroupMembers(long handle);
    /** Bits: 1 = dueño, 2 = permanente, 4 = grupo de red. */
    public static native int clientGroupFlags(long handle);
}
