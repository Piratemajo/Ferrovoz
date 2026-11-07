package dev.ferrovoz.api.event;

import dev.ferrovoz.api.Subscription;

import java.util.function.Consumer;

/**
 * Suscripción a los eventos de Ferrovoz. Un suscriptor a un tipo recibe también sus
 * subtipos ({@link VoiceEvent} los recibe todos). Si un suscriptor lanza una
 * excepción se apunta en el log y los demás siguen.
 */
public interface EventBus {
    default <E extends VoiceEvent> Subscription subscribe(Class<E> type, Consumer<? super E> listener) {
        return subscribe(type, EventPriority.NORMAL, listener);
    }

    <E extends VoiceEvent> Subscription subscribe(Class<E> type, EventPriority priority, Consumer<? super E> listener);
}
