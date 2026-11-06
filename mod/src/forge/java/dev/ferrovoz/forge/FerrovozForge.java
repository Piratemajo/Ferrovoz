package dev.ferrovoz.forge;

import dev.ferrovoz.Ferrovoz;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.SetupMessage;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceCommands;
import dev.ferrovoz.VoiceSetupPayload;
import io.netty.buffer.Unpooled;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraftforge.event.RegisterCommandsEvent;
import net.minecraftforge.event.TickEvent;
import net.minecraftforge.event.entity.player.PlayerEvent;
import net.minecraftforge.event.network.CustomPayloadEvent;
import net.minecraftforge.event.server.ServerStartedEvent;
import net.minecraftforge.event.server.ServerStoppingEvent;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.loading.FMLEnvironment;
import net.minecraftforge.fml.loading.FMLPaths;
import net.minecraftforge.network.ChannelBuilder;
import net.minecraftforge.network.EventNetworkChannel;
import net.minecraftforge.network.PacketDistributor;
import net.minecraftforge.server.permission.PermissionAPI;
import net.minecraftforge.server.permission.events.PermissionGatherEvent;
import net.minecraftforge.server.permission.nodes.PermissionNode;
import net.minecraftforge.server.permission.nodes.PermissionTypes;

import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/**
 * Ferrovoz en Forge: conecta eventos, red y permisos de Forge con {@link Ferrovoz}.
 * El cliente está en {@link FerrovozForgeClient}.
 *
 * <p>Red: canales raw de Forge con los mismos nombres y bytes que en el resto de
 * plataformas ({@link SetupMessage}).
 */
@Mod(Ferrovoz.MOD_ID)
public final class FerrovozForge implements Ferrovoz.Platform {
    static EventNetworkChannel setup;
    static EventNetworkChannel tunnel;
    static EventNetworkChannel version;

    private static final Map<String, PermissionNode<Boolean>> NODES = new LinkedHashMap<>();

    static {
        for (String node : new String[] {VoiceCommands.PERM_SPEAK, VoiceCommands.PERM_SHOUT, VoiceCommands.PERM_GROUPS}) {
            NODES.put(node, new PermissionNode<>(Ferrovoz.MOD_ID, node, PermissionTypes.BOOLEAN, (player, uuid, ctx) -> true));
        }
        // Moderación: por defecto solo operadores (nivel 3, como en Fabric).
        NODES.put(VoiceCommands.PERM_ADMIN, new PermissionNode<>(Ferrovoz.MOD_ID, VoiceCommands.PERM_ADMIN, PermissionTypes.BOOLEAN,
                (player, uuid, ctx) -> player != null && Commands.LEVEL_ADMINS.check(player.permissions())));
        // Prioridad en multitudes (anfitrión de un evento): por defecto, operadores.
        NODES.put(VoiceCommands.PERM_PRIORITY, new PermissionNode<>(Ferrovoz.MOD_ID, VoiceCommands.PERM_PRIORITY, PermissionTypes.BOOLEAN,
                (player, uuid, ctx) -> player != null && Commands.LEVEL_ADMINS.check(player.permissions())));
    }

    public FerrovozForge() {
        setup = channel(Protocol.CHANNEL);
        tunnel = channel(Protocol.TUNNEL);
        version = channel(Protocol.VERSION_CHANNEL);
        tunnel.addListener(FerrovozForge::onTunnelMessage);
        boolean client = !FMLEnvironment.dist.isDedicatedServer();
        if (client) {
            setup.addListener(FerrovozForgeClient::onSetupMessage);
            version.addListener(FerrovozForgeClient::onVersionMessage);
        }

        Ferrovoz.init(this);
        ServerStartedEvent.BUS.addListener(e -> Ferrovoz.onServerStarted(e.getServer()));
        ServerStoppingEvent.BUS.addListener(e -> Ferrovoz.onServerStopping());
        PlayerEvent.PlayerLoggedInEvent.BUS.addListener(e -> {
            if (e.getEntity() instanceof ServerPlayer p) Ferrovoz.onJoin(p);
        });
        PlayerEvent.PlayerLoggedOutEvent.BUS.addListener(e -> {
            if (e.getEntity() instanceof ServerPlayer p) Ferrovoz.onLeave(p);
        });
        RegisterCommandsEvent.BUS.addListener(e -> Ferrovoz.registerCommands(e.getDispatcher()));
        TickEvent.ServerTickEvent.Pre.BUS.addListener(e -> Ferrovoz.onStartTick());
        TickEvent.ServerTickEvent.Post.BUS.addListener(e -> Ferrovoz.onTick(e.server()));
        PermissionGatherEvent.Nodes.BUS.addListener(e -> e.addNodes(NODES.values().toArray(new PermissionNode<?>[0])));

        if (client) {
            FerrovozForgeClient.init();
        }
    }

