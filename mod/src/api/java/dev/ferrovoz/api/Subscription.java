package dev.ferrovoz.api;

/** A subscription (to events or to voice). Cancelling it more than once is fine. */
public interface Subscription extends AutoCloseable {
    void unsubscribe();

    @Override
    default void close() {
        unsubscribe();
    }
}
