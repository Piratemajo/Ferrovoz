package dev.ferrovoz.neoforge;

import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.VoiceTunnelPayload;
import dev.ferrovoz.client.FerrovozClient;
import dev.ferrovoz.client.SpeakerIcons;
import dev.ferrovoz.client.VoiceHud;
import dev.ferrovoz.client.VoiceSettingsScreen;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.loading.FMLPaths;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RegisterGuiLayersEvent;
import net.neoforged.neoforge.client.event.RegisterKeyMappingsEvent;
import net.neoforged.neoforge.client.event.SubmitCustomGeometryEvent;
import net.neoforged.neoforge.client.gui.IConfigScreenFactory;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import net.neoforged.neoforge.common.NeoForge;

import java.nio.file.Path;
import java.util.Set;

/** Ferrovoz en el cliente de NeoForge. Esta clase solo se carga en el cliente. */
final class FerrovozNeoForgeClient implements FerrovozClient.Platform {
    private static KeyMapping.Category category;

    private FerrovozNeoForgeClient() {}

    static void init(IEventBus modBus, ModContainer container) {
        // En NeoForge la categoría de teclas se crea y luego se registra en su evento.
        FerrovozClient.init(new FerrovozNeoForgeClient(), id -> category = new KeyMapping.Category(id));

        modBus.addListener(RegisterKeyMappingsEvent.class, e -> {
            if (category != null) e.registerCategory(category);
            for (KeyMapping key : FerrovozClient.keyMappings()) e.register(key);
        });
        modBus.addListener(RegisterGuiLayersEvent.class, e -> e.registerAboveAll(FerrovozClient.HUD_ID, VoiceHud::extract));

        IEventBus game = NeoForge.EVENT_BUS;
        game.addListener(ClientTickEvent.Post.class, e -> FerrovozClient.onTick(Minecraft.getInstance()));
        game.addListener(ClientPlayerNetworkEvent.LoggingIn.class, e -> FerrovozClient.onJoin());
        game.addListener(ClientPlayerNetworkEvent.LoggingOut.class, e -> FerrovozClient.onDisconnect());
        game.addListener(SubmitCustomGeometryEvent.class, e -> SpeakerIcons.submitWorld(
                e.getLevelRenderState().cameraRenderState.pos, e.getPoseStack(), e.getSubmitNodeCollector()));

        // Botón "Configurar" en la lista de mods: abre los ajustes de voz.
        if (!FerrovozClient.keyMappings().isEmpty()) {
            container.registerExtensionPoint(IConfigScreenFactory.class,
                    (IConfigScreenFactory) (mod, parent) -> new VoiceSettingsScreen(parent));
        }
    }

    // Llamados desde los manejadores de red de FerrovozNeoForge (solo en el cliente).
    static void onSetup(VoiceSetupPayload payload) {
        FerrovozClient.onSetup(payload);
    }

    static void onVersion(VersionPayload payload) {
        FerrovozClient.onVersion(payload);
    }

    static void onTunnel(byte[] packet) {
        FerrovozClient.onTunnel(packet);
    }

    @Override
    public Path configDir() {
        return FMLPaths.CONFIGDIR.get();
    }

    @Override
    public void sendTunnel(byte[] packet) {
        ClientPacketDistributor.sendToServer(new VoiceTunnelPayload(packet));
    }

    @Override
    public Set<String> serverChannels() {
        return Set.of();
    }
}
