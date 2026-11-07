package dev.ferrovoz.api;

/** How a player talks by proximity. Each mode has its own range. */
public enum TalkMode {
    WHISPER,
    NORMAL,
    SHOUT;

    /** Wire value: 0 normal, 1 whisper, 2 shout. */
    public static TalkMode fromWire(int value) {
        return switch (value) {
            case 1 -> WHISPER;
            case 2 -> SHOUT;
            default -> NORMAL;
        };
    }
}
