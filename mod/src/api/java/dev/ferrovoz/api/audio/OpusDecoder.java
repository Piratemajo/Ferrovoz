package dev.ferrovoz.api.audio;

/**
 * Decodificador Opus, con estado: uno por cada fuente, y las tramas en orden. No es
 * seguro usar uno desde varios hilos a la vez.
 */
public interface OpusDecoder extends AutoCloseable {
    /**
     * @param frame trama Opus, o null si se perdió (rellena el hueco de forma natural)
     * @return {@link AudioFormat#FRAME_SAMPLES} muestras
     * @throws IllegalStateException si ya está cerrado
     */
    short[] decode(byte[] frame);

    @Override
    void close();
}
