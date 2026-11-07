package dev.ferrovoz.api;

import java.util.Objects;

/** Un punto en un mundo. */
public record Position(VoiceWorld world, double x, double y, double z) {
    public Position {
        Objects.requireNonNull(world, "world");
        if (!Double.isFinite(x) || !Double.isFinite(y) || !Double.isFinite(z)) {
            throw new IllegalArgumentException("coordenadas no finitas: " + x + ", " + y + ", " + z);
        }
    }
}
