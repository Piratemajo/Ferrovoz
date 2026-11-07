package dev.ferrovoz.api;

import java.util.Objects;

/** A point in a world. */
public record Position(VoiceWorld world, double x, double y, double z) {
    public Position {
        Objects.requireNonNull(world, "world");
        if (!Double.isFinite(x) || !Double.isFinite(y) || !Double.isFinite(z)) {
            throw new IllegalArgumentException("coordinates must be finite: " + x + ", " + y + ", " + z);
        }
    }
}
