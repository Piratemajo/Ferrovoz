package dev.ferrovoz.api.audio;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Lee archivos WAV y los deja como los quiere Ferrovoz: 48 kHz, mono, 16 bits. Admite
 * PCM de 8, 16, 24 y 32 bits, float de 32 bits, cualquier número de canales (se
 * mezclan) y cualquier frecuencia (se convierte).
 */
public final class Wav {
    /** 10 minutos a 48 kHz: lo que cabe razonablemente en memoria de una vez. */
    private static final long MAX_SAMPLES = 48_000L * 60 * 10;

    private Wav() {}

    /** Lee todo el WAV. No cierra el stream. */
    public static short[] read(InputStream in) throws IOException {
        ByteBuffer b = ByteBuffer.wrap(readAll(in)).order(ByteOrder.LITTLE_ENDIAN);
        if (b.remaining() < 12 || b.getInt() != 0x46464952 /* RIFF */) throw new IOException("no es un WAV (falta RIFF)");
        b.getInt();
        if (b.getInt() != 0x45564157 /* WAVE */) throw new IOException("no es un WAV (falta WAVE)");
        int format = -1, channels = 0, rate = 0, bits = 0;
        while (b.remaining() >= 8) {
            int id = b.getInt();
            long size = Integer.toUnsignedLong(b.getInt());
            if (size > b.remaining()) size = b.remaining(); // algunos archivos ponen un tamaño de más
            int start = b.position();
            if (id == 0x20746d66 /* fmt  */) {
                if (size < 16) throw new IOException("bloque fmt demasiado corto");
                format = Short.toUnsignedInt(b.getShort());
                channels = Short.toUnsignedInt(b.getShort());
                rate = b.getInt();
                b.getInt();
                b.getShort();
                bits = Short.toUnsignedInt(b.getShort());
                if (format == 0xFFFE && size >= 26) { // WAVE_FORMAT_EXTENSIBLE: el formato real va dentro
                    b.position(start + 24);
                    format = Short.toUnsignedInt(b.getShort());
                }
            } else if (id == 0x61746164 /* data */) {
                if (format < 0) throw new IOException("bloque data antes que fmt");
                ByteBuffer data = b.slice(start, (int) size).order(ByteOrder.LITTLE_ENDIAN);
                return resample(toMono(data, format, channels, bits), rate);
            }
            b.position(start + (int) size + (int) (size & 1)); // los bloques van alineados a 2
        }
        throw new IOException("WAV sin bloque data");
    }

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[1 << 16];
        long limit = MAX_SAMPLES * 4 * 8 + (1 << 16); // 8 canales de 32 bits como mucho
        for (int n; (n = in.read(buf)) > 0; ) {
            out.write(buf, 0, n);
            if (out.size() > limit) throw new IOException("WAV demasiado largo (más de 10 minutos)");
        }
        return out.toByteArray();
    }

    private static float[] toMono(ByteBuffer data, int format, int channels, int bits) throws IOException {
        boolean pcm = format == 1, flt = format == 3;
        if (channels < 1 || channels > 8) throw new IOException("canales no admitidos: " + channels);
        if (!(pcm && (bits == 8 || bits == 16 || bits == 24 || bits == 32)) && !(flt && bits == 32)) {
            throw new IOException("formato no admitido (formato " + format + ", " + bits + " bits)");
        }
        int bytes = bits / 8;
        int frames = data.remaining() / (bytes * channels);
        if (frames > MAX_SAMPLES * 8) throw new IOException("WAV demasiado largo");
        float[] out = new float[frames];
        for (int f = 0; f < frames; f++) {
            float sum = 0;
            for (int c = 0; c < channels; c++) {
                sum += switch (bits) {
                    case 8 -> ((data.get() & 0xFF) - 128) / 128f;
                    case 16 -> data.getShort() / 32768f;
                    case 24 -> ((data.get() & 0xFF) | (data.get() & 0xFF) << 8 | data.get() << 16) / 8388608f;
                    default -> flt ? data.getFloat() : data.getInt() / 2147483648f;
                };
            }
            out[f] = sum / channels;
        }
        return out;
    }

    /** Interpolación lineal a 48 kHz. Suficiente para voz y efectos. */
    private static short[] resample(float[] in, int rate) throws IOException {
        if (rate < 1000 || rate > 384_000) throw new IOException("frecuencia no admitida: " + rate);
        long n = in.length == 0 ? 0 : (long) Math.floor((in.length - 1) * (double) AudioFormat.SAMPLE_RATE / rate) + 1;
        if (n > MAX_SAMPLES) throw new IOException("WAV demasiado largo (más de 10 minutos)");
        short[] out = new short[(int) n];
        double step = (double) rate / AudioFormat.SAMPLE_RATE;
        for (int i = 0; i < out.length; i++) {
            double pos = i * step;
            int a = (int) pos;
            int b = Math.min(a + 1, in.length - 1);
            double t = pos - a;
            double v = in[a] * (1 - t) + in[b] * t;
            out[i] = (short) Math.round(Math.max(-1, Math.min(1, v)) * 32767);
        }
        return out;
    }
}
