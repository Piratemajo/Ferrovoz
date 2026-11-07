package dev.ferrovoz.api.event;

import dev.ferrovoz.api.TalkMode;
import dev.ferrovoz.api.VoicePlayer;

/**
 * Empieza a oírse a un jugador. No sale si está silenciado. {@code group} es true si
 * habla a su grupo; entonces {@code mode} no importa.
 */
public record PlayerStartedSpeakingEvent(VoicePlayer player, TalkMode mode, boolean group) implements VoiceEvent {}
