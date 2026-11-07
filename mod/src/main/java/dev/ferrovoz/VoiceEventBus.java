package dev.ferrovoz;

import dev.ferrovoz.api.Subscription;
import dev.ferrovoz.api.event.EventBus;
import dev.ferrovoz.api.event.EventPriority;
import dev.ferrovoz.api.event.VoiceEvent;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;

/**
 * Bus de eventos de la API. La lista de suscriptores se copia al cambiar (pasa poco) y
 * se lee sin bloquear al lanzar (pasa cada tick), así que se puede suscribir y
 * cancelar desde cualquier hilo, también desde dentro de un suscriptor.
 */
final class VoiceEventBus implements EventBus {
    private record Entry(Class<? extends VoiceEvent> type, EventPriority priority, long order, Consumer<Object> listener) {}

    private static final Comparator<Entry> ORDER =
            Comparator.comparing(Entry::priority).thenComparingLong(Entry::order);

    private final AtomicLong next = new AtomicLong();
    private volatile List<Entry> entries = List.of();

    @Override
    @SuppressWarnings("unchecked")
    public <E extends VoiceEvent> Subscription subscribe(Class<E> type, EventPriority priority, Consumer<? super E> listener) {
        Objects.requireNonNull(type, "type");
        Objects.requireNonNull(priority, "priority");
        Objects.requireNonNull(listener, "listener");
        Entry entry = new Entry(type, priority, next.getAndIncrement(), e -> ((Consumer<Object>) listener).accept(e));
        synchronized (this) {
            List<Entry> copy = new ArrayList<>(entries);
            copy.add(entry);
            copy.sort(ORDER);
            entries = List.copyOf(copy);
        }
        return () -> {
            synchronized (VoiceEventBus.this) {
                List<Entry> copy = new ArrayList<>(entries);
                if (copy.remove(entry)) entries = List.copyOf(copy);
            }
        };
    }

    /** ¿Alguien escucha este tipo? Para no montar eventos que nadie va a ver. */
    boolean wants(Class<? extends VoiceEvent> type) {
        for (Entry e : entries) {
            if (e.type().isAssignableFrom(type)) return true;
        }
        return false;
    }

    /** Lanza el evento a quien escuche su tipo, en orden. Devuelve el mismo evento. */
    <E extends VoiceEvent> E post(E event) {
        for (Entry e : entries) {
            if (!e.type().isInstance(event)) continue;
            try {
                e.listener().accept(event);
            } catch (RuntimeException | LinkageError ex) {
                FerrovozLog.LOG.warn("A Ferrovoz event listener failed on {}", event.getClass().getSimpleName(), ex);
            }
        }
        return event;
    }
}
