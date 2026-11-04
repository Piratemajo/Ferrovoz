package dev.ferrovoz;

import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

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
    public static final Identifier ID = Identifier.fromNamespaceAndPath(Protocol.NAMESPACE, Protocol.PATH);
    public static final CustomPacketPayload.Type<VoiceSetupPayload> TYPE = new CustomPacketPayload.Type<>(ID);
    public static final StreamCodec<RegistryFriendlyByteBuf, VoiceSetupPayload> CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, VoiceSetupPayload::host,
            ByteBufCodecs.VAR_INT, VoiceSetupPayload::port,
            ByteBufCodecs.BYTE_ARRAY, VoiceSetupPayload::key,
            ByteBufCodecs.VAR_INT, VoiceSetupPayload::transport,
            ByteBufCodecs.LONG, VoiceSetupPayload::idMsb,
            ByteBufCodecs.LONG, VoiceSetupPayload::idLsb,
            VoiceSetupPayload::new);

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
