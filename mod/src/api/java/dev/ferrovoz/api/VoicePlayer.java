package dev.ferrovoz.api;

import java.util.Optional;
import java.util.OptionalDouble;
import java.util.UUID;

/**
 * A player with Ferrovoz. It is a view: every call reads the current state, so it can
 * be kept around while the player is online.
 *
 * <p>What a plugin sets ({@link #setRange}, {@link #setPermission}) lasts while the
 * server is running, including if the player leaves and comes back. It is not saved
 * to disk.
 */
public interface VoicePlayer {
    UUID id();

    /** In-game name (or the UUID once they've left). */
    String name();

    /** Their voice is flowing: the client has reached the relay. */
    boolean isConnected();

    /** They can be heard right now. */
    boolean isSpeaking();

    /** They muted their own microphone (M key). */
    boolean isMicrophoneMuted();

    /** They deafened themselves (hear nobody). */
    boolean isDeafened();

    /** Muted by a moderator. Saved to disk, like {@code /voice mute}. */
    boolean isMuted();

    /** Mutes or unmutes as a moderator. Does not fire {@code PlayerMuteEvent}. */
    void setMuted(boolean muted);

    /** Their group, if they're in one. */
    Optional<VoiceGroup> group();

    /** Packet loss on their connection, in percent. 0 if unknown. */
    int packetLoss();

    /**
     * A fixed range in blocks instead of the whisper, normal or shout one: a megaphone, a
     * stage. Between 1 and 30000.
     */
    void setRange(double blocks);

    /** Goes back to the range of each talk mode. */
    void resetRange();

    /** The range set with {@link #setRange}, if any. */
    OptionalDouble range();

    /** What a plugin has set (or {@link PermissionOverride#INHERIT}). */
    PermissionOverride permissionOverride(VoicePermission permission);

    /** Forces a permission, or hands it back to the permission system with {@code INHERIT}. Applies immediately. */
    void setPermission(VoicePermission permission, PermissionOverride value);

    /** The permission they have now, including plugin overrides. */
    boolean hasPermission(VoicePermission permission);
}
