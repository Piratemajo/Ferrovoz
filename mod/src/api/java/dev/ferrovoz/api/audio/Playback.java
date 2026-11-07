package dev.ferrovoz.api.audio;

import java.util.concurrent.CompletableFuture;

/** Una reproducción en curso de {@link AudioChannel#play}. */
public interface Playback {
    boolean isPlaying();

    void stop();

    /**
     * Se completa al acabar, por lo que sea. Si la fuente lanza una excepción, se
     * completa con ella.
     */
    CompletableFuture<Void> finished();
}
