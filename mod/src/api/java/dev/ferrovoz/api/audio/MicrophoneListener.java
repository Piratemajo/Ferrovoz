package dev.ferrovoz.api.audio;

/** Receives players' voice (see {@link AudioService#listen}). */
@FunctionalInterface
public interface MicrophoneListener {
    void onFrame(MicrophoneFrame frame);
}
