package dev.ferrovoz.forge;

import dev.ferrovoz.SetupMessage;
import dev.ferrovoz.VersionPayload;
import dev.ferrovoz.VoiceSetupPayload;
import dev.ferrovoz.client.FerrovozClient;
import dev.ferrovoz.client.SpeakerIcons;
import dev.ferrovoz.client.VoiceHud;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraftforge.client.event.AddGuiOverlayLayersEvent;
import net.minecraftforge.client.event.ClientPlayerNetworkEvent;
import net.minecraftforge.client.event.RegisterKeyMappingsEvent;
import net.minecraftforge.client.event.RenderAvatarEvent;
import net.minecraftforge.event.TickEvent;
import net.minecraftforge.event.network.CustomPayloadEvent;
import net.minecraftforge.fml.loading.FMLPaths;
import net.minecraftforge.network.PacketDistributor;

import java.nio.file.Path;
import java.util.Set;

/** Ferrovoz en el cliente de Forge. Esta clase solo se carga en el cliente. */
final class FerrovozForgeClient implements FerrovozClient.Platform {
    private FerrovozForgeClient() {}

    static void init() {
        FerrovozClient.init(new FerrovozForgeClient(), KeyMapping.Category::register);
        RegisterKeyMappingsEvent.BUS.addListener(e -> {
            for (KeyMapping key : FerrovozClient.keyMappings()) e.register(key);
        });
        AddGuiOverlayLayersEvent.BUS.addListener(e -> e.getLayeredDraw().add(FerrovozClient.HUD_ID, VoiceHud::extract));
        TickEvent.ClientTickEvent.Post.BUS.addListener(e -> FerrovozClient.onTick(Minecraft.getInstance()));
        ClientPlayerNetworkEvent.LoggingIn.BUS.addListener(e -> FerrovozClient.onJoin());
        ClientPlayerNetworkEvent.LoggingOut.BUS.addListener(e -> FerrovozClient.onDisconnect());
        // Forge no tiene un evento general para dibujar en el mundo: ponemos el icono
        // justo después de dibujar a cada jugador (la pose está en sus pies).
        RenderAvatarEvent.Post.BUS.addListener(e -> SpeakerIcons.submitForAvatar(
                e.getState().x, e.getState().y, e.getState().z, e.getState().boundingBoxHeight,
                e.getCameraState().pos, e.getPoseStack(), e.getNodeCollector()));
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
