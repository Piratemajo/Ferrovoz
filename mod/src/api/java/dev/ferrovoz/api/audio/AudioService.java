package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Subscription;

/** Sonido de los plugins, escucha de la voz y códec Opus. */
public interface AudioService {
    /**
     * Empieza a crear un canal de audio: un altavoz que manda el servidor y oyen los
     * jugadores como si alguien hablase.
     *
     * <pre>{@code
     * AudioChannel radio = api.audio().channel().music().at(position, 32);
     * radio.play(FrameSource.of(Wav.read(input)));
     * }</pre>
     */
    ChannelBuilder channel();

    /**
     * Recibe la voz de todos los jugadores, trama a trama, en Opus. Solo lo que se
     * reenvía: lo de un jugador silenciado no llega. El listener se llama en el hilo
     * {@code ferrovoz-microphone}, unas 50 veces por segundo por cada uno que habla:
     * hay que devolver rápido y llevar a otro hilo lo que sea lento.
     *
     * <p>Mientras no haya nadie escuchando, el relé no copia nada.
     */
    Subscription listen(MicrophoneListener listener);

    /** Codificador Opus de 20 ms a 48 kHz mono. {@code music} para música y efectos. */
    OpusEncoder createEncoder(int bitrate, boolean music);

    /** Decodificador Opus: uno por cada jugador o fuente que se decodifique. */
    OpusDecoder createDecoder();
}
