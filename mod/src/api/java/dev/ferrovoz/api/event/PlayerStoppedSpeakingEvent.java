package dev.ferrovoz.api.event;

import dev.ferrovoz.api.VoicePlayer;

/** The player hasn't been heard for a moment (about 400 ms). */
public record PlayerStoppedSpeakingEvent(VoicePlayer player) implements VoiceEvent {}
