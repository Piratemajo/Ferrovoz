package dev.ferrovoz;

import io.netty.buffer.ByteBuf;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/**
 * Servidor -> cliente por la conexión del juego: dónde está el relé y con qué clave.
 *
 * @param host        host del relé; vacío = el del juego
 * @param port        puerto UDP
 * @param key         clave ChaCha20-Poly1305 de 32 bytes, por jugador y sesión
 * @param transport   0 = automático, 1 = solo UDP, 2 = solo túnel
 * @param idMsb       mitad alta de la UUID registrada en el relé
 * @param idLsb       mitad baja
 *
 * La UUID la decide el servidor: en modo offline o detrás de un proxy no coincide con
 * la de la cuenta, así que el cliente debe usar esta.
 */
public record VoiceSetupPayload(String host, int port, byte[] key, int transport, long idMsb, long idLsb)
        implements CustomPacketPayload {
    /**
     * Versión del formato; va en el nombre del canal. Si no coincide el servidor no
     * envía nada y el jugador entra sin voz en vez de ser expulsado. Subir al cambiar
     * los campos.
     */
    public static final int PROTOCOL = Protocol.VERSION;
    public static final ResourceLocation ID = ResourceLocation.fromNamespaceAndPath(Protocol.NAMESPACE, Protocol.PATH);
    public static final CustomPacketPayload.Type<VoiceSetupPayload> TYPE = new CustomPacketPayload.Type<>(ID);
    /**
     * long en 8 bytes big endian. En 1.21.1 no existe {@code ByteBufCodecs.LONG}, y
     * {@code VAR_LONG} no vale: Paper y Forge lo envían con 8 bytes fijos.
     */
    private static final StreamCodec<ByteBuf, Long> LONG = new StreamCodec<>() {
        @Override
        public Long decode(ByteBuf buf) {
            return buf.readLong();
        }

        @Override
        public void encode(ByteBuf buf, Long value) {
            buf.writeLong(value);
        }
    };

    public static final StreamCodec<RegistryFriendlyByteBuf, VoiceSetupPayload> CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, VoiceSetupPayload::host,
            ByteBufCodecs.VAR_INT, VoiceSetupPayload::port,
            ByteBufCodecs.BYTE_ARRAY, VoiceSetupPayload::key,
            ByteBufCodecs.VAR_INT, VoiceSetupPayload::transport,
            LONG, VoiceSetupPayload::idMsb,
            LONG, VoiceSetupPayload::idLsb,
            VoiceSetupPayload::new);

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
