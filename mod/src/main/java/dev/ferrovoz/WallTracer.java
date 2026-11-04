package dev.ferrovoz;

/**
 * Cuenta los bloques sólidos entre dos puntos con el mundo del servidor, para la
 * oclusión que el cliente no puede calcular (hablante fuera de su render distance).
 *
 * <p>Nunca carga chunks: si el segmento pasa por uno sin cargar devuelve
 * {@link #UNKNOWN}. Cargarlos desde aquí congelaría el servidor.
 *
 * Sin dependencias de Minecraft: cada plataforma implementa {@link BlockProbe}.
 */
public final class WallTracer {
    /** Bloques sólidos para oclusión total. */
    public static final int BLOCKS_FOR_FULL = 3;
    public static final float UNKNOWN = -1f;
    private static final double STEP = 0.5;

    public interface BlockProbe {
        boolean isChunkLoaded(int chunkX, int chunkZ);

        boolean isSolid(int x, int y, int z);
    }

    private WallTracer() {}

    /** 0 = nada en medio, 1 = {@value #BLOCKS_FOR_FULL} bloques sólidos o más, {@link #UNKNOWN} = chunk sin cargar. */
    public static float occlusion(BlockProbe probe, double x0, double y0, double z0, double x1, double y1, double z1) {
        double dx = x1 - x0, dy = y1 - y0, dz = z1 - z0;
        double length = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (length < 1.0) return 0f;
        int steps = (int) Math.ceil(length / STEP);
        int lastX = Integer.MIN_VALUE, lastY = Integer.MIN_VALUE, lastZ = Integer.MIN_VALUE;
        int lastChunkX = Integer.MIN_VALUE, lastChunkZ = Integer.MIN_VALUE;
        int blocks = 0;
        for (int i = 1; i < steps; i++) {
            double t = i / (double) steps;
            int bx = (int) Math.floor(x0 + dx * t), by = (int) Math.floor(y0 + dy * t), bz = (int) Math.floor(z0 + dz * t);
            if (bx == lastX && by == lastY && bz == lastZ) continue;
            lastX = bx;
            lastY = by;
            lastZ = bz;
            int cx = bx >> 4, cz = bz >> 4;
            if (cx != lastChunkX || cz != lastChunkZ) {
                if (!probe.isChunkLoaded(cx, cz)) return UNKNOWN;
                lastChunkX = cx;
                lastChunkZ = cz;
            }
            if (probe.isSolid(bx, by, bz) && ++blocks >= BLOCKS_FOR_FULL) break;
        }
        return Math.min(1f, blocks / (float) BLOCKS_FOR_FULL);
    }
}
