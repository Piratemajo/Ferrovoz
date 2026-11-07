package dev.ferrovoz.api.event;

/**
 * Orden de los suscriptores. Los de {@link #EARLY} deciden primero; los de
 * {@link #LATE} ven la decisión final (por ejemplo, si un evento quedó cancelado).
 */
public enum EventPriority {
    EARLY,
    NORMAL,
    LATE
}
