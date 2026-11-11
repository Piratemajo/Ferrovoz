package dev.ferrovoz;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.PriorityQueue;

/**
 * Propagación del sonido entre bloques. No depende de Minecraft (cada plataforma
 * implementa {@link Blocks}), así se puede probar con mundos falsos.
 *
 * <ul>
 *   <li>{@link #occlusion}: cuánto tapa lo que hay en línea recta.</li>
 *   <li>{@link #findPath}: si la línea recta está tapada, busca un camino libre
 *       (puerta, ventana, esquina) y devuelve la dirección del hueco, la longitud y
 *       las esquinas.</li>
 *   <li>{@link #measure}: acústica del sitio con rayos: reverb, RT60 (Sabine con
 *       tamaño y materiales) y reflexiones tempranas.</li>
 * </ul>
 */
public final class SoundGeometry {
    /** Lo que importa de cada bloque para el sonido. */
    public interface Blocks {
        /**
         * Cuánto tapa: 0 = pasa (aire, plantas, puerta abierta), 1 = macizo, entre
         * medias = puerta cerrada, cristal, vallas, losas.
         */
        float wall(int x, int y, int z);

        /** Absorción de la superficie (0 = piedra o cristal, ~0.7 = lana). */
        float absorption(int x, int y, int z);
    }

    /** Camino libre: dirección del hueco por el que entra la voz, longitud y esquinas. */
    public record Path(double dirX, double dirY, double dirZ, double length, int corners) {}

    /**
     * Acústica de un sitio: wet (0..1), {@code room} (0..1, ver {@link #roomForRt60}),
     * absorción de agudos (0..1) y reflexiones tempranas en grupos de 6
     * {@code [dx, dy, dz, retardo s, ganancia, absorción de la pared]}. La absorción
     * va por reflexión porque cada pared devuelve el sonido con su propio color: la
     * piedra un chasquido brillante, la lana un golpe sordo.
     */
    public record Acoustics(float wet, float room, float damp, float rt60, float[] reflections) {}

    /** Bloques macizos para oclusión total. */
    public static final float BLOCKS_FOR_FULL = 3f;
    /** Velocidad del sonido en bloques por segundo. */
    public static final double SPEED_OF_SOUND = 343.0;
    private static final int RAYS = 42;
    private static final int RAY_LENGTH = 32;
    private static final int MAX_REFLECTIONS = 8;
    private static final int HEURISTIC_WEIGHT = 2;
    /** Separación mínima entre reflexiones tempranas (>= 3 ms). */
    private static final double MIN_DISTANCE_APART = 0.5;
    /** Energía total de las reflexiones respecto al directo (0.25 = -6 dB). */
    private static final double MAX_EARLY_ENERGY = 0.25;
    /**
     * Sabine ajustado a bloques: RT60 = K * r / alfa, con r el radio medio. La fórmula
     * teórica da colas demasiado largas en piedra (los bloques no son lisos y las
     * salas nunca están vacías). Cuarto de piedra de 7x7: ~1.2 s.
     */
    private static final double SABINE_K = 0.034;
    private static final double MIN_ALPHA = 0.1;

    private SoundGeometry() {}

    // --- Oclusión ---

    /** 0 = nada en medio, 1 = {@value #BLOCKS_FOR_FULL} bloques macizos (o equivalente) o más. */
    public static float occlusion(Blocks b, double x0, double y0, double z0, double x1, double y1, double z1) {
        double dx = x1 - x0, dy = y1 - y0, dz = z1 - z0;
        double length = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (length < 1.0) return 0f;
        int steps = (int) Math.ceil(length / 0.4);
        long last = Long.MIN_VALUE;
        float total = 0f;
        for (int i = 1; i < steps; i++) {
            double t = i / (double) steps;
            int x = floor(x0 + dx * t), y = floor(y0 + dy * t), z = floor(z0 + dz * t);
            long key = key(x, y, z);
            if (key == last) continue;
            last = key;
            total += b.wall(x, y, z);
            if (total >= BLOCKS_FOR_FULL) break;
        }
        return Math.min(1f, total / BLOCKS_FOR_FULL);
    }

