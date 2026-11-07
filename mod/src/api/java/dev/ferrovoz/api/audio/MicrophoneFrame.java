package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.TalkMode;

import java.util.UUID;

/**
 * 20 ms of a player's voice, as Opus (48 kHz, mono). {@code sequence} goes up by one
 * each frame: a jump means a lost frame, which {@code decoder.decode(null)} can fill in.
 */
public record MicrophoneFrame(UUID player, int sequence, TalkMode mode, boolean group, byte[] opus) {}
