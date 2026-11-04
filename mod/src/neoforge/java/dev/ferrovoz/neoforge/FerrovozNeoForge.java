package dev.ferrovoz.neoforge;

import dev.ferrovoz.Ferrovoz;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceCommands;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.VoiceTunnelPayload;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.common.extensions.ICommonPacketListener;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.event.entity.player.PlayerEvent;
import net.neoforged.neoforge.event.server.ServerStartedEvent;
import net.neoforged.neoforge.event.server.ServerStoppingEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.registration.HandlerThread;
import net.neoforged.neoforge.network.registration.NetworkRegistry;
import net.neoforged.neoforge.network.registration.PayloadRegistrar;
import net.neoforged.neoforge.server.permission.PermissionAPI;
import net.neoforged.neoforge.server.permission.events.PermissionGatherEvent;
import net.neoforged.neoforge.server.permission.nodes.PermissionNode;
import net.neoforged.neoforge.server.permission.nodes.PermissionTypes;

import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/**
 * Ferrovoz en NeoForge: conecta eventos, red y permisos con {@link Ferrovoz}. El
 * cliente está en {@link FerrovozNeoForgeClient}.
 *
 * <p>Permisos con la API de NeoForge ({@code ferrovoz.speak}...), que LuckPerms
 * gestiona igual que en Fabric y Paper.
 */
@Mod(Ferrovoz.MOD_ID)
public final class FerrovozNeoForge implements Ferrovoz.Platform {
    private static final Map<String, PermissionNode<Boolean>> NODES = new LinkedHashMap<>();

    static {
        for (String node : new String[] {VoiceCommands.PERM_SPEAK, VoiceCommands.PERM_SHOUT, VoiceCommands.PERM_GROUPS}) {
            NODES.put(node, new PermissionNode<>(Ferrovoz.MOD_ID, node, PermissionTypes.BOOLEAN, (player, uuid, ctx) -> true));
        }
        // Moderación: por defecto solo operadores (nivel 3, como en Fabric).
        NODES.put(VoiceCommands.PERM_ADMIN, new PermissionNode<>(Ferrovoz.MOD_ID, VoiceCommands.PERM_ADMIN, PermissionTypes.BOOLEAN,
                (player, uuid, ctx) -> player != null && Commands.LEVEL_ADMINS.check(player.permissions())));
    }

    public FerrovozNeoForge(IEventBus modBus, ModContainer container, Dist dist) {
        Ferrovoz.init(this);
        modBus.addListener(RegisterPayloadHandlersEvent.class, FerrovozNeoForge::registerPayloads);

        IEventBus game = NeoForge.EVENT_BUS;
        game.addListener(ServerStartedEvent.class, e -> Ferrovoz.onServerStarted(e.getServer()));
        game.addListener(ServerStoppingEvent.class, e -> Ferrovoz.onServerStopping());
        game.addListener(PlayerEvent.PlayerLoggedInEvent.class, e -> {
            if (e.getEntity() instanceof ServerPlayer p) Ferrovoz.onJoin(p);
        });
        game.addListener(PlayerEvent.PlayerLoggedOutEvent.class, e -> {
            if (e.getEntity() instanceof ServerPlayer p) Ferrovoz.onLeave(p);
        });
        game.addListener(RegisterCommandsEvent.class, e -> Ferrovoz.registerCommands(e.getDispatcher()));
        game.addListener(ServerTickEvent.Pre.class, e -> Ferrovoz.onStartTick());
        game.addListener(ServerTickEvent.Post.class, e -> Ferrovoz.onTick(e.getServer()));
        game.addListener(PermissionGatherEvent.Nodes.class, e -> e.addNodes(NODES.values().toArray(new PermissionNode<?>[0])));

        if (dist.isClient()) {
            FerrovozNeoForgeClient.init(modBus, container);
        }
    }

    /**
     * Mismos canales que en Fabric y Paper, opcionales (sin Ferrovoz se entra igual).
     * Los handlers de cliente solo se ejecutan en el cliente.
     */
    private static void registerPayloads(RegisterPayloadHandlersEvent event) {
        PayloadRegistrar registrar = event.registrar("1").optional();
        registrar.playToClient(VoiceSetupPayload.TYPE, VoiceSetupPayload.CODEC,
                (payload, ctx) -> FerrovozNeoForgeClient.onSetup(payload));
        registrar.playToClient(VersionPayload.TYPE, VersionPayload.CODEC,
                (payload, ctx) -> FerrovozNeoForgeClient.onVersion(payload));
        // El túnel se atiende en el hilo de red, sin esperar al tick.
        event.registrar("1").optional().executesOn(HandlerThread.NETWORK).playBidirectional(
                VoiceTunnelPayload.TYPE, VoiceTunnelPayload.CODEC,
                (payload, ctx) -> {
                    if (ctx.player() instanceof ServerPlayer p) Ferrovoz.onTunnel(p, payload.data());
                },
                (payload, ctx) -> FerrovozNeoForgeClient.onTunnel(payload.data()));
    }

    // --- Ferrovoz.Platform ---

    @Override
    public String name() {
        return "NeoForge";
    }

    @Override
    public Path configDir() {
        return FMLPaths.CONFIGDIR.get();
    }

    @Override
    public boolean canSend(ServerPlayer player, String channel) {
        // NeoForge añade ICommonPacketListener a la conexión del jugador.
        return NetworkRegistry.hasChannel((ICommonPacketListener) player.connection, Identifier.parse(channel));
    }

    @Override
    public void sendSetup(ServerPlayer player, VoiceSetupPayload payload) {
        PacketDistributor.sendToPlayer(player, payload);
    }

    @Override
    public void sendTunnel(ServerPlayer player, byte[] packet) {
        PacketDistributor.sendToPlayer(player, new VoiceTunnelPayload(packet));
    }

    @Override
    public void sendVersion(ServerPlayer player, VersionPayload payload) {
        PacketDistributor.sendToPlayer(player, payload);
    }

    @Override
    public Set<String> clientChannels(ServerPlayer player) {
        return Set.of(); // en NeoForge no hay versiones anteriores a 0.6
    }

    @Override
    public boolean hasPermission(ServerPlayer player, String node) {
        PermissionNode<Boolean> n = NODES.get(node);
        return n != null && Boolean.TRUE.equals(PermissionAPI.getPermission(player, n));
    }

    @Override
    public boolean hasPermission(CommandSourceStack source, String node) {
        ServerPlayer player = source.getPlayer();
        if (player != null) return hasPermission(player, node);
        // Consola o bloque de comandos: según su nivel de permisos.
        return !VoiceCommands.PERM_ADMIN.equals(node) || Commands.LEVEL_ADMINS.check(source.permissions());
    }
}
