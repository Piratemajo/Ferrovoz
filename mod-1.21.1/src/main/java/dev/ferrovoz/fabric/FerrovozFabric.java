package dev.ferrovoz.fabric;

import dev.ferrovoz.Ferrovoz;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.ReplayVoicePayload;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceCommands;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.VoiceTunnelPayload;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.command.v2.CommandRegistrationCallback;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerLifecycleEvents;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.fabricmc.fabric.api.networking.v1.PayloadTypeRegistry;
import net.fabricmc.fabric.api.networking.v1.ServerPlayConnectionEvents;
import net.fabricmc.fabric.api.networking.v1.ServerPlayNetworking;
import net.fabricmc.loader.api.FabricLoader;
import me.lucko.fabric.api.permissions.v0.Permissions;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerPlayer;

import java.nio.file.Path;
import java.util.HashSet;
import java.util.Set;

/**
 * Ferrovoz en Fabric (lado común: servidor dedicado o integrado). Solo conecta
 * los eventos y la red de Fabric con {@link Ferrovoz}.
 *
 * <p>Permisos: en 1.21.1, {@code fabric-permissions-api} (la de LuckPerms, va dentro
 * del jar) con {@code ferrovoz.speak}..., igual que en Paper. Sin LuckPerms, los
 * valores por defecto.
 */
public final class FerrovozFabric implements ModInitializer, Ferrovoz.Platform {
    @Override
    public void onInitialize() {
        PayloadTypeRegistry.playS2C().register(VoiceSetupPayload.TYPE, VoiceSetupPayload.CODEC);
        PayloadTypeRegistry.playS2C().register(VoiceTunnelPayload.TYPE, VoiceTunnelPayload.CODEC);
        PayloadTypeRegistry.playC2S().register(VoiceTunnelPayload.TYPE, VoiceTunnelPayload.CODEC);
        PayloadTypeRegistry.playS2C().register(VersionPayload.TYPE, VersionPayload.CODEC);
        // Voz dentro de las repeticiones (Replay Mod, Flashback): ningún servidor la manda.
        PayloadTypeRegistry.playS2C().register(ReplayVoicePayload.TYPE, ReplayVoicePayload.CODEC);

        Ferrovoz.init(this);

        ServerLifecycleEvents.SERVER_STARTED.register(Ferrovoz::onServerStarted);
        ServerLifecycleEvents.SERVER_STOPPING.register(s -> Ferrovoz.onServerStopping());
        ServerPlayConnectionEvents.JOIN.register((handler, sender, srv) -> Ferrovoz.onJoin(handler.player));
        ServerPlayConnectionEvents.DISCONNECT.register((handler, srv) -> Ferrovoz.onLeave(handler.player));
        // Túnel: voz que llega por la conexión del juego.
        ServerPlayNetworking.registerGlobalReceiver(VoiceTunnelPayload.TYPE,
                (payload, context) -> Ferrovoz.onTunnel(context.player(), payload.data()));
        CommandRegistrationCallback.EVENT.register((dispatcher, registryAccess, environment) -> Ferrovoz.registerCommands(dispatcher));
        ServerTickEvents.START_SERVER_TICK.register(srv -> Ferrovoz.onStartTick());
        ServerTickEvents.END_SERVER_TICK.register(Ferrovoz::onTick);
    }

    @Override
    public String name() {
        return "Fabric";
    }

    @Override
    public Path configDir() {
        return FabricLoader.getInstance().getConfigDir();
    }

    @Override
    public boolean canSend(ServerPlayer player, String channel) {
        return ServerPlayNetworking.canSend(player, ResourceLocation.parse(channel));
    }

    @Override
    public void sendSetup(ServerPlayer player, VoiceSetupPayload payload) {
        ServerPlayNetworking.send(player, payload);
    }

    @Override
    public void sendTunnel(ServerPlayer player, byte[] packet) {
        ServerPlayNetworking.send(player, new VoiceTunnelPayload(packet));
    }

    @Override
    public void sendVersion(ServerPlayer player, VersionPayload payload) {
        ServerPlayNetworking.send(player, payload);
    }

    @Override
    public Set<String> clientChannels(ServerPlayer player) {
        Set<String> channels = new HashSet<>();
        for (ResourceLocation id : ServerPlayNetworking.getSendable(player)) channels.add(id.toString());
        return channels;
    }

    /** Nivel de operador para la moderación (como en 26.3, NeoForge y Forge). */
    private static final int ADMIN_LEVEL = 3;

    private static String permission(String node) {
        return Ferrovoz.MOD_ID + "." + node;
    }

    /** Valor por defecto: la moderación solo para operadores; lo demás, para todos. */
    @Override
    public boolean hasPermission(ServerPlayer player, String node) {
        return VoiceCommands.opByDefault(node)
                ? Permissions.check(player, permission(node), ADMIN_LEVEL)
                : Permissions.check(player, permission(node), true);
    }

    @Override
    public boolean hasPermission(CommandSourceStack source, String node) {
        return VoiceCommands.opByDefault(node)
                ? Permissions.check(source, permission(node), ADMIN_LEVEL)
                : Permissions.check(source, permission(node), true);
    }
}
