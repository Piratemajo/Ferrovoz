package dev.ferrovoz.bukkit;

import dev.ferrovoz.ApiCore;
import dev.ferrovoz.FerrovozConfig;
import dev.ferrovoz.FerrovozNative;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.SetupMessage;
import dev.ferrovoz.TunnelPump;
import dev.ferrovoz.VoiceCommands;
import dev.ferrovoz.WallScheduler;
import dev.ferrovoz.WallTracer;
import org.bukkit.World;
import org.bukkit.Bukkit;
import org.bukkit.Location;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerChangedWorldEvent;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerRespawnEvent;
import org.bukkit.event.player.PlayerQuitEvent;
import org.bukkit.event.player.PlayerRegisterChannelEvent;
import org.bukkit.command.PluginCommand;
import org.bukkit.plugin.ServicePriority;
import org.bukkit.plugin.java.JavaPlugin;

import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Plugin para Paper/Purpur/Folia/Spigot. Hace lo mismo que el lado servidor del mod
 * ({@code dev.ferrovoz.Ferrovoz}): arranca el relé, registra a los jugadores que
 * tienen el mod y le pasa las posiciones cada tick.
 *
 * <p>Folia: el código de cada jugador corre en el hilo de su región (ver
 * {@link PlayerScheduling}) y el estado compartido es concurrente. Las llamadas
 * nativas son seguras desde cualquier hilo.
 *
 * <p>La API ({@code dev.ferrovoz.api.FerrovozApi}) se registra también en el
 * ServicesManager de Bukkit.
 */
public final class FerrovozPlugin extends JavaPlugin implements Listener {
    private FerrovozConfig config;
    private volatile long server; // handle del VoiceServer (0 = parado)
    private final Set<UUID> registered = ConcurrentHashMap.newKeySet();
    private PlayerScheduling scheduling;
    /** Ticks por jugador, para refrescar permisos cada pocos segundos. */
    private final java.util.Map<UUID, Integer> ticks = new ConcurrentHashMap<>();
    private static final int PERMISSION_REFRESH_TICKS = 100;
    /** Jugadores con voz (túnel, avisos de grupo, oclusión). */
    private final java.util.Map<UUID, Player> online = new ConcurrentHashMap<>();
    private final WallScheduler walls = new WallScheduler();
    private TunnelPump tunnel;
    private ApiCore api;
    private PasswordLogFilter passwordFilter;
    private java.util.concurrent.ScheduledExecutorService events;

    /** Jugadores a los que ya se ha enviado la versión (una vez por conexión). */
    private final Set<UUID> versionSent = ConcurrentHashMap.newKeySet();
    /** Segundos tras entrar para comprobar la versión (los canales del cliente llegan en ese rato). */
    private static final long VERSION_CHECK_DELAY_MS = 3000;

