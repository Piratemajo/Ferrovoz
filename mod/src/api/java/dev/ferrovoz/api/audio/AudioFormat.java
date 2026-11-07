package dev.ferrovoz.api.audio;

/** El único formato que usa Ferrovoz: 48 kHz, mono, tramas de 20 ms, PCM de 16 bits. */
public final class AudioFormat {
    public static final int SAMPLE_RATE = 48_000;
    /** Muestras en una trama de 20 ms. */
    public static final int FRAME_SAMPLES = 960;
    public static final int FRAME_MILLIS = 20;

    private AudioFormat() {}
}