    /** ¿Hay línea de visión sin nada que tape? */
    public static boolean clear(Blocks b, double x0, double y0, double z0, double x1, double y1, double z1) {
        double dx = x1 - x0, dy = y1 - y0, dz = z1 - z0;
        double length = Math.sqrt(dx * dx + dy * dy + dz * dz);
        int steps = Math.max(1, (int) Math.ceil(length / 0.25));
        long last = Long.MIN_VALUE;
        for (int i = 1; i < steps; i++) {
            double t = i / (double) steps;
            int x = floor(x0 + dx * t), y = floor(y0 + dy * t), z = floor(z0 + dz * t);
            long key = key(x, y, z);
            if (key == last) continue;
            last = key;
            if (b.wall(x, y, z) > 0f) return false;
        }
        return true;
    }

    // --- Caminos alrededor de obstáculos ---

    /**
     * Camino libre más corto del oyente ({@code l}) al hablante ({@code s}), o null si
     * la línea recta ya está libre, si no hay camino o si mide más de {@code maxLength}.
     * Explora como mucho {@code maxNodes} bloques (un par de ms).
     */
    public static Path findPath(Blocks b, double lx, double ly, double lz, double sx, double sy, double sz,
                                int maxNodes, double maxLength) {
        if (clear(b, lx, ly, lz, sx, sy, sz)) return null;
        int[] start = {floor(lx), floor(ly), floor(lz)};
        int[] goal = {floor(sx), floor(sy), floor(sz)};
        if (b.wall(start[0], start[1], start[2]) > 0f || b.wall(goal[0], goal[1], goal[2]) > 0f) return null;

        // A* en rejilla (6 vecinos), heurística Manhattan con peso 2: explora muchos
        // menos bloques a cambio de un camino no siempre óptimo, que luego se endereza
        // con string pulling.
        record Node(int x, int y, int z, int g, int f) {}
        Map<Long, Long> cameFrom = new HashMap<>();
        Map<Long, Integer> best = new HashMap<>();
        PriorityQueue<Node> open = new PriorityQueue<>((a, c) -> a.f != c.f ? Integer.compare(a.f, c.f) : Integer.compare(c.g, a.g));
        long startKey = key(start[0], start[1], start[2]), goalKey = key(goal[0], goal[1], goal[2]);
        best.put(startKey, 0);
        open.add(new Node(start[0], start[1], start[2], 0, HEURISTIC_WEIGHT * manhattan(start, goal)));
        int[][] dirs = {{1, 0, 0}, {-1, 0, 0}, {0, 1, 0}, {0, -1, 0}, {0, 0, 1}, {0, 0, -1}};
        int expanded = 0;
        boolean found = false;
        while (!open.isEmpty() && expanded < maxNodes) {
            Node n = open.poll();
            long k = key(n.x, n.y, n.z);
            if (n.g > best.getOrDefault(k, Integer.MAX_VALUE)) continue;
            if (k == goalKey) {
                found = true;
                break;
            }
            expanded++;
            if (n.g > maxLength) continue;
            for (int[] d : dirs) {
                int x = n.x + d[0], y = n.y + d[1], z = n.z + d[2];
                if (b.wall(x, y, z) > 0f) continue;
                long nk = key(x, y, z);
                int g = n.g + 1;
                if (g < best.getOrDefault(nk, Integer.MAX_VALUE)) {
                    best.put(nk, g);
                    cameFrom.put(nk, k);
                    open.add(new Node(x, y, z, g, g + HEURISTIC_WEIGHT * (Math.abs(x - goal[0]) + Math.abs(y - goal[1]) + Math.abs(z - goal[2]))));
                }
            }
        }
        if (!found) return null;

        // Centros de los bloques del camino, del oyente al hablante.
        List<double[]> cells = new ArrayList<>();
        for (long k = goalKey; ; k = cameFrom.get(k)) {
            cells.add(0, center(k));
            if (k == startKey) break;
        }
        cells.set(0, new double[] {lx, ly, lz});
        cells.set(cells.size() - 1, new double[] {sx, sy, sz});

        // String pulling: desde cada punto se salta al más lejano visible. Quedan solo
        // las esquinas reales y el primer tramo apunta al hueco.
        List<double[]> pulled = new ArrayList<>();
        pulled.add(cells.get(0));
        int i = 0;
        while (i < cells.size() - 1) {
            double[] from = cells.get(i);
            int next = i + 1;
            for (int j = cells.size() - 1; j > i + 1; j--) {
                double[] to = cells.get(j);
                if (clear(b, from[0], from[1], from[2], to[0], to[1], to[2])) {
                    next = j;
                    break;
                }
            }
            pulled.add(cells.get(next));
            i = next;
        }
        double length = 0;
        for (int p = 1; p < pulled.size(); p++) length += dist(pulled.get(p - 1), pulled.get(p));
        if (length > maxLength) return null;
        double[] first = pulled.get(1);
        double dx = first[0] - lx, dy = first[1] - ly, dz = first[2] - lz;
        double norm = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (norm < 1e-6) return null;
        return new Path(dx / norm, dy / norm, dz / norm, length, Math.max(1, pulled.size() - 2));
    }

