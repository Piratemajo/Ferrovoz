package dev.ferrovoz.api;

/** Voice permissions a plugin can override for a player. */
public enum VoicePermission {
    /** {@code ferrovoz.speak}: can be heard. */
    SPEAK,
    /** {@code ferrovoz.shout}: can shout (otherwise shouting is normal voice). */
    SHOUT,
    /** {@code ferrovoz.priority}: always heard, even in a crowd or while everyone is hushed. */
    PRIORITY
}
