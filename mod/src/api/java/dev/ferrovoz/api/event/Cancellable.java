package dev.ferrovoz.api.event;

/**
 * Un evento que se lanza antes de hacer algo y se puede impedir. Solo se lanzan cuando
 * lo pide un jugador (con {@code /voz}); lo que hace un plugin por la API no los lanza,
 * para que dos plugins no se pisen ni entren en bucle.
 *
 * <p>Si se cancela, Ferrovoz no responde nada al jugador: el plugin que lo cancela le
 * dice por qué.
 */
public interface Cancellable extends VoiceEvent {
    boolean isCancelled();

    void setCancelled(boolean cancelled);
}
