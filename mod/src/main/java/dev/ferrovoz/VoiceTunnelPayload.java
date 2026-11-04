package dev.ferrovoz;

import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/**
 * Paquete de voz cifrado por la conexión del juego (ambos sentidos), cuando no hay
 * UDP. El contenido es el mismo que por UDP; Minecraft solo lo transporta.
 */
public record VoiceTunnelPayload(byte[] data) implements CustomPacketPayload {
    public static final Identifier ID = Identifier.fromNamespaceAndPath(Protocol.NAMESPACE, Protocol.TUNNEL_PATH);
    public static final CustomPacketPayload.Type<VoiceTunnelPayload> TYPE = new CustomPacketPayload.Type<>(ID);
    public static final StreamCodec<RegistryFriendlyByteBuf, VoiceTunnelPayload> CODEC = StreamCodec.composite(
            ByteBufCodecs.BYTE_ARRAY, VoiceTunnelPayload::data,
            VoiceTunnelPayload::new);

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
