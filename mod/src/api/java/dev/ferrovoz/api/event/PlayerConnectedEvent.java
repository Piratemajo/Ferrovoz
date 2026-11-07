package dev.ferrovoz.api.event;

import dev.ferrovoz.api.VoicePlayer;

/** A player's voice is now flowing (their client has reached the relay). */
public record PlayerConnectedEvent(VoicePlayer player) implements VoiceEvent {}
