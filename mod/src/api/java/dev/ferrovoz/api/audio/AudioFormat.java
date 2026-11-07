package dev.ferrovoz.api.audio;

/** The only format Ferrovoz uses: 48 kHz, mono, 20 ms frames, 16-bit PCM. */
public final class AudioFormat {
    public static final int SAMPLE_RATE = 48_000;
    /** Samples in a 20 ms frame. */
    public static final int FRAME_SAMPLES = 960;
    public static final int FRAME_MILLIS = 20;

    private AudioFormat() {}
}
