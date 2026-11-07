package dev.ferrovoz.api.event;

import dev.ferrovoz.api.Subscription;

import java.util.function.Consumer;

/**
 * Subscriptions to Ferrovoz events. A subscriber to a type also receives its subtypes
 * ({@link VoiceEvent} receives everything). If a subscriber throws, the exception is
 * logged and the rest still run.
 */
public interface EventBus {
    default <E extends VoiceEvent> Subscription subscribe(Class<E> type, Consumer<? super E> listener) {
        return subscribe(type, EventPriority.NORMAL, listener);
    }

    <E extends VoiceEvent> Subscription subscribe(Class<E> type, EventPriority priority, Consumer<? super E> listener);
}
