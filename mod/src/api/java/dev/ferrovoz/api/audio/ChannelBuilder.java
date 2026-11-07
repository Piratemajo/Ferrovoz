package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Position;

import java.util.Collection;
import java.util.UUID;

/**
 * Options for a channel and where it plays. The methods returning {@link AudioChannel}
 * create it; up to 1024 can be open at once on the whole server.
 *
 * @throws IllegalStateException on creation, if the relay isn't running or 1024 are already open
 */
public interface ChannelBuilder {
    /** Opus bitrate, from 6000 to 256000 (48000 by default). */
    ChannelBuilder bitrate(int bitsPerSecond);

    /** For music and sound effects rather than speech (the Opus mode that treats non-speech better). */
    ChannelBuilder music();

    /** Plays from a point; heard in 3D by anyone within {@code range} blocks. */
    AudioChannel at(Position position, double range);

    /** Plays from wherever a player is (they hear it too), in 3D. */
    AudioChannel following(UUID player, double range);

    /** No position: to these players, wherever they are. */
    AudioChannel to(Collection<UUID> listeners);

    /** No position: to whoever is in that group at each moment. */
    AudioChannel toGroup(String group);
}
