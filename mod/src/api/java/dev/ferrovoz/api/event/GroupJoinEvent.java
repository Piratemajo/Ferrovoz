package dev.ferrovoz.api.event;

import java.util.UUID;

/**
 * A player is about to join a group with {@code /voice group join}. Fired before the
 * password is checked.
 */
public final class GroupJoinEvent extends CancellableEvent {
    private final UUID player;
    private final String group;

    public GroupJoinEvent(UUID player, String group) {
        this.player = player;
        this.group = group;
    }

    public UUID player() {
        return player;
    }

    public String group() {
        return group;
    }
}
