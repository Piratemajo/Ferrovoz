package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Position;

import java.util.Collection;
import java.util.UUID;

/**
 * A server-side speaker. Each call to {@link #send} is one 20 ms frame: send them in
 * real time, 50 per second, or use {@link #play}, which paces them for you.
 *
 * <p>Every Ferrovoz client plays it, including those older than the API (which list it
 * among the people talking under an odd name). It closes by itself when the relay stops.
 */
public interface AudioChannel extends AutoCloseable {
    /** The id clients see it under. Starts with {@code 66657272-6f76-6f7a} ("ferrovoz"). */
    UUID id();

    boolean isOpen();

    void moveTo(Position position);

    void follow(UUID player);

    void sendTo(Collection<UUID> listeners);

    void sendToGroup(String group);

    /** Range when it has a position, from 1 to 30000 blocks. */
    void setRange(double blocks);

    /** Volume of what is sent with {@link #send}, from 0 to 4 (1 = unchanged). */
    void setVolume(float volume);

    float volume();

    /**
     * Encodes and sends 20 ms: {@link AudioFormat#FRAME_SAMPLES} samples.
     *
     * @return false if the channel is closed
     * @throws IllegalArgumentException if the frame doesn't have 960 samples
     */
    boolean send(short[] frame);

    /** Sends an already encoded Opus frame (20 ms, 48 kHz, mono, up to 1275 bytes). */
    boolean sendOpus(byte[] frame);

    /**
     * Plays a source in real time on a Ferrovoz thread, replacing whatever was playing
     * on this channel. Playback ends when the source returns null, when it is stopped or
     * when the channel closes.
     */
    Playback play(FrameSource source);

    /** Stops the current playback, if any. */
    void stop();

    @Override
    void close();
}
