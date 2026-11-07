package dev.ferrovoz.api.event;

import java.util.UUID;

/** Group membership changed. It has already happened: it can't be cancelled. */
public record GroupMemberEvent(Type type, String group, UUID player) implements VoiceEvent {
    public enum Type {
        JOINED,
        LEFT,
        KICKED,
        /** They came back to the server and were put back in their persistent group. */
        REJOINED,
        /** {@code player} is the new owner. */
        NEW_OWNER
    }
}
