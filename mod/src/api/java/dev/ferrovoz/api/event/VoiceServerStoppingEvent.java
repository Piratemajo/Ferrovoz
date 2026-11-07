package dev.ferrovoz.api.event;

/** El relé de voz va a pararse. Los canales de audio se cierran justo después. */
public record VoiceServerStoppingEvent() implements VoiceEvent {}
