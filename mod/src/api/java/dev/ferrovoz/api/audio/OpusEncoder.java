package dev.ferrovoz.api.audio;

/** Opus encoder. Not safe to use from several threads at once. */
public interface OpusEncoder extends AutoCloseable {
    /**
     * @param frame {@link AudioFormat#FRAME_SAMPLES} samples
     * @return the Opus frame
     * @throws IllegalArgumentException if it isn't 960 samples
     * @throws IllegalStateException if it's already closed
     */
    byte[] encode(short[] frame);

    @Override
    void close();
}
