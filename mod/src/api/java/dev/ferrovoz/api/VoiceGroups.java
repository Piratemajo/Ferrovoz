package dev.ferrovoz.api;

import java.util.List;
import java.util.Optional;
import java.util.UUID;

/** The server's voice groups. Names are case-insensitive. */
public interface VoiceGroups {
    Optional<VoiceGroup> get(String name);

    /** Sorted by name. */
    List<VoiceGroup> all();

    /**
     * Creates a temporary group with {@code owner} in it, who must have Ferrovoz. The name
     * is up to 24 characters with no {@code |} or {@code §}; the password up to 32, or null.
     * Does not fire {@code GroupCreateEvent}.
     *
     * @throws VoiceGroupException if the name is invalid or taken, or the player isn't there
     */
    VoiceGroup create(String name, String password, UUID owner);
}