    @Override
    public void onEnable() {
        // Siempre, haya voz o no, para que el cliente sepa por qué no la tiene.
        Bukkit.getMessenger().registerOutgoingPluginChannel(this, Protocol.VERSION_CHANNEL);
        scheduling = PlayerScheduling.create(this);
        events = java.util.concurrent.Executors.newSingleThreadScheduledExecutor(r -> {
            Thread t = new Thread(r, "ferrovoz-events");
            t.setDaemon(true);
            return t;
        });
        Bukkit.getPluginManager().registerEvents(this, this);
        // Que las contraseñas de /voz grupo no queden en latest.log.
        try {
            passwordFilter = PasswordLogFilter.install(getLogger());
        } catch (Throwable e) { // sin log4j: no debería pasar en Paper ni en Spigot
            getLogger().warning("Could not hide group passwords from the console log: " + e);
        }
        // /voz responde aunque el relé no esté en marcha.
        BukkitVoiceCommands handler = new BukkitVoiceCommands(() -> server, () -> config, online);
        PluginCommand voz = getCommand("voz");
        if (voz != null) {
            voz.setExecutor(handler);
            voz.setTabCompleter(handler);
        }

        // Siempre la variante sin audio.
        if (!FerrovozNative.load(true)) {
            getLogger().severe("No Ferrovoz native library for " + FerrovozNative.platform()
                    + ": the server works, but without voice chat (players with Ferrovoz will be told).");
            return;
        }

        api = new ApiCore(new ApiCore.Bridge() {
            @Override
            public String platform() {
                return PlayerScheduling.isFolia() ? "Folia" : scheduling instanceof PaperScheduling ? "Paper" : "Spigot";
            }

            @Override
            public String nameOf(UUID id) {
                Player p = online.get(id);
                if (p == null) p = Bukkit.getPlayer(id);
                return p == null ? null : p.getName();
            }

            @Override
            public dev.ferrovoz.api.VoiceWorld world(Object world) {
                if (!(world instanceof World w)) {
                    throw new IllegalArgumentException("expected an org.bukkit.World: " + world.getClass().getName());
                }
                return new ApiCore.World(w.getName(), dimensionOf(w));
            }
        });
        api.register();
        getServer().getServicesManager().register(dev.ferrovoz.api.FerrovozApi.class, api, this, ServicePriority.Normal);

        config = FerrovozConfig.load(getDataFolder().toPath().resolve("ferrovoz.properties"));
        server = FerrovozNative.serverStart(config.bindHost, config.port, config.maxDistance);
        if (server == 0) {
            getLogger().severe("Could not open UDP port " + config.port
                    + ". Is it in use? The server works, but without voice chat.");
            return;
        }
        FerrovozNative.serverSetRanges(server, config.maxDistance, config.whisperDistance, config.shoutDistance);
        FerrovozNative.serverSetRoleRules(server, config.spectators, config.dead);
        FerrovozNative.serverSetMaxVoices(server, config.maxVoices);
        FerrovozNative.serverSetCaveEcho(server, config.caveEcho);
        FerrovozNative.serverSetUnderwaterEffect(server, config.underwaterEffect);
        // Silencios y grupos permanentes.
        FerrovozNative.serverSetDataFile(server, getDataFolder().toPath().resolve("data.txt").toAbsolutePath().toString());

        Bukkit.getMessenger().registerOutgoingPluginChannel(this, Protocol.CHANNEL);
        Bukkit.getMessenger().registerOutgoingPluginChannel(this, Protocol.TUNNEL);
        // Túnel: puede llegar desde el hilo de red; las llamadas nativas son thread-safe.
        Bukkit.getMessenger().registerIncomingPluginChannel(this, Protocol.TUNNEL, (channel, player, message) -> {
            UUID id = player.getUniqueId();
            byte[] packet = SetupMessage.decodeTunnel(message);
            long s = server;
            if (packet != null && s != 0 && registered.contains(id)) {
                FerrovozNative.serverHandleTunnel(s, id.getMostSignificantBits(), id.getLeastSignificantBits(), packet);
            }
        });

        // Envío del túnel desde su propio hilo cada 5 ms.
        tunnel = new TunnelPump(() -> server, (id, packet) -> {
            Player p = online.get(id);
            if (p != null) p.sendPluginMessage(this, Protocol.TUNNEL, SetupMessage.encodeTunnel(packet));
        });
        api.started(server, config);
        // Avisos de grupo y eventos de la API, cada tick en el hilo principal (en Folia, el global).
        VoiceCommands.Host host = handler.host();
        scheduling.repeatGlobally(() -> {
            try {
                VoiceCommands.dispatchEvents(host);
            } catch (RuntimeException e) {
                getLogger().warning("Voice event dispatch failed: " + e);
            }
        }, 1L);
        // Oclusión con el mundo del servidor. En Folia no: leer bloques de otra región no
        // es seguro, así que allí solo la calcula el cliente.
        if (!PlayerScheduling.isFolia()) {
            Bukkit.getScheduler().runTaskTimer(this, this::updateWalls, 1L, 1L);
        }

        // Por si se habilita con jugadores conectados.
        for (Player p : Bukkit.getOnlinePlayers()) {
            scheduling.runFor(p, () -> trySetup(p));
            scheduleVersionCheck(p);
        }
        String mode = PlayerScheduling.isFolia() ? "Folia" : scheduling instanceof PaperScheduling ? "Paper" : "Spigot";
        getLogger().info("Voice server on UDP port " + config.port + " (" + mode + " mode)");
    }

    @Override
    public void onDisable() {
        // Las tareas las cancela el servidor al deshabilitar.
        if (passwordFilter != null) passwordFilter.deactivate();
        if (tunnel != null) tunnel.close();
        if (events != null) events.shutdownNow();
        if (api != null) {
            api.stopping();
            api.unregister();
        }
        long s = server;
        server = 0;
        if (s != 0) {
            FerrovozNative.serverStop(s);
        }
        registered.clear();
        online.clear();
        ticks.clear();
        versionSent.clear();
    }

