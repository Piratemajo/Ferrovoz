package dev.ferrovoz.api.audio;

import dev.ferrovoz.api.Position;

import java.util.Collection;
import java.util.UUID;

/**
 * Opciones de un canal y dónde suena. Los métodos que devuelven {@link AudioChannel}
 * lo crean; hasta 1024 a la vez en todo el servidor.
 *
 * @throws IllegalStateException al crear, si el relé no está en marcha o ya hay 1024
 */
public interface ChannelBuilder {
    /** Bitrate de Opus, de 6000 a 256000 (por defecto 48000). */
    ChannelBuilder bitrate(int bitsPerSecond);

    /** Para música y efectos, no voz (modo de Opus que cuida más lo que no es habla). */
    ChannelBuilder music();

    /** Suena desde un punto; lo oye quien esté a menos de {@code range} bloques, con 3D. */
    AudioChannel at(Position position, double range);

    /** Suena desde donde esté un jugador (él también lo oye), con 3D. */
    AudioChannel following(UUID player, double range);

    /** Sin posición, a estos jugadores, estén donde estén. */
    AudioChannel to(Collection<UUID> listeners);

    /** Sin posición, a quien esté en ese grupo en cada momento. */
    AudioChannel toGroup(String group);
}
