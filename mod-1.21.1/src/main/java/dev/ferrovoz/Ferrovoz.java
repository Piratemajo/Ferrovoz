package dev.ferrovoz;

import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import org.slf4j.Logger;

import java.nio.file.Path;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Lado servidor (dedicado o integrado), común a Fabric, NeoForge y Forge. Arranca el
 * relé, le pasa entradas, salidas, posiciones y permisos, calcula la oclusión con el
 * mundo del servidor, avisa de los cambios en los grupos y registra {@code /voz}.
 *
 * <p>Cada loader implementa {@link Platform} y llama a los {@code on*} desde sus
 * eventos ({@code dev.ferrovoz.fabric}, {@code .neoforge}, {@code .forge}).
 */
public final class Ferrovoz {
    public static final String MOD_ID = "ferrovoz";
    public static final Logger LOG = FerrovozLog.LOG;

    /** Lo que depende del loader. */
    public interface Platform {
        /** "Fabric", "NeoForge", "Forge" (para el log). */
        String name();

        Path configDir();

        /** ¿El cliente escucha en ese canal? */
        boolean canSend(ServerPlayer player, String channel);

        void sendSetup(ServerPlayer player, VoiceSetupPayload payload);

        void sendTunnel(ServerPlayer player, byte[] packet);

        void sendVersion(ServerPlayer player, VersionPayload payload);

        /** Canales que anuncia el cliente, para detectar versiones antiguas. Vacío si el loader no lo sabe. */
        Set<String> clientChannels(ServerPlayer player);

        /** {@code node}: "speak", "shout", "groups", "admin" (ver {@link VoiceCommands}). */
        boolean hasPermission(ServerPlayer player, String node);

        boolean hasPermission(CommandSourceStack source, String node);
    }

    /** Ticks entre consultas de permisos (pueden cambiar en caliente). */
    private static final int PERMISSION_REFRESH_TICKS = 100;
    /** Paredes calculadas en el servidor, con un coste fijo por tick. */
    private static WallScheduler walls = new WallScheduler();

    private static Platform platform;
    private static boolean nativeLoaded;
    private static FerrovozConfig config;
    private static Path dataFile;
    private static volatile long server; // handle del VoiceServer (0 = parado)
    /** Jugadores con voz (túnel y oclusión). */
    private static final Map<UUID, ServerPlayer> registered = new ConcurrentHashMap<>();
    private static TunnelPump tunnel;
    private static VoiceCommands.Host host;
    private static int ticks;

    private Ferrovoz() {}

    /**
     * Al cargar el mod. Sin librería nativa para esta plataforma el juego sigue, sin
     * voz, y se avisa a quien tenga Ferrovoz.
     */
    public static void init(Platform p) {
        platform = p;
        nativeLoaded = FerrovozNative.load();
        if (!nativeLoaded) return;
        Path configDir = p.configDir();
        config = FerrovozConfig.load(configDir.resolve("ferrovoz.properties"));
        dataFile = configDir.resolve("ferrovoz-data.txt");
        if (Bench.enabled()) {
            LOG.warn("Ferrovoz in BENCHMARK mode ({} virtual players): the server will shut itself down when done", Bench.PLAYERS);
        }
    }

    static Platform platform() {
        return platform;
    }

    static FerrovozConfig config() {
        return config;
    }

    static long voiceServer() {
        return server;
    }

    public static void onServerStarted(MinecraftServer srv) {
        if (!nativeLoaded) return;
        server = FerrovozNative.serverStart(config.bindHost, config.port, config.maxDistance);
        if (server == 0) {
            LOG.error("Ferrovoz: could not open UDP port {}. Is it in use?", config.port);
            return;
        }
        FerrovozNative.serverSetRanges(server, config.maxDistance, config.whisperDistance, config.shoutDistance);
        FerrovozNative.serverSetRoleRules(server, config.spectators, config.dead);
        FerrovozNative.serverSetMaxVoices(server, config.maxVoices);
        walls = new WallScheduler();
        // Silencios y grupos permanentes.
        FerrovozNative.serverSetDataFile(server, dataFile.toAbsolutePath().toString());
        host = VanillaVoiceCommands.host(srv, () -> server, () -> config);
        // Envío del túnel desde su propio hilo cada 5 ms.
        tunnel = new TunnelPump(() -> server, (id, packet) -> {
            ServerPlayer p = registered.get(id);
            if (p != null) platform.sendTunnel(p, packet);
        });
        LOG.info("Ferrovoz: voice chat running ({})", platform.name());
        Bench.start(srv); // solo con -Dferrovoz.bench=N
    }

    public static void onServerStopping() {
        if (tunnel != null) {
            tunnel.close();
            tunnel = null;
        }
        long s = server;
        server = 0;
        if (s != 0) {
            FerrovozNative.serverStop(s);
        }
        registered.clear();
    }

    /**
     * Envía la versión al cliente (0.6+). Si es un Ferrovoz anterior con otro protocolo,
     * se le explica por chat.
     */
    private static void announceVersion(ServerPlayer player) {
        boolean voice = server != 0;
        if (platform.canSend(player, Protocol.VERSION_CHANNEL)) {
            platform.sendVersion(player, VersionPayload.current(voice));
            return;
        }
        Set<String> channels = platform.clientChannels(player);
        Protocol.ClientKind kind = Protocol.classify(channels);
        String notice = Protocol.legacyNotice(kind, VanillaVoiceCommands.localeOf(player));
        if (notice != null) {
            player.sendSystemMessage(Component.literal(notice));
            LOG.info("{} has a Ferrovoz version with a different protocol ({}, the server uses {}); they have been told",
                    player.getName().getString(), Protocol.protocolOf(channels), Protocol.VERSION);
        }
    }

