package dev.ferrovoz.api;

/**
 * Ajustes del relé en marcha. Empiezan con los de {@code ferrovoz.properties}; lo que
 * se cambie aquí dura hasta que se reinicie el servidor.
 */
public interface VoiceSettings {
    /** Alcance de un modo, en bloques. */
    double range(TalkMode mode);

    /** Entre 1 y 30000 bloques. */
    void setRange(TalkMode mode, double blocks);

    /** Voces por separado como mucho en cada zona; el resto va al murmullo. 0 = sin límite. */
    int maxVoices();

    void setMaxVoices(int maxVoices);

    /** Eco en cuevas y salas. */
    boolean caveEcho();

    void setCaveEcho(boolean enabled);

    /**
     * Silencio general: solo se oye a quien tenga {@link VoicePermission#PRIORITY}. Para
     * discursos y presentaciones. A los demás se les ve con el micro silenciado.
     */
    boolean isHushed();

    void setHushed(boolean hushed);
}
