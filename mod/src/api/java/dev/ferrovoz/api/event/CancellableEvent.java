package dev.ferrovoz.api.event;

/** Base de los eventos que se pueden cancelar. */
public abstract class CancellableEvent implements Cancellable {
    private volatile boolean cancelled;

    @Override
    public boolean isCancelled() {
        return cancelled;
    }

    @Override
    public void setCancelled(boolean cancelled) {
        this.cancelled = cancelled;
    }
}
