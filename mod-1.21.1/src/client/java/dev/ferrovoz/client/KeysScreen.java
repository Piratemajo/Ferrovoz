package dev.ferrovoz.client;

import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.ChatFormatting;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.locale.Language;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.network.chat.MutableComponent;
import org.lwjgl.glfw.GLFW;

import java.util.ArrayList;
import java.util.List;

/**
 * Teclas de Ferrovoz desde los ajustes de voz: clic en una acción y después la tecla.
 * Escape la deja sin asignar. Son las mismas que en Opciones > Controles.
 */
final class KeysScreen extends Screen {
    private static final int ROW = 24;
    private static final int KEY_WIDTH = 110;
    private static final int RESET_WIDTH = 50;
    /** Separación entre nombre y botón. */
    private static final int GAP = 8;
    private static final FormattedText ELLIPSIS = FormattedText.of("...");
    private static final int WHITE = 0xFFFFFFFF;
    private static final int GREY = 0xFFAAAAAA;

    private final Screen parent;
    private final List<Row> rows = new ArrayList<>();
    /** La acción que espera su tecla nueva, o null. */
    private KeyMapping selected;
    /** Ancho de las filas y de la columna de nombres (dependen del idioma y la pantalla). */
    private int rowWidth, labelWidth;

    private record Row(KeyMapping key, Button button, Button reset, int y) {}

    KeysScreen(Screen parent) {
        super(Component.translatable("ferrovoz.keys.title"));
        this.parent = parent;
    }

    @Override
    protected void init() {
        rows.clear();
        List<KeyMapping> keys = FerrovozClient.keyMappings();
        // La columna mide lo que el nombre más largo sin salirse de la pantalla; si aun
        // así no cabe, se recorta con "...".
        int widest = 0;
        for (KeyMapping key : keys) {
            widest = Math.max(widest, this.font.width(Component.translatable(key.getName())));
        }
        int buttons = GAP + KEY_WIDTH + 4 + RESET_WIDTH;
        rowWidth = Math.min(widest + buttons, this.width - 20);
        labelWidth = rowWidth - buttons;
        int x = (this.width - rowWidth) / 2;
        int y = Math.max(36, (this.height - (keys.size() + 2) * ROW) / 2);
        for (KeyMapping key : keys) {
            Button button = Button.builder(Component.empty(), b -> {
                selected = key;
                refresh();
            }).bounds(x + rowWidth - KEY_WIDTH - RESET_WIDTH - 4, y, KEY_WIDTH, 20).build();
            Button reset = Button.builder(Component.translatable("controls.reset"), b -> {
                key.setKey(key.getDefaultKey());
                KeyMapping.resetMapping();
                refresh();
            }).bounds(x + rowWidth - RESET_WIDTH, y, RESET_WIDTH, 20).build();
            addRenderableWidget(button);
            addRenderableWidget(reset);
            rows.add(new Row(key, button, reset, y));
            y += ROW;
        }
        addRenderableWidget(Button.builder(Component.translatable("gui.done"), b -> onClose())
                .bounds((this.width - 150) / 2, y + 10, 150, 20).build());
        refresh();
    }

    /** Texto de los botones: tecla actual, "> ... <" si espera una nueva, rojo si hay conflicto. */
    private void refresh() {
        for (Row r : rows) {
            MutableComponent label = r.key().getTranslatedKeyMessage().copy();
            if (r.key() == selected) {
                r.button().setMessage(Component.literal("> ").append(label.withStyle(ChatFormatting.YELLOW)).append(" <"));
            } else {
                r.button().setMessage(conflicts(r.key()) ? label.withStyle(ChatFormatting.RED) : label);
            }
            r.reset().active = !r.key().isDefault();
        }
    }

    /** ¿Otra acción (de Ferrovoz o del juego) usa la misma tecla? */
    private boolean conflicts(KeyMapping key) {
        if (key.isUnbound()) return false;
        for (KeyMapping other : this.minecraft.options.keyMappings) {
            if (other != key && key.same(other)) return true;
        }
        return false;
    }

    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        if (selected != null) {
            // Escape la deja sin tecla; cualquier otra pasa a ser la suya.
            selected.setKey(keyCode == GLFW.GLFW_KEY_ESCAPE ? InputConstants.UNKNOWN : InputConstants.getKey(keyCode, scanCode));
            KeyMapping.resetMapping();
            selected = null;
            refresh();
            return true;
        }
        return super.keyPressed(keyCode, scanCode, modifiers);
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float delta) {
        super.render(g, mouseX, mouseY, delta);
        g.drawString(this.font, this.title, (this.width - this.font.width(this.title)) / 2, 14, WHITE, true);
        Component hint = Component.translatable(selected != null ? "ferrovoz.keys.press" : "ferrovoz.keys.hint");
        g.drawString(this.font, hint, (this.width - this.font.width(hint)) / 2, 24, GREY, false);
        int x = (this.width - rowWidth) / 2;
        for (Row r : rows) {
            Component name = Component.translatable(r.key().getName());
            if (this.font.width(name) <= labelWidth) {
                g.drawString(this.font, name, x, r.y() + 6, WHITE, false);
            } else {
                FormattedText cut = FormattedText.composite(
                        this.font.substrByWidth(name, labelWidth - this.font.width(ELLIPSIS)), ELLIPSIS);
                g.drawString(this.font, Language.getInstance().getVisualOrder(cut), x, r.y() + 6, WHITE, false);
            }
        }
    }

    @Override
    public void onClose() {
        this.minecraft.options.save(); // se guardan con las opciones del juego
        this.minecraft.setScreen(parent);
    }
}
