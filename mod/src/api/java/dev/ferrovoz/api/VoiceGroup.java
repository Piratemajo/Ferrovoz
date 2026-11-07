package dev.ferrovoz.api;

import java.util.Optional;
import java.util.Set;
import java.util.UUID;

/**
 * A voice group: its members hear each other at any distance and in any world. It is a
 * view by name; once the group is deleted, {@link #exists()} becomes false and changes
 * throw {@link VoiceGroupException}.
 *
 * <p>Temporary groups are deleted when they become empty. Persistent ones are saved to
 * disk and remember their members.
 */
public interface VoiceGroup {
    String name();

    boolean exists();

    Optional<UUID> owner();

    /** Members who are online now. */
    Set<UUID> members();

    boolean hasPassword();

    boolean isPersistent();

    void setPersistent(boolean persistent);

    /** Adds a player without asking for the password. If they were in another group, they leave it. */
    void add(UUID player);

    /** Removes a player. false if they weren't in it. */
    boolean remove(UUID player);

    /** Deletes it even if it is persistent; its members are left without a group. */
    void delete();
}
