package dev.ferrovoz.api;

/** Una suscripción (a eventos o a la voz). Se puede cancelar varias veces sin problema. */
public interface Subscription extends AutoCloseable {
    void unsubscribe();

    @Override
    default void close() {
        unsubscribe();
    }
}
