package dev.ferrovoz.forge;

import dev.ferrovoz.SetupMessage;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.client.FerrovozClient;
import dev.ferrovoz.client.SpeakerIcons;
import dev.ferrovoz.client.VoiceHud;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraftforge.client.event.ClientPlayerNetworkEvent;
import net.minecraftforge.client.event.CustomizeGuiOverlayEvent;
import net.minecraftforge.client.event.RegisterKeyMappingsEvent;
import net.minecraftforge.client.event.RenderPlayerEvent;
import net.minecraftforge.common.MinecraftForge;
import net.minecraftforge.event.TickEvent;
import net.minecraftforge.event.network.CustomPayloadEvent;
import net.minecraftforge.eventbus.api.IEventBus;
import net.minecraftforge.fml.loading.FMLPaths;
import net.minecraftforge.network.PacketDistributor;

import java.nio.file.Path;
import java.util.Set;

/** Ferrovoz en el cliente de Forge 52 (Minecraft 1.21.1). Esta clase solo se carga en el cliente. */
final class FerrovozForgeClient implements FerrovozClient.Platform {
    private FerrovozForgeClient() {}

    static void init(IEventBus modBus) {
        FerrovozClient.init(new FerrovozForgeClient());
        // Las teclas se registran en el bus del mod.
        modBus.addListener((RegisterKeyMappingsEvent e) -> {
            for (KeyMapping key : FerrovozClient.keyMappings()) e.register(key);
        });

        IEventBus game = MinecraftForge.EVENT_BUS;
        // Forge 52 no tiene un evento general para el HUD: se usa el del chat, que salta
        // en cada fotograma mientras se dibuja el HUD (y no con F1).
        game.addListener((CustomizeGuiOverlayEvent.Chat e) -> VoiceHud.render(e.getGuiGraphics()));
        game.addListener((TickEvent.ClientTickEvent e) -> {
            if (e.phase == TickEvent.Phase.END) FerrovozClient.onTick(Minecraft.getInstance());
        });
        game.addListener((ClientPlayerNetworkEvent.LoggingIn e) -> FerrovozClient.onJoin());
        game.addListener((ClientPlayerNetworkEvent.LoggingOut e) -> FerrovozClient.onDisconnect());
        // Icono justo después de dibujar a cada jugador (la pose está en sus pies).
        game.addListener((RenderPlayerEvent.Post e) -> SpeakerIcons.renderForPlayer(
                e.getEntity(), e.getPoseStack(), e.getMultiBufferSource(), e.getPartialTick()));
    }

    // Mensajes del servidor (llegan en el hilo de red: los pasamos al hilo del juego).
    static void onSetupMessage(CustomPayloadEvent e) {
        if (e.getSource().isServerSide()) return;
        SetupMessage.Setup s = SetupMessage.decodeSetup(FerrovozForge.bytes(e));
        if (s != null) {
            VoiceSetupPayload payload = new VoiceSetupPayload(s.host(), s.port(), s.key(), s.transport(), s.idMsb(), s.idLsb());
            e.getSource().enqueueWork(() -> FerrovozClient.onSetup(payload));
        }
        e.getSource().setPacketHandled(true);
    }

    static void onVersionMessage(CustomPayloadEvent e) {
        if (e.getSource().isServerSide()) return;
        SetupMessage.Version v = SetupMessage.decodeVersion(FerrovozForge.bytes(e));
        if (v != null) {
            VersionPayload payload = new VersionPayload(v.protocol(), v.version(), v.voiceAvailable());
            e.getSource().enqueueWork(() -> FerrovozClient.onVersion(payload));
        }
        e.getSource().setPacketHandled(true);
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
        FerrovozForge.tunnel.send(FerrovozForge.buffer(SetupMessage.encodeTunnel(packet)), PacketDistributor.SERVER.noArg());
    }

    @Override
    public Set<String> serverChannels() {
        return Set.of();
    }
}
