package dev.ferrovoz.api.audio;

import java.util.Arrays;
import java.util.Objects;

/**
 * Audio for {@link AudioChannel#play}: returns a frame of
 * {@link AudioFormat#FRAME_SAMPLES} samples each time, or null at the end. It is called
 * on Ferrovoz's playback thread every 20 ms, so it must be fast.
 */
@FunctionalInterface
public interface FrameSource {
    short[] next();

    /** A whole clip (48 kHz, mono) split into frames; the last one is padded with silence. */
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

    /** The same clip on a loop, until the playback is stopped. */
    static FrameSource loop(short[] samples) {
        Objects.requireNonNull(samples, "samples");
        if (samples.length == 0) throw new IllegalArgumentException("empty clip");
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
