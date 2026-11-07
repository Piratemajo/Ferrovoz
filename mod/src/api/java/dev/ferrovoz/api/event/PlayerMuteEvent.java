package dev.ferrovoz.api.event;

import java.util.Optional;
import java.util.UUID;

/** Un moderador va a silenciar (o a quitar el silencio) con {@code /voz silenciar}. */
public final class PlayerMuteEvent extends CancellableEvent {
    private final UUID target;
    private final boolean mute;
    private final UUID moderator;

    /** {@code moderator} null si es la consola. */
    public PlayerMuteEvent(UUID target, boolean mute, UUID moderator) {
        this.target = target;
        this.mute = mute;
        this.moderator = moderator;
    }

    public UUID target() {
        return target;
    }

    /** true al silenciar, false al quitar el silencio. */
    public boolean mute() {
        return mute;
    }

    public Optional<UUID> moderator() {
        return Optional.ofNullable(moderator);
    }
}
