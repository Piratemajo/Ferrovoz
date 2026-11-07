package dev.ferrovoz.api.event;

import java.util.UUID;

/** Ha cambiado quién está en un grupo. Ya ha pasado: no se puede cancelar. */
public record GroupMemberEvent(Type type, String group, UUID player) implements VoiceEvent {
    public enum Type {
        JOINED,
        LEFT,
        KICKED,
        /** Volvió a entrar al servidor y se le devolvió a su grupo permanente. */
        REJOINED,
        /** {@code player} es el nuevo dueño. */
        NEW_OWNER
    }
}