    // --- Versión ---

    private void scheduleVersionCheck(Player p) {
        if (events == null || events.isShutdown()) return;
        events.schedule(() -> {
            if (p.isOnline()) scheduling.runFor(p, () -> announceVersion(p));
        }, VERSION_CHECK_DELAY_MS, java.util.concurrent.TimeUnit.MILLISECONDS);
    }

    /**
     * Envía la versión al cliente (0.6+). Si es un Ferrovoz anterior con otro protocolo,
     * se le explica por chat en su idioma.
     */
    private void announceVersion(Player p) {
        if (!p.isOnline()) return;
        Set<String> channels = p.getListeningPluginChannels();
        if (channels.contains(Protocol.VERSION_CHANNEL)) {
            if (versionSent.add(p.getUniqueId())) {
                p.sendPluginMessage(this, Protocol.VERSION_CHANNEL,
                        SetupMessage.encodeVersion(Protocol.VERSION, Protocol.modVersion(), server != 0));
            }
            return;
        }
        Protocol.ClientKind kind = Protocol.classify(channels);
        String notice = Protocol.legacyNotice(kind, BukkitVoiceCommands.localeOf(p));
        if (notice != null && versionSent.add(p.getUniqueId())) {
            p.sendMessage(notice);
            getLogger().info(p.getName() + " has a Ferrovoz version with a different protocol ("
                    + Protocol.protocolOf(channels) + ", the server uses " + Protocol.VERSION + "); they have been told");
        }
    }

    /** Paredes calculadas en el servidor, con un coste fijo por tick (solo chunks cargados). */
    private void updateWalls() {
        long s = server;
        if (s == 0) return;
        walls.tick(s, this::eye);
    }

    /** Ojos de un jugador con voz y cómo mirar los bloques de su mundo (solo chunks cargados). */
    private WallScheduler.Eye eye(UUID id) {
        Player p = online.get(id);
        if (p == null) return null;
        Location eye = p.getEyeLocation();
        World world = eye.getWorld();
        if (world == null) return null;
        return new WallScheduler.Eye(world, eye.getX(), eye.getY(), eye.getZ(), new WallTracer.BlockProbe() {
            @Override
            public boolean isChunkLoaded(int cx, int cz) {
                return world.isChunkLoaded(cx, cz);
            }

            @Override
            public boolean isSolid(int x, int y, int z) {
                return world.getBlockAt(x, y, z).getType().isOccluding();
            }
        });
    }

    /**
     * El cliente anuncia sus canales con un paquete "register" que puede llegar antes
     * o después de terminar de entrar, así que se comprueba en los dos momentos.
     */
    @EventHandler
    public void onJoin(PlayerJoinEvent e) {
        trySetup(e.getPlayer());
        scheduleVersionCheck(e.getPlayer());
    }

    @EventHandler
    public void onRegisterChannel(PlayerRegisterChannelEvent e) {
        // Puede llegar desde el hilo de red: pasar al hilo del jugador.
        Player p = e.getPlayer();
        if (!p.isOnline()) return;
        if (Protocol.CHANNEL.equals(e.getChannel())) {
            scheduling.runFor(p, () -> trySetup(p));
        } else if (Protocol.VERSION_CHANNEL.equals(e.getChannel())) {
            scheduling.runFor(p, () -> announceVersion(p));
        }
    }

    /**
     * Folia: si la tarea del jugador se retirase al morir o cambiar de mundo, se vuelve
     * a enganchar (trackEveryTick sustituye a la anterior).
     */
    @EventHandler
    public void onRespawn(PlayerRespawnEvent e) {
        retrack(e.getPlayer());
    }

    @EventHandler
    public void onChangedWorld(PlayerChangedWorldEvent e) {
        retrack(e.getPlayer());
    }

    private void retrack(Player p) {
        if (registered.contains(p.getUniqueId())) {
            scheduling.runFor(p, () -> {
                if (registered.contains(p.getUniqueId()) && p.isOnline()) {
                    scheduling.trackEveryTick(p, () -> updatePosition(p));
                }
            });
        }
    }