    // --- Acústica ---

    /** RT60 (s) para un {@code room} 0..1 (igual que en Rust). */
    public static float rt60ForRoom(float room) {
        return 0.25f + 3.25f * Math.max(0f, Math.min(1f, room));
    }

    public static float roomForRt60(float rt60) {
        return Math.max(0f, Math.min(1f, (rt60 - 0.25f) / 3.25f));
    }

    /** Lanza {@value #RAYS} rayos por la esfera y mide la sala. */
    public static Acoustics measure(Blocks b, double x, double y, double z) {
        int hits = 0, upRays = 0, upHits = 0;
        double sumDistance = 0, sumAlpha = 0, sumHitAlpha = 0;
        double[][] dirs = sphere(RAYS);
        double[] hitDist = new double[RAYS];
        float[] hitAlpha = new float[RAYS];
        for (int r = 0; r < RAYS; r++) {
            double[] d = dirs[r];
            hitDist[r] = -1;
            for (double t = 0.5; t <= RAY_LENGTH; t += 0.5) {
                int bx = floor(x + d[0] * t), by = floor(y + d[1] * t), bz = floor(z + d[2] * t);
                if (b.wall(bx, by, bz) >= 0.5f) {
                    hitDist[r] = t;
                    hitAlpha[r] = Math.max(0f, Math.min(1f, b.absorption(bx, by, bz)));
                    break;
                }
            }
            // Rayos hacia arriba: ¿hay techo? (las hojas no cuentan, el cielo se ve entre ellas)
            if (d[1] > 0.7) {
                upRays++;
                if (hitDist[r] > 0) upHits++;
            }
            if (hitDist[r] > 0) {
                hits++;
                sumDistance += hitDist[r];
                sumAlpha += hitAlpha[r];
                sumHitAlpha += hitAlpha[r];
            } else {
                sumAlpha += 1.0; // aire libre: no vuelve nada
            }
        }
        if (hits == 0) return new Acoustics(0f, 0f, 0.3f, 0.25f, new float[0]);
        double enclosed = hits / (double) RAYS;
        double meanDistance = sumDistance / hits;
        double alpha = Math.max(MIN_ALPHA, sumAlpha / RAYS);
        // Sabine para una sala esférica de radio medio r: RT60 ~ V/(S*alfa) ~ r/alfa.
        float rt60 = (float) Math.max(0.25, Math.min(3.5, SABINE_K * Math.max(1.0, meanDistance) / alpha));
        // Solo hay reverb si está bastante cerrado: el suelo ya es la mitad de los rayos,
        // así que al aire libre o junto a una pared no hay.
        double t = Math.max(0, Math.min(1, (enclosed - 0.55) / 0.4));
        // En huecos muy pequeños (3x3) la reverb se funde con la voz: menos.
        double size = Math.max(0.3, Math.min(1.0, meanDistance / 5.0));
        // Sin techo no hay reverb (un bosque rodea mucho, pero el sonido se escapa por arriba).
        double roof = upRays == 0 ? 0 : Math.max(0, Math.min(1, (upHits / (double) upRays - 0.5) / 0.4));
        roof = roof * roof * (3 - 2 * roof);
        float wet = (float) (t * t * (3 - 2 * t) * 0.45 * size * roof);
        float damp = (float) Math.max(0.1, Math.min(0.85, 0.15 + 1.2 * sumHitAlpha / hits));

        // Reflexiones tempranas: las paredes que más devuelven, en direcciones distintas.
        Integer[] order = new Integer[RAYS];
        double[] gain = new double[RAYS];
        for (int r = 0; r < RAYS; r++) {
            order[r] = r;
            gain[r] = hitDist[r] >= 1.0 ? (1 - hitAlpha[r]) / (1 + hitDist[r] / 4.0) : 0;
        }
        java.util.Arrays.sort(order, (p, q) -> Double.compare(gain[q], gain[p]));
        List<Integer> chosen = new ArrayList<>();
        for (int r : order) {
            if (gain[r] <= 0.02 || chosen.size() >= MAX_REFLECTIONS) break;
            boolean apart = true;
            for (int c : chosen) {
                double[] a = dirs[r], o = dirs[c];
                // Ni misma dirección ni casi el mismo retardo: paredes a la misma distancia
                // sumarían en el mismo instante y sonaría metálico.
                if (a[0] * o[0] + a[1] * o[1] + a[2] * o[2] > Math.cos(Math.toRadians(40))
                        || Math.abs(hitDist[r] - hitDist[c]) < MIN_DISTANCE_APART) {
                    apart = false;
                    break;
                }
            }
            if (apart) chosen.add(r);
        }
        // Total como mucho -6 dB respecto al directo.
        double energy = 0;
        for (int r : chosen) energy += gain[r] * gain[r] * (0.3 + 0.7 * enclosed) * (0.3 + 0.7 * enclosed);
        double norm = energy > MAX_EARLY_ENERGY ? Math.sqrt(MAX_EARLY_ENERGY / energy) : 1.0;
        float[] refl = new float[chosen.size() * 6];
        for (int c = 0; c < chosen.size(); c++) {
            int r = chosen.get(c);
            refl[6 * c] = (float) dirs[r][0];
            refl[6 * c + 1] = (float) dirs[r][1];
            refl[6 * c + 2] = (float) dirs[r][2];
            refl[6 * c + 3] = (float) (2 * hitDist[r] / SPEED_OF_SOUND);
            refl[6 * c + 4] = (float) (gain[r] * (0.3 + 0.7 * enclosed) * norm);
            refl[6 * c + 5] = hitAlpha[r];
        }
        return new Acoustics(wet, roomForRt60(rt60), damp, rt60, refl);
    }