    /** Canal opcional (sin Ferrovoz se entra igual) y sin versión, que ya va en el nombre. */
    private static EventNetworkChannel channel(String name) {
        return ChannelBuilder.named(Identifier.parse(name))
                .networkProtocolVersion(1)
                .optional()
                .acceptedVersions((status, remote) -> true)
                .eventNetworkChannel();
    }

    static byte[] bytes(CustomPayloadEvent e) {
        FriendlyByteBuf buf = e.getPayload();
        if (buf == null) return null;
        byte[] out = new byte[buf.readableBytes()];
        buf.readBytes(out);
        return out;
    }

    static FriendlyByteBuf buffer(byte[] data) {
        return new FriendlyByteBuf(Unpooled.wrappedBuffer(data));
    }

    /** Túnel en los dos sentidos. Se atiende en el hilo de red. */
    private static void onTunnelMessage(CustomPayloadEvent e) {
        byte[] packet = SetupMessage.decodeTunnel(bytes(e));
        if (e.getSource().isServerSide()) {
            ServerPlayer sender = e.getSource().getSender();
            if (sender != null) Ferrovoz.onTunnel(sender, packet);
        } else {
            FerrovozForgeClient.onTunnel(packet);
        }
        e.getSource().setPacketHandled(true);
    }

    // --- Ferrovoz.Platform ---

    @Override
    public String name() {
        return "Forge";
    }

    @Override
    public Path configDir() {
        return FMLPaths.CONFIGDIR.get();
    }

    private static EventNetworkChannel byName(String channel) {
        return switch (channel) {
            case Protocol.CHANNEL -> setup;
            case Protocol.TUNNEL -> tunnel;
            case Protocol.VERSION_CHANNEL -> version;
            default -> null;
        };
    }

    @Override
    public boolean canSend(ServerPlayer player, String channel) {
        EventNetworkChannel c = byName(channel);
        if (c == null) return false;
        net.minecraft.network.Connection connection = connectionOf(player);
        return connection != null && c.isRemotePresent(connection);
    }

    private static volatile java.lang.reflect.Method getConnection;

    /**
     * Conexión del jugador. {@code getConnection()} lo añade Forge con sus parches y no
     * existe en el Minecraft sin parchear contra el que se compila: por reflexión.
     */
    private static net.minecraft.network.Connection connectionOf(ServerPlayer player) {
        try {
            java.lang.reflect.Method m = getConnection;
            if (m == null) {
                m = player.connection.getClass().getMethod("getConnection");
                getConnection = m;
            }
            return (net.minecraft.network.Connection) m.invoke(player.connection);
        } catch (ReflectiveOperationException | RuntimeException e) {
            Ferrovoz.LOG.warn("Ferrovoz: could not read the connection of {}", player.getName().getString(), e);
            return null;
        }
    }

    @Override
    public void sendSetup(ServerPlayer player, VoiceSetupPayload p) {
        setup.send(buffer(SetupMessage.encode(p.host(), p.port(), p.key(), p.transport(),
                new java.util.UUID(p.idMsb(), p.idLsb()))), PacketDistributor.PLAYER.with(player));
    }

    @Override
    public void sendTunnel(ServerPlayer player, byte[] packet) {
        tunnel.send(buffer(SetupMessage.encodeTunnel(packet)), PacketDistributor.PLAYER.with(player));
    }

    @Override
    public void sendVersion(ServerPlayer player, VersionPayload p) {
        version.send(buffer(SetupMessage.encodeVersion(p.protocol(), p.version(), p.voiceAvailable())),
                PacketDistributor.PLAYER.with(player));
    }

    @Override
    public Set<String> clientChannels(ServerPlayer player) {
        return Set.of(); // en Forge no hay versiones anteriores a 0.6
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
        return !VoiceCommands.opByDefault(node) || Commands.LEVEL_ADMINS.check(source.permissions());
    }
}
