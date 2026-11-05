package dev.ferrovoz;

import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.LongSupplier;

/**
 * Hilo que envía la voz pendiente de los jugadores por túnel cada
 * {@value #INTERVAL_MS} ms, sin esperar al tick (50 ms). Enviar desde otro hilo es
 * seguro: todo acaba en el canal de Netty. Común al mod y al plugin; cada uno
 * implementa {@link Sink}.
 */
public final class TunnelPump implements AutoCloseable {
    public static final int INTERVAL_MS = 5;

    /** Envío de un paquete a un jugador por la conexión del juego. */
    public interface Sink {
        void send(UUID player, byte[] packet);
    }

    private final Set<UUID> players = ConcurrentHashMap.newKeySet();
    private final LongSupplier server;
    private final Sink sink;
    private final Thread thread;
    private volatile boolean running = true;

    public TunnelPump(LongSupplier server, Sink sink) {
        this.server = server;
        this.sink = sink;
        this.thread = new Thread(this::run, "ferrovoz-tunnel");
        this.thread.setDaemon(true);
        this.thread.start();
    }

    public void add(UUID player) {
        players.add(player);
    }

    public void remove(UUID player) {
        players.remove(player);
    }

    private void run() {
        while (running) {
            long srv = server.getAsLong();
            if (srv != 0) {
                for (UUID id : players) {
                    byte[][] pending = FerrovozNative.serverPollTunnel(srv, id.getMostSignificantBits(), id.getLeastSignificantBits());
                    if (pending == null) continue;
                    for (byte[] packet : pending) {
                        try {
                            sink.send(id, packet);
                        } catch (RuntimeException e) {
                            // El jugador está saliendo: se pierde un paquete, sin más.
                        }
                    }
                }
            }
            try {
                Thread.sleep(INTERVAL_MS);
            } catch (InterruptedException e) {
                return;
            }
        }
    }

    @Override
    public void close() {
        running = false;
        thread.interrupt();
        players.clear();
        // Que no siga enviando con el relé ya parado.
        if (Thread.currentThread() != thread) {
            try {
                thread.join(1000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
    }
}
