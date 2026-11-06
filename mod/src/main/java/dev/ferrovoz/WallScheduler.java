package dev.ferrovoz;

import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/**
 * Reparte el cálculo de paredes del servidor entre ticks, con un coste fijo.
 *
 * <p>Cada {@value #REFRESH_TICKS} ticks pide al núcleo las parejas (hablante,
 * oyente) activas, que ya vienen de más cerca a más lejos y limitadas a las 2000
 * más cercanas. Cada tick calcula como mucho {@value #TRACES_PER_TICK}. Si ninguno
 * de los dos se ha movido de bloque desde la última vez, reutiliza el valor sin
 * recalcular. Los resultados de cada tick van al núcleo en una sola llamada. Así
 * el tick cuesta lo mismo con 10 personas que con 250: lo que no da tiempo a
 * calcular se queda sin oclusión del servidor y el cliente usa la suya.
 *
 * <p>No toca Minecraft: cada plataforma da un {@link Locator}. Se usa solo desde el
 * hilo del servidor.
 */
public final class WallScheduler {
    /** Líneas por tick como mucho (cada una, unos 100 bloques mirados). */
    public static final int TRACES_PER_TICK = 300;
    /** Ticks entre peticiones de parejas al núcleo. */
    private static final int REFRESH_TICKS = 4;
    /**
     * Un valor sin moverse nadie se reutiliza hasta este tiempo. Después se recalcula
     * aunque no se hayan movido, por si se abre una puerta o se rompe un bloque.
     */
    private static final int REUSE_TICKS = 8;

    /** Dónde están los ojos de un jugador con voz. */
    public interface Locator {
        /** null si no está o no se sabe. */
        Eye locate(UUID id);
    }

    /** Ojos de un jugador: su mundo, su posición y cómo mirar los bloques de ese mundo. */
    public record Eye(Object world, double x, double y, double z, WallTracer.BlockProbe probe) {}

    private record Pair(long speakerMsb, long speakerLsb, long listenerMsb, long listenerLsb) {}

    private record Known(long speakerBlock, long listenerBlock, float occlusion, int tick) {}

    private record Job(Pair pair, Eye speaker, Eye listener) {}

    private final Map<Pair, Known> known = new HashMap<>();
    private final ArrayDeque<Job> queue = new ArrayDeque<>();
    private int tick;
    // Resultados de este tick, para enviarlos juntos.
    private long[] outPairs = new long[64];
    private float[] outAmounts = new float[16];
    private int outCount;

    /** Cada tick del servidor. */
    public void tick(long server, Locator locator) {
        tick++;
        if (tick % REFRESH_TICKS == 0) {
            refresh(server, locator);
        }
        for (int n = 0; n < TRACES_PER_TICK && !queue.isEmpty(); n++) {
            Job j = queue.poll();
            Eye s = j.speaker(), l = j.listener();
            float occ = WallTracer.occlusion(s.probe(), s.x(), s.y(), s.z(), l.x(), l.y(), l.z());
            if (occ == WallTracer.UNKNOWN) continue;
            known.put(j.pair(), new Known(block(s), block(l), occ, tick));
            add(j.pair(), occ);
        }
        flush(server);
    }

    /** Lo que quedaba en cola se descarta: las parejas nuevas, más cercanas primero. */
    private void refresh(long server, Locator locator) {
        queue.clear();
        long[] pairs = FerrovozNative.serverActivePairs(server);
        if (pairs == null || pairs.length == 0) {
            known.clear();
            return;
        }
        Map<UUID, Eye> eyes = new HashMap<>();
        Set<Pair> active = new HashSet<>();
        for (int i = 0; i + 3 < pairs.length; i += 4) {
            Pair pair = new Pair(pairs[i], pairs[i + 1], pairs[i + 2], pairs[i + 3]);
            active.add(pair);
            Eye s = eye(eyes, locator, new UUID(pairs[i], pairs[i + 1]));
            Eye l = eye(eyes, locator, new UUID(pairs[i + 2], pairs[i + 3]));
            if (s == null || l == null || !s.world().equals(l.world())) continue;
            Known k = known.get(pair);
            if (k != null && k.speakerBlock() == block(s) && k.listenerBlock() == block(l) && tick - k.tick() < REUSE_TICKS) {
                add(pair, k.occlusion()); // el núcleo olvida la oclusión al segundo
                continue;
            }
            queue.add(new Job(pair, s, l)); // ya en orden, de más cerca a más lejos
        }
        known.keySet().retainAll(active);
    }

    private static Eye eye(Map<UUID, Eye> cache, Locator locator, UUID id) {
        if (cache.containsKey(id)) return cache.get(id);
        Eye e = locator.locate(id);
        cache.put(id, e);
        return e;
    }

    private void add(Pair p, float occlusion) {
        if (outCount == outAmounts.length) {
            outAmounts = Arrays.copyOf(outAmounts, outCount * 2);
            outPairs = Arrays.copyOf(outPairs, outCount * 8);
        }
        int i = outCount * 4;
        outPairs[i] = p.speakerMsb();
        outPairs[i + 1] = p.speakerLsb();
        outPairs[i + 2] = p.listenerMsb();
        outPairs[i + 3] = p.listenerLsb();
        outAmounts[outCount++] = occlusion;
    }

    private void flush(long server) {
        if (outCount == 0) return;
        FerrovozNative.serverSetOcclusions(server, Arrays.copyOf(outPairs, outCount * 4), Arrays.copyOf(outAmounts, outCount));
        outCount = 0;
    }

    /** Bloque de los ojos, empaquetado en un long (como BlockPos). */
    static long block(Eye e) {
        long x = (long) Math.floor(e.x()), y = (long) Math.floor(e.y()), z = (long) Math.floor(e.z());
        return ((x & 0x3FFFFFFL) << 38) | ((z & 0x3FFFFFFL) << 12) | (y & 0xFFFL);
    }
}
