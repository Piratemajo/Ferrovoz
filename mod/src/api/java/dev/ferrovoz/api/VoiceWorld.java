package dev.ferrovoz.api;

/**
 * Un mundo (dimensión) para Ferrovoz. Se obtiene con {@link FerrovozApi#world}. Dos
 * objetos del mismo mundo son {@code equals}.
 */
public interface VoiceWorld {
    /** {@code minecraft:overworld} en los mods; el nombre del mundo en Paper. */
    String name();
}
