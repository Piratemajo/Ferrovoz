package dev.ferrovoz.api.event;

import java.util.UUID;

/** Un jugador ha dejado la voz: ha salido del servidor o la ha desactivado. */
public record PlayerDisconnectedEvent(UUID player) implements VoiceEvent {}
