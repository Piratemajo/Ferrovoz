package dev.ferrovoz.fabric;

import dev.ferrovoz.ReplayVoicePayload;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.VoiceTunnelPayload;
import dev.ferrovoz.client.FerrovozClient;
import dev.ferrovoz.client.SpeakerIcons;
import dev.ferrovoz.client.VoiceHud;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.keybinding.v1.KeyBindingHelper;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayNetworking;
import net.fabricmc.fabric.api.client.rendering.v1.HudRenderCallback;
import net.fabricmc.fabric.api.client.rendering.v1.WorldRenderEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.KeyMapping;
import net.minecraft.resources.ResourceLocation;

import java.nio.file.Path;
import java.util.HashSet;
import java.util.Set;

/** Ferrovoz en el cliente de Fabric 1.21.1: conecta eventos, red, teclas y HUD con {@link FerrovozClient}. */
public final class FerrovozFabricClient implements ClientModInitializer, FerrovozClient.Platform {
    @Override
    public void onInitializeClient() {
        FerrovozClient.init(this);
        for (KeyMapping key : FerrovozClient.keyMappings()) {
            KeyBindingHelper.registerKeyBinding(key);
        }

        // Los receptores se registran aunque falte la librería nativa, para poder
        // explicar al jugador por qué no tiene voz.
        ClientPlayNetworking.registerGlobalReceiver(VersionPayload.TYPE, (payload, context) -> FerrovozClient.onVersion(payload));
        ClientPlayNetworking.registerGlobalReceiver(VoiceSetupPayload.TYPE, (payload, context) -> FerrovozClient.onSetup(payload));
        ClientPlayNetworking.registerGlobalReceiver(VoiceTunnelPayload.TYPE, (payload, context) -> FerrovozClient.onTunnel(payload.data()));
        ClientPlayNetworking.registerGlobalReceiver(ReplayVoicePayload.TYPE, (payload, context) -> FerrovozClient.onReplayVoice(payload));
        ClientPlayConnectionEvents.JOIN.register((handler, sender, mc) -> FerrovozClient.onJoin());
        ClientPlayConnectionEvents.DISCONNECT.register((handler, mc) -> FerrovozClient.onDisconnect());
        ClientTickEvents.END_CLIENT_TICK.register(FerrovozClient::onTick);

        HudRenderCallback.EVENT.register(VoiceHud::render);
        WorldRenderEvents.AFTER_ENTITIES.register(context -> SpeakerIcons.renderWorld(
                context.camera().getPosition(), context.consumers(),
                context.tickCounter().getGameTimeDeltaPartialTick(false)));
    }

    @Override
    public Path configDir() {
        return FabricLoader.getInstance().getConfigDir();
    }

    @Override
    public void sendTunnel(byte[] packet) {
        ClientPlayNetworking.send(new VoiceTunnelPayload(packet));
    }

    @Override
    public Set<String> serverChannels() {
        Set<String> channels = new HashSet<>();
        for (ResourceLocation id : ClientPlayNetworking.getSendable()) channels.add(id.toString());
        return channels;
    }
}
