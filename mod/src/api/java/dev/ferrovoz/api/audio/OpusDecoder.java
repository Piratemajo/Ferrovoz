package dev.ferrovoz.api.audio;

/**
 * Opus decoder. It keeps state: use one per source and feed frames in order. Not safe
 * to use from several threads at once.
 */
public interface OpusDecoder extends AutoCloseable {
    /**
     * @param frame an Opus frame, or null if it was lost (the gap is filled in naturally)
     * @return {@link AudioFormat#FRAME_SAMPLES} samples
     * @throws IllegalStateException if it's already closed
     */
    short[] decode(byte[] frame);

    @Override
    void close();
}