    @EventHandler
    public void onQuit(PlayerQuitEvent e) {
        UUID id = e.getPlayer().getUniqueId();
        versionSent.remove(id);
        scheduling.untrack(id);
        ticks.remove(id);
        online.remove(id);
        if (tunnel != null) tunnel.remove(id);
        if (registered.remove(id) && server != 0) {
            FerrovozNative.serverRemovePlayer(server, id.getMostSignificantBits(), id.getLeastSignificantBits());
        }
    }

    /** En el hilo del jugador. */
    private void trySetup(Player player) {
        if (server == 0 || !player.isOnline()) return;
        if (!player.getListeningPluginChannels().contains(Protocol.CHANNEL)) {
            return; // sin el mod o con otra versión; puede registrar el canal más tarde
        }
        UUID id = player.getUniqueId();
        if (!registered.add(id)) return; // ya estaba (add es atómico)
        byte[] key = FerrovozNative.serverAddPlayer(server, id.getMostSignificantBits(), id.getLeastSignificantBits());
        if (key == null) {
            registered.remove(id);
            return;
        }
        online.put(id, player);
        if (player.getListeningPluginChannels().contains(Protocol.TUNNEL) && tunnel != null) {
            tunnel.add(id);
        }
        syncPermissions(player);
        player.sendPluginMessage(this, Protocol.CHANNEL,
                SetupMessage.encode(config.publicHost, config.port, key, config.transport, id));
        scheduling.trackEveryTick(player, () -> updatePosition(player));
    }

    /** Permisos de Bukkit, más lo que haya forzado un plugin por la API. */
    private void syncPermissions(Player p) {
        if (server == 0) return;
        api.syncPlayer(p.getUniqueId(), p.hasPermission(BukkitVoiceCommands.node(VoiceCommands.PERM_SPEAK)),
                p.hasPermission(BukkitVoiceCommands.node(VoiceCommands.PERM_SHOUT)),
                p.hasPermission(BukkitVoiceCommands.node(VoiceCommands.PERM_PRIORITY)));
    }

    /**
     * Cabeza bajo el agua, mirando el bloque de los ojos. Funciona en Spigot, Paper y
     * Folia (en Folia estamos en el hilo de su región).
     */
    private static boolean eyeInWater(Location eye) {
        if (eye.getWorld() == null) return false;
        org.bukkit.block.Block b = eye.getBlock();
        org.bukkit.Material t = b.getType();
        if (t == org.bukkit.Material.WATER || t == org.bukkit.Material.BUBBLE_COLUMN
                || t == org.bukkit.Material.KELP || t == org.bukkit.Material.KELP_PLANT
                || t == org.bukkit.Material.SEAGRASS || t == org.bukkit.Material.TALL_SEAGRASS) {
            return true;
        }
        // Bloques waterlogged también cuentan.
        return b.getBlockData() instanceof org.bukkit.block.data.Waterlogged w && w.isWaterlogged();
    }

    /** Cualquier entero que distinga mundos; el nombre es estable mientras corre el servidor. */
    private static int dimensionOf(World world) {
        return world.getName().hashCode();
    }

    /** Vivo, espectador o muerto (para {@code spectators} y {@code dead}). */
    private static int roleOf(Player p) {
        if (p.getGameMode() == org.bukkit.GameMode.SPECTATOR) return FerrovozNative.ROLE_SPECTATOR;
        if (p.isDead()) return FerrovozNative.ROLE_DEAD;
        return FerrovozNative.ROLE_ALIVE;
    }

    /** Cada tick, en el hilo del jugador. */
    private void updatePosition(Player p) {
        long s = server;
        if (s == 0) return;
        UUID id = p.getUniqueId();
        long msb = id.getMostSignificantBits(), lsb = id.getLeastSignificantBits();
        Location eye = p.getEyeLocation();
        int dimension = eye.getWorld() == null ? 0 : dimensionOf(eye.getWorld());
        FerrovozNative.serverUpdatePlayer(s, msb, lsb, eye.getX(), eye.getY(), eye.getZ(), dimension, eyeInWater(eye), roleOf(p));

        // Permisos refrescados cada 5 s (LuckPerms puede cambiarlos en caliente).
        if (ticks.merge(id, 1, Integer::sum) % PERMISSION_REFRESH_TICKS == 0) {
            syncPermissions(p);
        }

    }
}
