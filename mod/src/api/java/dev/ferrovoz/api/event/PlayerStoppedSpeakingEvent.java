package dev.ferrovoz.api.event;

import dev.ferrovoz.api.VoicePlayer;

/** Hace un momento (unos 400 ms) que no se le oye. */
public record PlayerStoppedSpeakingEvent(VoicePlayer player) implements VoiceEvent {}
