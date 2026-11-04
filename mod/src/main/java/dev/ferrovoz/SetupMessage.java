package dev.ferrovoz;

import java.io.ByteArrayOutputStream;
import java.io.DataOutputStream;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * Codifica el mensaje de {@link Protocol#CHANNEL} a bytes, igual que el
 * {@code StreamCodec} de {@code VoiceSetupPayload}. Lo usan el plugin y Forge, que
 * trabajan con plugin messages en bruto.
 *
 * <p>Codificación de Minecraft: VarInt de 7 bits por byte con bit de continuación;
 * cadenas y arrays con VarInt de longitud delante; float y long en big endian.
 */
public final class SetupMessage {
    private SetupMessage() {}

    public static byte[] encode(String host, int port, byte[] key, int transport, UUID id) {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream(96);
        try (DataOutputStream out = new DataOutputStream(bytes)) {
            byte[] hostUtf8 = host.getBytes(StandardCharsets.UTF_8);
            writeVarInt(out, hostUtf8.length);
            out.write(hostUtf8);
            writeVarInt(out, port);
            writeVarInt(out, key.length);
            out.write(key);
            writeVarInt(out, transport);
            out.writeLong(id.getMostSignificantBits());
            out.writeLong(id.getLeastSignificantBits());
        } catch (IOException e) {
            throw new UncheckedIOException(e); // no pasa en memoria
        }
        return bytes.toByteArray();
    }

    /** Mensaje de {@link Protocol#VERSION_CHANNEL} (formato fijo). */
    public static byte[] encodeVersion(int protocol, String version, boolean voiceAvailable) {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream(32);
        try (DataOutputStream out = new DataOutputStream(bytes)) {
            writeVarInt(out, protocol);
            byte[] v = version.getBytes(StandardCharsets.UTF_8);
            writeVarInt(out, v.length);
            out.write(v);
            out.writeBoolean(voiceAvailable);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return bytes.toByteArray();
    }

    /** Paquete del túnel: VarInt longitud + bytes (como {@code ByteBufCodecs.BYTE_ARRAY}). */
    public static byte[] encodeTunnel(byte[] packet) {
        ByteArrayOutputStream bytes = new ByteArrayOutputStream(packet.length + 3);
        try (DataOutputStream out = new DataOutputStream(bytes)) {
            writeVarInt(out, packet.length);
            out.write(packet);
        } catch (IOException e) {
            throw new UncheckedIOException(e);
        }
        return bytes.toByteArray();
    }

    /** Inverso de {@link #encodeTunnel}; null si el mensaje está mal formado. */
    public static byte[] decodeTunnel(byte[] message) {
        int value = 0, pos = 0, i = 0;
        while (true) {
            if (i >= message.length || pos > 28) return null;
            byte b = message[i++];
            value |= (b & 0x7F) << pos;
            if ((b & 0x80) == 0) break;
            pos += 7;
        }
        if (value < 0 || value > 2048 || i + value != message.length) return null;
        byte[] out = new byte[value];
        System.arraycopy(message, i, out, 0, value);
        return out;
    }

    // --- Lectura (Forge recibe bytes, no records) ---

    /** Igual que {@code VoiceSetupPayload}, sin depender de Minecraft. */
    public record Setup(String host, int port, byte[] key, int transport, long idMsb, long idLsb) {}

    /** Igual que {@code VersionPayload}, sin depender de Minecraft. */
    public record Version(int protocol, String version, boolean voiceAvailable) {}

    /** Inverso de {@link #encode}; null si el mensaje está mal formado. */
    public static Setup decodeSetup(byte[] message) {
        try {
            Reader r = new Reader(message);
            String host = r.string(255);
            int port = r.varInt();
            byte[] key = r.bytes(64);
            int transport = r.varInt();
            long msb = r.longBE(), lsb = r.longBE();
            return r.done() ? new Setup(host, port, key, transport, msb, lsb) : null;
        } catch (IndexOutOfBoundsException | IllegalArgumentException e) {
            return null;
        }
    }

    /** Inverso de {@link #encodeVersion}; null si el mensaje está mal formado. */
    public static Version decodeVersion(byte[] message) {
        try {
            Reader r = new Reader(message);
            int protocol = r.varInt();
            String version = r.string(64);
            boolean available = r.u8() != 0;
            return r.done() ? new Version(protocol, version, available) : null;
        } catch (IndexOutOfBoundsException | IllegalArgumentException e) {
            return null;
        }
    }

    private static final class Reader {
        private final byte[] b;
        private int i;

        Reader(byte[] b) {
            this.b = b;
        }

        int u8() {
            return b[i++] & 0xFF;
        }

        int varInt() {
            int value = 0;
            for (int shift = 0; shift <= 28; shift += 7) {
                int x = u8();
                value |= (x & 0x7F) << shift;
                if ((x & 0x80) == 0) return value;
            }
            throw new IllegalArgumentException("VarInt demasiado largo");
        }

        byte[] bytes(int max) {
            int n = varInt();
            if (n < 0 || n > max) throw new IllegalArgumentException("longitud " + n);
            byte[] out = new byte[n];
            System.arraycopy(b, i, out, 0, n);
            i += n;
            return out;
        }

        String string(int maxBytes) {
            return new String(bytes(maxBytes * 4), StandardCharsets.UTF_8);
        }

        long longBE() {
            long v = 0;
            for (int k = 0; k < 8; k++) v = (v << 8) | u8();
            return v;
        }

        boolean done() {
            return i == b.length;
        }
    }

    static void writeVarInt(DataOutputStream out, int value) throws IOException {
        while ((value & ~0x7F) != 0) {
            out.writeByte((value & 0x7F) | 0x80);
            value >>>= 7;
        }
        out.writeByte(value);
    }
}
