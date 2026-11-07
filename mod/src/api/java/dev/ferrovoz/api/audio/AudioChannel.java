package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Position;

import java.util.Collection;
import java.util.UUID;

/**
 * Un altavoz del servidor. Cada llamada a {@link #send} es una trama de 20 ms: hay que
 * enviarlas a ritmo, 50 por segundo, o usar {@link #play}, que lo hace solo.
 *
 * <p>Lo reproducen todos los clientes de Ferrovoz, también los anteriores a la API
 * (que lo muestran en la lista de quién habla con un nombre raro). Se cierra solo al
 * parar el relé.
 */
public interface AudioChannel extends AutoCloseable {
    /** Id con el que lo ven los clientes. Empieza por {@code 66657272-6f76-6f7a} ("ferrovoz"). */
    UUID id();

    boolean isOpen();

    void moveTo(Position position);

    void follow(UUID player);

    void sendTo(Collection<UUID> listeners);

    void sendToGroup(String group);

    /** Alcance cuando tiene posición, de 1 a 30000 bloques. */
    void setRange(double blocks);

    /** Volumen de lo que se manda con {@link #send}, de 0 a 4 (1 = tal cual). */
    void setVolume(float volume);

    float volume();

    /**
     * Codifica y envía 20 ms: {@link AudioFormat#FRAME_SAMPLES} muestras.
     *
     * @return false si el canal está cerrado
     * @throws IllegalArgumentException si la trama no tiene 960 muestras
     */
    boolean send(short[] frame);

    /** Envía una trama Opus ya codificada (20 ms, 48 kHz, mono, hasta 1275 bytes). */
    boolean sendOpus(byte[] frame);

    /**
     * Reproduce una fuente a ritmo, en un hilo de Ferrovoz. Para lo que esté sonando en
     * este canal. La reproducción acaba cuando la fuente devuelve null, al pararla o al
     * cerrar el canal.
     */
    Playback play(FrameSource source);

    /** Para la reproducción en curso, si hay. */
    void stop();

    @Override
    void close();
}
