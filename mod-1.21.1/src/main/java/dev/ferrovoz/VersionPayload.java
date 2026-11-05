package dev.ferrovoz;

import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/**
 * Servidor -> cliente: versión de Ferrovoz y de protocolo del servidor. Canal y
 * formato fijos (ver {@link Protocol#VERSION_CHANNEL}) para que cualquier versión
 * pueda avisar de que la otra no es compatible.
 *
 * @param protocol       {@link Protocol#VERSION} del servidor
 * @param version        versión del mod/plugin
 * @param voiceAvailable false si el relé no funciona (puerto ocupado, sin binario
 *                       para la plataforma...)
 */
public record VersionPayload(int protocol, String version, boolean voiceAvailable) implements CustomPacketPayload {
    public static final ResourceLocation ID = ResourceLocation.fromNamespaceAndPath(Protocol.NAMESPACE, Protocol.VERSION_PATH);
    public static final CustomPacketPayload.Type<VersionPayload> TYPE = new CustomPacketPayload.Type<>(ID);
    public static final StreamCodec<RegistryFriendlyByteBuf, VersionPayload> CODEC = StreamCodec.composite(
            ByteBufCodecs.VAR_INT, VersionPayload::protocol,
            ByteBufCodecs.STRING_UTF8, VersionPayload::version,
            ByteBufCodecs.BOOL, VersionPayload::voiceAvailable,
            VersionPayload::new);

    public static VersionPayload current(boolean voiceAvailable) {
        return new VersionPayload(Protocol.VERSION, Protocol.modVersion(), voiceAvailable);
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
