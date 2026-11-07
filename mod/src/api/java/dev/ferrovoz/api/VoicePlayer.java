package dev.ferrovoz.api;

import java.util.Optional;
import java.util.OptionalDouble;
import java.util.UUID;

/**
 * Un jugador con Ferrovoz. Es una vista: cada llamada consulta el estado actual, así
 * que se puede guardar mientras el jugador esté conectado.
 *
 * <p>Lo que fija un plugin ({@link #setRange}, {@link #setPermission}) dura mientras el
 * servidor esté encendido, también si el jugador sale y vuelve. No se guarda en disco.
 */
public interface VoicePlayer {
    UUID id();

    /** Nombre en el juego (o el UUID si ya no está). */
    String name();

    /** Su voz ya va y viene: el cliente ha contactado con el relé. */
    boolean isConnected();

    /** Se le está oyendo ahora mismo. */
    boolean isSpeaking();

    /** Ha silenciado su micro (tecla M). */
    boolean isMicrophoneMuted();

    /** Se ha ensordecido (no oye a nadie). */
    boolean isDeafened();

    /** Silenciado por moderación. Se guarda en disco, como {@code /voz silenciar}. */
    boolean isMuted();

    /** Silencia o quita el silencio de moderación. No lanza {@code PlayerMuteEvent}. */
    void setMuted(boolean muted);

    /** Su grupo, si está en uno. */
    Optional<VoiceGroup> group();

    /** Pérdida de paquetes de su conexión, en %. 0 si no se sabe. */
    int packetLoss();

    /**
     * Alcance fijo en bloques en vez del de susurro, normal o grito: un megáfono, un
     * escenario. Entre 1 y 30000.
     */
    void setRange(double blocks);

    /** Vuelve al alcance de cada modo. */
    void resetRange();

    /** El alcance fijado con {@link #setRange}, si hay uno. */
    OptionalDouble range();

    /** El valor que ha fijado un plugin (o {@link PermissionOverride#INHERIT}). */
    PermissionOverride permissionOverride(VoicePermission permission);

    /** Fuerza un permiso, o lo devuelve al sistema de permisos con {@code INHERIT}. Se aplica al momento. */
    void setPermission(VoicePermission permission, PermissionOverride value);

    /** El permiso que tiene ahora, contando lo que haya forzado un plugin. */
    boolean hasPermission(VoicePermission permission);
}
