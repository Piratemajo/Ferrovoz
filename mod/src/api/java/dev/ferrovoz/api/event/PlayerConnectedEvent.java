package dev.ferrovoz.api.event;

import dev.ferrovoz.api.VoicePlayer;

/** A un jugador ya le llega la voz (su cliente ha contactado con el relé). */
public record PlayerConnectedEvent(VoicePlayer player) implements VoiceEvent {}
