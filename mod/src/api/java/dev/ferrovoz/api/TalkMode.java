package dev.ferrovoz.api;

/** Cómo habla un jugador por proximidad. Cada modo tiene su alcance. */
public enum TalkMode {
    WHISPER,
    NORMAL,
    SHOUT;

    /** Valor del protocolo: 0 normal, 1 susurro, 2 grito. */
    public static TalkMode fromWire(int value) {
        return switch (value) {
            case 1 -> WHISPER;
            case 2 -> SHOUT;
            default -> NORMAL;
        };
    }
}
