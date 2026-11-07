package dev.ferrovoz.api.event;

import java.util.Optional;
import java.util.UUID;

/** A group was deleted. {@code by} is empty if a plugin or the console deleted it. */
public record GroupDeletedEvent(String group, Optional<UUID> by) implements VoiceEvent {}
