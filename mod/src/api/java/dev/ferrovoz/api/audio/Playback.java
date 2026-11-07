package dev.ferrovoz.api.audio;

import java.util.concurrent.CompletableFuture;

/** A playback in progress, from {@link AudioChannel#play}. */
public interface Playback {
    boolean isPlaying();

    void stop();

    /**
     * Completes when it ends, for whatever reason. If the source throws, it completes
     * exceptionally with that exception.
     */
    CompletableFuture<Void> finished();
}
