package dev.ferrovoz.api.audio;

import java.util.Arrays;
import java.util.Objects;

/**
 * Audio para {@link AudioChannel#play}: devuelve una trama de
 * {@link AudioFormat#FRAME_SAMPLES} muestras cada vez, o null al acabar. Se llama en
 * el hilo de reproducción de Ferrovoz, cada 20 ms; tiene que ser rápido.
 */
@FunctionalInterface
public interface FrameSource {
    short[] next();

    /** Un audio entero (48 kHz, mono) troceado en tramas; la última se rellena con silencio. */
    static FrameSource of(short[] samples) {
        Objects.requireNonNull(samples, "samples");
        return new FrameSource() {
            private int pos;

            @Override
            public short[] next() {
                if (pos >= samples.length) return null;
                int end = Math.min(pos + AudioFormat.FRAME_SAMPLES, samples.length);
                short[] frame = Arrays.copyOf(Arrays.copyOfRange(samples, pos, end), AudioFormat.FRAME_SAMPLES);
                pos = end;
                return frame;
            }
        };
    }

    /** El mismo audio en bucle, hasta que se pare la reproducción. */
    static FrameSource loop(short[] samples) {
        Objects.requireNonNull(samples, "samples");
        if (samples.length == 0) throw new IllegalArgumentException("audio vacío");
        return new FrameSource() {
            private int pos;

            @Override
            public short[] next() {
                short[] frame = new short[AudioFormat.FRAME_SAMPLES];
                for (int i = 0; i < frame.length; i++) {
                    frame[i] = samples[pos];
                    pos = (pos + 1) % samples.length;
                }
                return frame;
            }
        };
    }
}