    public static void onJoin(ServerPlayer player) {
        if (platform == null) return;
        announceVersion(player);
        if (server == 0) return;
        if (!platform.canSend(player, Protocol.CHANNEL)) {
            LOG.info("{} joined without voice chat (no Ferrovoz, or not compatible with protocol {})",
                    player.getName().getString(), Protocol.VERSION);
            return;
        }
        UUID id = player.getUUID();
        byte[] key = FerrovozNative.serverAddPlayer(server, id.getMostSignificantBits(), id.getLeastSignificantBits());
        if (key == null) return;
        registered.put(id, player);
        syncPermissions(player);
        if (platform.canSend(player, Protocol.TUNNEL) && tunnel != null) {
            tunnel.add(id);
        }
        platform.sendSetup(player, new VoiceSetupPayload(config.publicHost, config.port, key, config.transport,
                id.getMostSignificantBits(), id.getLeastSignificantBits()));
    }

    public static void onLeave(ServerPlayer player) {
        UUID id = player.getUUID();
        if (tunnel != null) tunnel.remove(id);
        if (registered.remove(id) != null && server != 0) {
            FerrovozNative.serverRemovePlayer(server, id.getMostSignificantBits(), id.getLeastSignificantBits());
        }
    }

    /** Voz por el túnel. Puede llamarse desde el hilo de red. */
    public static void onTunnel(ServerPlayer player, byte[] packet) {
        UUID id = player.getUUID();
        long s = server;
        if (s != 0 && packet != null && registered.containsKey(id)) {
            FerrovozNative.serverHandleTunnel(s, id.getMostSignificantBits(), id.getLeastSignificantBits(), packet);
        }
    }

    public static void registerCommands(CommandDispatcher<CommandSourceStack> dispatcher) {
        VanillaVoiceCommands.register(dispatcher, () -> server, () -> config);
    }

    private static void syncPermissions(ServerPlayer p) {
        UUID id = p.getUUID();
        FerrovozNative.serverSetPermissions(server, id.getMostSignificantBits(), id.getLeastSignificantBits(),
                platform.hasPermission(p, VoiceCommands.PERM_SPEAK),
                platform.hasPermission(p, VoiceCommands.PERM_SHOUT));
        FerrovozNative.serverSetPriority(server, id.getMostSignificantBits(), id.getLeastSignificantBits(),
                platform.hasPermission(p, VoiceCommands.PERM_PRIORITY));
    }

    /** Al empezar cada tick (solo para el benchmark). */
    public static void onStartTick() {
        Bench.onStartTick();
    }

    /** Al terminar cada tick. */
    public static void onTick(MinecraftServer srv) {
        long s = server;
        if (s == 0) return;
        long started = System.nanoTime();
        ticks++;
        boolean refreshPermissions = ticks % PERMISSION_REFRESH_TICKS == 0;
        for (ServerPlayer p : srv.getPlayerList().getPlayers()) {
            UUID id = p.getUUID();
            // Al reaparecer el ServerPlayer es otro objeto: se guarda el actual para
            // que locate() no use la posición donde murió.
            if (registered.replace(id, p) == null) continue;
            // El hash del ResourceKey identifica la dimensión sin depender de nombres de la API.
            int dimension = p.level().dimension().toString().hashCode();
            FerrovozNative.serverUpdatePlayer(s, id.getMostSignificantBits(), id.getLeastSignificantBits(),
                    p.getX(), p.getEyeY(), p.getZ(), dimension, p.isUnderWater(), roleOf(p));
            if (refreshPermissions) {
                syncPermissions(p);
            }
        }

        if (host != null) {
            VoiceCommands.dispatchEvents(host); // avisos de grupo por chat
        }
        Bench.tickPlayers(s, refreshPermissions); // solo en modo benchmark
        walls.tick(s, Ferrovoz::eye);
        Bench.ferrovozTime(System.nanoTime() - started);
        Bench.onEndTick(srv, s);
    }

    /** Vivo, espectador o muerto (para {@code spectators} y {@code dead}). */
    private static int roleOf(ServerPlayer p) {
        if (p.isSpectator()) return FerrovozNative.ROLE_SPECTATOR;
        if (p.isDeadOrDying()) return FerrovozNative.ROLE_DEAD;
        return FerrovozNative.ROLE_ALIVE;
    }

    /** Posición de los ojos de un jugador con voz (real o virtual del benchmark). */
    record Located(Level level, double x, double y, double z) {}

    private static Located locate(UUID id) {
        ServerPlayer p = registered.get(id);
        if (p != null) return new Located(p.level(), p.getX(), p.getEyeY(), p.getZ());
        return Bench.locate(id);
    }

    /** Ojos de un jugador con voz y cómo mirar los bloques de su mundo (solo chunks cargados). */
    private static WallScheduler.Eye eye(UUID id) {
        Located p = locate(id);
        if (p == null) return null;
        Level level = p.level();
        return new WallScheduler.Eye(level, p.x(), p.y(), p.z(), new WallTracer.BlockProbe() {
            @Override
            public boolean isChunkLoaded(int cx, int cz) {
                return level.hasChunkAt(new BlockPos(cx << 4, 64, cz << 4));
            }

            @Override
            public boolean isSolid(int x, int y, int z) {
                BlockPos pos = new BlockPos(x, y, z);
                return level.getBlockState(pos).isSolidRender(level, pos);
            }
        });
    }
}
