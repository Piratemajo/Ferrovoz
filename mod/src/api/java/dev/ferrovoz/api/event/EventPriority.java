package dev.ferrovoz.api.event;

/**
 * Order of subscribers. {@link #EARLY} ones decide first; {@link #LATE} ones see the
 * final outcome (for example, whether an event ended up cancelled).
 */
public enum EventPriority {
    EARLY,
    NORMAL,
    LATE
}
