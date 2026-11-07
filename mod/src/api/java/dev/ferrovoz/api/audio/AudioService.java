package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Subscription;

/** Sound from plugins, voice listening and the Opus codec. */
public interface AudioService {
    /**
     * Starts building an audio channel: a speaker driven by the server that players hear
     * as if someone were talking.
     *
     * <pre>{@code
     * AudioChannel radio = api.audio().channel().music().at(position, 32);
     * radio.play(FrameSource.of(Wav.read(input)));
     * }</pre>
     */
    ChannelBuilder channel();

    /**
     * Receives every player's voice, frame by frame, as Opus. Only what is relayed: a
     * muted player's voice doesn't arrive. The listener runs on the
     * {@code ferrovoz-microphone} thread, about 50 times per second for each player
     * talking: return quickly and move anything slow to another thread.
     *
     * <p>While nobody is listening, the relay copies nothing.
     */
    Subscription listen(MicrophoneListener listener);

    /** Opus encoder for 20 ms at 48 kHz mono. {@code music} for music and sound effects. */
    OpusEncoder createEncoder(int bitrate, boolean music);

    /** Opus decoder: one for each player or source being decoded. */
    OpusDecoder createDecoder();
}
