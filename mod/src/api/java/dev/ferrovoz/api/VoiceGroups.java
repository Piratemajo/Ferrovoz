package dev.ferrovoz.api;

import java.util.List;
import java.util.Optional;
import java.util.UUID;

/** Los grupos de voz del servidor. Los nombres no distinguen mayúsculas. */
public interface VoiceGroups {
    Optional<VoiceGroup> get(String name);

    /** Ordenados por nombre. */
    List<VoiceGroup> all();

    /**
     * Crea un grupo temporal con {@code owner} dentro, que ha de tener Ferrovoz. Nombre
     * de hasta 24 caracteres, sin {@code |}; contraseña de hasta 32 o null.
     * No lanza {@code GroupCreateEvent}.
     *
     * @throws VoiceGroupException si el nombre no vale, ya existe o el jugador no está
     */
    VoiceGroup create(String name, String password, UUID owner);
}
