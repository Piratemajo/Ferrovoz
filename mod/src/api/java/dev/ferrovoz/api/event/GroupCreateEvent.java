package dev.ferrovoz.api.event;

import java.util.UUID;

/** Un jugador va a crear un grupo con {@code /voz grupo crear}. */
public final class GroupCreateEvent extends CancellableEvent {
    private final UUID player;
    private final String group;
    private final boolean withPassword;

    public GroupCreateEvent(UUID player, String group, boolean withPassword) {
        this.player = player;
        this.group = group;
        this.withPassword = withPassword;
    }

    public UUID player() {
        return player;
    }

    public String group() {
        return group;
    }

    public boolean withPassword() {
        return withPassword;
    }
}