    /** {@code n} direcciones uniformes en la esfera (espiral de Fibonacci). */
    static double[][] sphere(int n) {
        double[][] out = new double[n][];
        double golden = Math.PI * (3 - Math.sqrt(5));
        for (int i = 0; i < n; i++) {
            double y = 1 - 2 * (i + 0.5) / n;
            double r = Math.sqrt(1 - y * y);
            double a = golden * i;
            out[i] = new double[] {Math.cos(a) * r, y, Math.sin(a) * r};
        }
        return out;
    }

    // --- Utilidades ---

    private static int floor(double v) {
        return (int) Math.floor(v);
    }

    private static int manhattan(int[] a, int[] c) {
        return Math.abs(a[0] - c[0]) + Math.abs(a[1] - c[1]) + Math.abs(a[2] - c[2]);
    }

    private static double dist(double[] a, double[] c) {
        double dx = a[0] - c[0], dy = a[1] - c[1], dz = a[2] - c[2];
        return Math.sqrt(dx * dx + dy * dy + dz * dz);
    }

    /** Clave de 64 bits de un bloque (26 bits x, 12 y, 26 z). */
    static long key(int x, int y, int z) {
        return ((long) (x & 0x3FFFFFF) << 38) | ((long) (y & 0xFFF) << 26) | (z & 0x3FFFFFF);
    }

    private static double[] center(long k) {
        int x = (int) (k >> 38), y = (int) ((k >> 26) & 0xFFF), z = (int) (k & 0x3FFFFFF);
        // Extensión de signo.
        x = (x << 6) >> 6;
        y = (y << 20) >> 20;
        z = (z << 6) >> 6;
        return new double[] {x + 0.5, y + 0.5, z + 0.5};
    }
}
