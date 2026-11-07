package dev.ferrovoz.api;

/**
 * A world (dimension) as Ferrovoz sees it. Get one with {@link FerrovozApi#world}. Two
 * objects for the same world are {@code equals}.
 */
public interface VoiceWorld {
    /** {@code minecraft:overworld} on mod loaders; the world name on Paper. */
    String name();
}
