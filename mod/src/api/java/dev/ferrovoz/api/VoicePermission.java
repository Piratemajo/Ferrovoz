package dev.ferrovoz.api;

/** Permisos de voz que un plugin puede forzar para un jugador. */
public enum VoicePermission {
    /** {@code ferrovoz.speak}: que se le oiga. */
    SPEAK,
    /** {@code ferrovoz.shout}: gritar (si no, el grito sale como voz normal). */
    SHOUT,
    /** {@code ferrovoz.priority}: se le oye siempre, aunque haya multitud o silencio general. */
    PRIORITY
}
