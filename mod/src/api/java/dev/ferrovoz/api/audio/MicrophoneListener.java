package dev.ferrovoz.api.audio;

/** Recibe la voz de los jugadores (ver {@link AudioService#listen}). */
@FunctionalInterface
public interface MicrophoneListener {
    void onFrame(MicrophoneFrame frame);
}
