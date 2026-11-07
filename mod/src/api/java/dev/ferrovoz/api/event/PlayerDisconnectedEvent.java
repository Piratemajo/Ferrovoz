package dev.ferrovoz.api.event;

import java.util.UUID;

/** A player left voice chat: they left the server or turned voice off. */
public record PlayerDisconnectedEvent(UUID player) implements VoiceEvent {}
