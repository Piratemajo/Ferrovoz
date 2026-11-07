package dev.ferrovoz.api.event;

import java.util.Optional;
import java.util.UUID;

/** Se ha borrado un grupo. {@code by} está vacío si lo borró un plugin o la consola. */
public record GroupDeletedEvent(String group, Optional<UUID> by) implements VoiceEvent {}
