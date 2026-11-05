package dev.ferrovoz.client;

import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.Component;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.UUID;

/** Volumen por jugador conectado (o silenciarlo solo para uno mismo). Se guarda. */
final class PlayerVolumeScreen extends Screen {
    private static final int ROWS_PER_PAGE = 7;
    private static final int ROW = 24;
    private static final int WIDTH = 300;
    private static final int WHITE = 0xFFFFFFFF;
    private static final int GREY = 0xFFAAAAAA;
    private static final int GREEN = 0xFF55FF55;

    private final Screen parent;
    private final ClientConfig config = FerrovozClient.config();
    private final List<Row> rows = new ArrayList<>();
    private int page;

    private record Row(UUID id, String name, int y) {}

    PlayerVolumeScreen(Screen parent) {
        super(Component.translatable("ferrovoz.players.title"));
        this.parent = parent;
    }

    private List<PlayerInfo> players() {
        List<PlayerInfo> list = new ArrayList<>();
        if (this.minecraft.getConnection() == null || this.minecraft.player == null) return list;
        UUID me = this.minecraft.player.getUUID();
        for (PlayerInfo info : this.minecraft.getConnection().getOnlinePlayers()) {
            if (!info.getProfile().getId().equals(me)) list.add(info);
        }
        list.sort(Comparator.comparing(i -> i.getProfile().getName().toLowerCase()));
        return list;
    }

    @Override
    protected void init() {
        rows.clear();
        List<PlayerInfo> all = players();
        int pages = Math.max(1, (all.size() + ROWS_PER_PAGE - 1) / ROWS_PER_PAGE);
        page = Math.min(page, pages - 1);
        int x = (this.width - WIDTH) / 2;
        int y = 40;

        int from = page * ROWS_PER_PAGE;
        for (int i = from; i < Math.min(all.size(), from + ROWS_PER_PAGE); i++) {
            PlayerInfo info = all.get(i);
            UUID id = info.getProfile().getId();
            rows.add(new Row(id, info.getProfile().getName(), y));

            // [-] [ 100% ] [+] [Silenciar]
            int bx = x + 120;
            Button value = Button.builder(Component.literal(percent(config.volumeOf(id))), b -> set(id, 1f, b))
                    .bounds(bx + 22, y, 50, 20).build();
            addRenderableWidget(Button.builder(Component.literal("−"), b -> set(id, config.volumeOf(id) - 0.1f, value))
                    .bounds(bx, y, 20, 20).build());
            addRenderableWidget(value);
            addRenderableWidget(Button.builder(Component.literal("+"), b -> set(id, config.volumeOf(id) + 0.1f, value))
                    .bounds(bx + 74, y, 20, 20).build());
            addRenderableWidget(Button.builder(muteLabel(id), b -> {
                set(id, config.volumeOf(id) <= 0f ? 1f : 0f, value);
                b.setMessage(muteLabel(id));
            }).bounds(bx + 98, y, 82, 20).build());
            y += ROW;
        }

        int footer = 40 + ROWS_PER_PAGE * ROW + 8;
        if (pages > 1) {
            Button prev = Button.builder(Component.literal("◀"), b -> {
                page--;
                rebuild();
            }).bounds(x, footer, 20, 20).build();
            Button next = Button.builder(Component.literal("▶"), b -> {
                page++;
                rebuild();
            }).bounds(x + WIDTH - 20, footer, 20, 20).build();
            prev.active = page > 0;
            next.active = page < pages - 1;
            addRenderableWidget(prev);
            addRenderableWidget(next);
        }
        addRenderableWidget(Button.builder(Component.translatable("gui.done"), b -> onClose())
                .bounds((this.width - 150) / 2, footer, 150, 20).build());
    }

    private void rebuild() {
        clearWidgets();
        init();
    }

    private void set(UUID id, float volume, Button valueButton) {
        float v = Math.max(0f, Math.min(2f, Math.round(volume * 10f) / 10f));
        config.setVolume(id, v);
        long h = FerrovozClient.handle();
        if (h != 0) {
            FerrovozNative.clientSetPlayerVolume(h, id.getMostSignificantBits(), id.getLeastSignificantBits(), v);
        }
        valueButton.setMessage(Component.literal(percent(v)));
    }

    private Component muteLabel(UUID id) {
        return Component.translatable(config.volumeOf(id) <= 0f ? "ferrovoz.players.unmute" : "ferrovoz.players.mute");
    }

    private static String percent(float v) {
        return Math.round(v * 100f) + "%";
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float delta) {
        super.render(g, mouseX, mouseY, delta);
        g.drawString(this.font, this.title, (this.width - this.font.width(this.title)) / 2, 14, WHITE, true);
        int x = (this.width - WIDTH) / 2;
        if (rows.isEmpty()) {
            Component empty = Component.translatable("ferrovoz.players.empty");
            g.drawString(this.font, empty, (this.width - this.font.width(empty)) / 2, 60, GREY, false);
            return;
        }
        long[] speaking = FerrovozClient.speaking();
        for (Row r : rows) {
            boolean talking = false;
            for (int i = 0; speaking != null && i + 2 < speaking.length; i += 3) {
                if (speaking[i] == r.id.getMostSignificantBits() && speaking[i + 1] == r.id.getLeastSignificantBits()) {
                    talking = true;
                    break;
                }
            }
            String label = (talking ? "♪ " : "") + r.name;
            g.drawString(this.font, label, x, r.y + 6, talking ? GREEN : (config.volumeOf(r.id) <= 0f ? GREY : WHITE), false);
        }
    }

    @Override
    public void onClose() {
        config.save();
        this.minecraft.setScreen(parent);
    }
}
