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
import net.fabricmc.fabric.api.client.keymapping.v1.KeyMappingHelper;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayConnectionEvents;
import net.fabricmc.fabric.api.client.networking.v1.ClientPlayNetworking;
import net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry;
import net.fabricmc.fabric.api.client.rendering.v1.level.LevelRenderEvents;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.KeyMapping;
import net.minecraft.resources.Identifier;

import java.nio.file.Path;
import java.util.HashSet;
import java.util.Set;

/** Ferrovoz en el cliente de Fabric: conecta eventos, red, teclas y HUD con {@link FerrovozClient}. */
public final class FerrovozFabricClient implements ClientModInitializer, FerrovozClient.Platform {
    @Override
    public void onInitializeClient() {
        FerrovozClient.init(this, KeyMapping.Category::register);
        for (KeyMapping key : FerrovozClient.keyMappings()) {
            KeyMappingHelper.registerKeyMapping(key);
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

        HudElementRegistry.addLast(FerrovozClient.HUD_ID, VoiceHud::extract);
        LevelRenderEvents.COLLECT_SUBMITS.register(context ->
                SpeakerIcons.submitWorld(context.levelState().cameraRenderState.pos, context.poseStack(), context.submitNodeCollector()));
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
        for (Identifier id : ClientPlayNetworking.getSendable()) channels.add(id.toString());
        return channels;
    }
}
