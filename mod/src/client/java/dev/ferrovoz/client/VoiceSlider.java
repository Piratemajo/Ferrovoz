package dev.ferrovoz.client;

import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.network.chat.Component;

import java.util.function.Consumer;
import java.util.function.Function;

/**
 * Deslizador de Minecraft entre {@code min} y {@code max}. El valor interno de
 * {@link AbstractSliderButton} va de 0 a 1; aquí lo traducimos al rango real,
 * redondeado a {@code step}.
 */
final class VoiceSlider extends AbstractSliderButton {
    private final float min;
    private final float max;
    private final float step;
    private final Function<Float, Component> label;
    private final Consumer<Float> onChange;

    VoiceSlider(int x, int y, int width, float min, float max, float step, float initial,
                Function<Float, Component> label, Consumer<Float> onChange) {
        super(x, y, width, 20, label.apply(initial), toSlider(initial, min, max));
        this.min = min;
        this.max = max;
        this.step = step;
        this.label = label;
        this.onChange = onChange;
    }

    private static double toSlider(float v, float min, float max) {
        return Math.max(0.0, Math.min(1.0, (v - min) / (double) (max - min)));
    }

    float current() {
        float v = (float) (min + this.value * (max - min));
        return Math.max(min, Math.min(max, Math.round(v / step) * step));
    }

    @Override
    public void updateMessage() {
        this.setMessage(label.apply(current()));
    }

    @Override
    public void applyValue() {
        onChange.accept(current());
    }
}
