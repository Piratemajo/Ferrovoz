package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.TalkMode;

import java.util.UUID;

/**
 * 20 ms de la voz de un jugador, en Opus (48 kHz, mono). {@code sequence} sube de uno
 * en uno: un salto es una trama perdida, que se puede rellenar con
 * {@code decoder.decode(null)}.
 */
public record MicrophoneFrame(UUID player, int sequence, TalkMode mode, boolean group, byte[] opus) {}
