package dev.ferrovoz.api.event;

/** The voice relay is about to stop. Audio channels are closed right after. */
public record VoiceServerStoppingEvent() implements VoiceEvent {}
