package dev.ferrovoz.api.event;

import java.util.Optional;
import java.util.UUID;

/** A moderator is about to mute (or unmute) someone with {@code /voice mute}. */
public final class PlayerMuteEvent extends CancellableEvent {
    private final UUID target;
    private final boolean mute;
    private final UUID moderator;

    /** {@code moderator} is null for the console. */
    public PlayerMuteEvent(UUID target, boolean mute, UUID moderator) {
        this.target = target;
        this.mute = mute;
        this.moderator = moderator;
    }

    public UUID target() {
        return target;
    }

    /** true when muting, false when unmuting. */
    public boolean mute() {
        return mute;
    }

    public Optional<UUID> moderator() {
        return Optional.ofNullable(moderator);
    }
}
