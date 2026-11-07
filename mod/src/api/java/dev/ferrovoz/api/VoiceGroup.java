package dev.ferrovoz.api;

import java.util.Optional;
import java.util.Set;
import java.util.UUID;

/**
 * Un grupo de voz: sus miembros se oyen a cualquier distancia y en cualquier mundo.
 * Es una vista por nombre; si el grupo se borra, {@link #exists()} pasa a false y los
 * cambios lanzan {@link VoiceGroupException}.
 *
 * <p>Los grupos temporales se borran al quedarse vacíos. Los permanentes se guardan en
 * disco y recuerdan a sus miembros.
 */
public interface VoiceGroup {
    String name();

    boolean exists();

    Optional<UUID> owner();

    /** Miembros conectados ahora. */
    Set<UUID> members();

    boolean hasPassword();

    boolean isPersistent();

    void setPersistent(boolean persistent);

    /** Mete a un jugador sin pedir contraseña. Si estaba en otro grupo, sale de él. */
    void add(UUID player);

    /** Saca a un jugador. false si no estaba. */
    boolean remove(UUID player);

    /** Lo borra aunque sea permanente; sus miembros se quedan sin grupo. */
    void delete();
}
