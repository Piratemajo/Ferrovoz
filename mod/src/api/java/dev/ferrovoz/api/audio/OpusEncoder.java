package dev.ferrovoz.api.audio;

/** Codificador Opus. No es seguro usar uno desde varios hilos a la vez. */
public interface OpusEncoder extends AutoCloseable {
    /**
     * @param frame {@link AudioFormat#FRAME_SAMPLES} muestras
     * @return la trama Opus
     * @throws IllegalArgumentException si no son 960 muestras
     * @throws IllegalStateException si ya está cerrado
     */
    byte[] encode(short[] frame);

    @Override
    void close();
}
