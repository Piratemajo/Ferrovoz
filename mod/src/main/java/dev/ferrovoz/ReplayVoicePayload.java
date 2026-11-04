package dev.ferrovoz;

import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/**
 * Trama de voz guardada en una repetición (Replay Mod o Flashback).
 *
 * <p>No la envía ningún servidor: el cliente la inserta en la grabación como si
 * viniera del servidor y al reproducir llega como cualquier otro paquete. Contenido:
 * un byte de formato ({@link #FORMAT}) y la trama sin cifrar (hablante, secuencia,
 * alcance, grupo, agua, reverb y Opus).
 *
 * <p>El canal ({@code ferrovoz:replay_voice}) no lleva versión porque las repeticiones
 * se guardan; si cambia el formato, cambia el primer byte.
 */
public record ReplayVoicePayload(byte[] data) implements CustomPacketPayload {
    public static final byte FORMAT = 1;
    public static final Identifier ID = Identifier.fromNamespaceAndPath(Protocol.NAMESPACE, "replay_voice");
    public static final CustomPacketPayload.Type<ReplayVoicePayload> TYPE = new CustomPacketPayload.Type<>(ID);
    public static final StreamCodec<RegistryFriendlyByteBuf, ReplayVoicePayload> CODEC = StreamCodec.composite(
            ByteBufCodecs.BYTE_ARRAY, ReplayVoicePayload::data,
            ReplayVoicePayload::new);

    /** Añade el byte de formato a una trama del núcleo. */
    public static ReplayVoicePayload of(byte[] frame) {
        byte[] data = new byte[frame.length + 1];
        data[0] = FORMAT;
        System.arraycopy(frame, 0, data, 1, frame.length);
        return new ReplayVoicePayload(data);
    }

    /** La trama sin el byte de formato, o null si el formato es desconocido. */
    public byte[] frame() {
        if (data.length < 2 || data[0] != FORMAT) return null;
        byte[] frame = new byte[data.length - 1];
        System.arraycopy(data, 1, frame, 0, frame.length);
        return frame;
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
