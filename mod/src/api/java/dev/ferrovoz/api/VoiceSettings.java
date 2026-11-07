package dev.ferrovoz.api;

/**
 * Settings of the running relay. They start from {@code ferrovoz.properties}; changes
 * made here last until the server restarts.
 */
public interface VoiceSettings {
    /** Range of a talk mode, in blocks. */
    double range(TalkMode mode);

    /** Between 1 and 30000 blocks. */
    void setRange(TalkMode mode, double blocks);

    /** Most separate voices per zone; the rest goes into the crowd murmur. 0 = no limit. */
    int maxVoices();

    void setMaxVoices(int maxVoices);

    /** Echo in caves and rooms. */
    boolean caveEcho();

    void setCaveEcho(boolean enabled);

    /**
     * Hush: only players with {@link VoicePermission#PRIORITY} can be heard. For speeches
     * and shows. Everyone else shows up with a muted microphone.
     */
    boolean isHushed();

    void setHushed(boolean hushed);
}
