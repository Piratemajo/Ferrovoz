package dev.ferrovoz.api.event;

import java.util.UUID;

/**
 * Un jugador va a entrar en un grupo con {@code /voz grupo unirse}. Se lanza antes de
 * comprobar la contraseña.
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
