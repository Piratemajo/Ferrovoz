package dev.ferrovoz.api.event;

import dev.ferrovoz.api.TalkMode;
import dev.ferrovoz.api.VoicePlayer;

/**
 * A player starts being heard. Not fired while they're muted. {@code group} is true if
 * they're talking to their group; {@code mode} doesn't matter then.
 */
public record PlayerStartedSpeakingEvent(VoicePlayer player, TalkMode mode, boolean group) implements VoiceEvent {}
