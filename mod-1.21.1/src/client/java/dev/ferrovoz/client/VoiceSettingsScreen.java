package dev.ferrovoz.client;

import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * Ajustes de voz: dispositivos, volúmenes, modo, umbral con vúmetro, denoiser y
 * prueba de micro. Sin conexión al relé abre un cliente de prueba sin red.
 */
public final class VoiceSettingsScreen extends Screen {
    private static final int ROW = 24;
    private static final int WIDTH = 300;
    private static final int GREEN = 0xFF55FF55;
    private static final int YELLOW = 0xFFFFDD55;
    private static final int RED = 0xFFFF5555;
    private static final int GREY = 0xFFAAAAAA;
    private static final int DARK = 0xFF202020;
    private static final int WHITE = 0xFFFFFFFF;

    private final Screen parent;
    private final ClientConfig config;
    /** Cliente de prueba (solo sin conexión real). 0 = ninguno. */
    private long preview;
    private boolean monitor;

    private List<String> inputs = List.of();
    private List<String> outputs = List.of();

    private int meterX, meterY, meterW;

    public VoiceSettingsScreen(Screen parent) {
        super(Component.translatable("ferrovoz.settings.title"));
        this.parent = parent;
        this.config = FerrovozClient.config();
    }

    @Override
    protected void init() {
        // Si vuelve la conexión real, el de prueba sobra (no dos clientes con el micro).
        if (preview != 0 && FerrovozClient.handle() != 0) {
            FerrovozNative.clientStop(preview);
            preview = 0;
        }
        if (preview == 0 && FerrovozClient.handle() == 0) {
            preview = FerrovozNative.previewStart();
            config.applyTo(preview);
        }
        inputs = withDefault(FerrovozNative.listInputDevices());
        outputs = withDefault(FerrovozNative.listOutputDevices());

        int x = (this.width - WIDTH) / 2;
        int y = Math.max(34, (this.height - 268) / 2); // alto total de los controles

        // Dispositivos
        addRenderableWidget(cycle(x, y, WIDTH, () -> Component.translatable("ferrovoz.settings.input", deviceLabel(config.inputDevice)),
                b -> {
                    config.inputDevice = next(inputs, config.inputDevice);
                    FerrovozNative.clientSetDevices(h(), config.inputDevice, config.outputDevice);
                }));
        y += ROW;
        addRenderableWidget(cycle(x, y, WIDTH, () -> Component.translatable("ferrovoz.settings.output", deviceLabel(config.outputDevice)),
                b -> {
                    config.outputDevice = next(outputs, config.outputDevice);
                    FerrovozNative.clientSetDevices(h(), config.inputDevice, config.outputDevice);
                }));
        y += ROW;

        // Volúmenes, de dos en dos para que quepa en pantallas bajas
        int half = (WIDTH - 4) / 2;
        stepper(x, y, half, "ferrovoz.settings.volume", () -> config.volume, v -> {
            config.volume = v;
            FerrovozNative.clientSetVolume(h(), v);
        }, 0f, 2f, 0.1f, 1f, VoiceSettingsScreen::percent);
        // Volumen de grupo
        stepper(x + half + 4, y, half, "ferrovoz.settings.group_volume", () -> config.groupVolume, v -> {
            config.groupVolume = v;
            FerrovozNative.clientSetGroupVolume(h(), v);
        }, 0f, 2f, 0.1f, 1f, VoiceSettingsScreen::percent);
        y += ROW;

        // Ganancia y umbral
        stepper(x, y, half, "ferrovoz.settings.mic_gain", () -> config.micGain, v -> {
            config.micGain = v;
            FerrovozNative.clientSetMicGain(h(), v);
        }, 0f, 4f, 0.1f, 1f, VoiceSettingsScreen::percent);
        stepper(x + half + 4, y, half, "ferrovoz.settings.threshold", () -> config.vadThreshold, v -> {
            config.vadThreshold = v;
            FerrovozNative.clientSetVadThreshold(h(), v);
        }, 0f, 0.2f, 0.005f, 0.02f, v -> percent(v * 5f));
        y += ROW;

        // Vúmetro (se dibuja en render)
        meterX = x;
        meterY = y + 2;
        meterW = WIDTH;
        y += 26; // barra (8) + texto (9) + margen

        // Interruptores, de dos en dos
        List<Button> toggles = new ArrayList<>();
        toggles.add(cycle(0, 0, half, () -> Component.translatable(config.mode == 1
                        ? "ferrovoz.settings.mode.voice_activation" : "ferrovoz.settings.mode.push_to_talk"),
                b -> {
                    config.mode = config.mode == 1 ? 0 : 1;
                    FerrovozNative.clientSetMode(h(), config.mode);
                }));
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.noise_suppression", onOff(config.noiseSuppression)),
                b -> {
                    config.noiseSuppression = !config.noiseSuppression;
                    FerrovozNative.clientSetNoiseSuppression(h(), config.noiseSuppression);
                }));
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.monitor", onOff(monitor)),
                b -> {
                    monitor = !monitor;
                    FerrovozNative.clientSetMonitor(h(), monitor);
                }));
        // Efectos del entorno
        // Si el servidor lo ha quitado se dice, pero el ajuste se guarda igual para otros servidores.
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.cave_echo",
                        FerrovozNative.clientCaveEchoAllowed(h()) ? onOff(config.caveEcho) : Component.translatable("ferrovoz.settings.off_by_server")),
                b -> {
                    config.caveEcho = !config.caveEcho;
                    FerrovozNative.clientSetCaveEcho(h(), config.caveEcho);
                }));
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.underwater",
                        FerrovozNative.clientUnderwaterAllowed(h()) ? onOff(config.underwaterEffect) : Component.translatable("ferrovoz.settings.off_by_server")),
                b -> {
                    config.underwaterEffect = !config.underwaterEffect;
                    FerrovozNative.clientSetUnderwaterEffect(h(), config.underwaterEffect);
                }));
        // AEC y AGC
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.echo_cancel", onOff(config.echoCancel)),
                b -> {
                    config.echoCancel = !config.echoCancel;
                    FerrovozNative.clientSetMicProcessing(h(), config.echoCancel, config.autoGain);
                }));
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.auto_gain", onOff(config.autoGain)),
                b -> {
                    config.autoGain = !config.autoGain;
                    FerrovozNative.clientSetMicProcessing(h(), config.echoCancel, config.autoGain);
                }));
        // HRTF (con altavoces mejor apagado)
        toggles.add(cycle(0, 0, half, () -> Component.translatable("ferrovoz.settings.hrtf", onOff(config.hrtf)),
                b -> {
                    config.hrtf = !config.hrtf;
                    FerrovozNative.clientSetHrtf(h(), config.hrtf);
                }));
        for (int i = 0; i < toggles.size(); i++) {
            Button b = toggles.get(i);
            b.setPosition(i % 2 == 0 ? x : x + half + 4, y);
            addRenderableWidget(b);
            if (i % 2 == 1) y += ROW;
        }
        y += 6;

        // Jugadores, teclas y salir
        int third = (WIDTH - 8) / 3;
        Button players = Button.builder(Component.translatable("ferrovoz.settings.players"),
                b -> this.minecraft.setScreen(new PlayerVolumeScreen(this))).bounds(x, y, third, 20).build();
        players.active = this.minecraft.getConnection() != null;
        addRenderableWidget(players);
        addRenderableWidget(Button.builder(Component.translatable("ferrovoz.settings.keys"),
                b -> this.minecraft.setScreen(new KeysScreen(this))).bounds(x + third + 4, y, third, 20).build());
        addRenderableWidget(Button.builder(Component.translatable("gui.done"), b -> onClose())
                .bounds(x + 2 * (third + 4), y, WIDTH - 2 * (third + 4), 20).build());
        y += ROW;
        // Desactivar la voz del todo (se guarda para todos los servidores)
        addRenderableWidget(cycle(x, y, WIDTH, () -> Component.translatable("ferrovoz.settings.voice", onOff(!config.voiceDisabled)),
                b -> {
                    FerrovozNative.clientSetMonitor(h(), false);
                    monitor = false;
                    FerrovozClient.setVoiceEnabled(config.voiceDisabled);
                    rebuildWidgets(); // cambia entre cliente de prueba y real
                }));
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float delta) {
        super.render(g, mouseX, mouseY, delta);

        g.drawString(this.font, this.title, (this.width - this.font.width(this.title)) / 2, 12, WHITE, true);

        // Estado de conexión y dispositivos
        Component status;
        int color;
        long handle = h();
        int audio = handle == 0 ? 2 : FerrovozNative.clientAudioStatus(handle);
        if (handle == 0 || audio == 2) {
            status = Component.translatable("ferrovoz.settings.status.device_error");
            color = RED;
        } else if (FerrovozNative.clientMicSilent(handle)) {
            // 3 s de ceros exactos: permiso denegado (macOS) o micro silenciado.
            status = Component.translatable(FerrovozClient.isMac()
                    ? "ferrovoz.settings.status.mic_blocked_mac" : "ferrovoz.settings.status.mic_blocked");
            color = RED;
        } else if (config.voiceDisabled) {
            status = Component.translatable("ferrovoz.settings.status.voice_off");
            color = YELLOW;
        } else if (isPreview()) {
            status = Component.translatable("ferrovoz.settings.status.preview");
            color = YELLOW;
        } else if (FerrovozNative.clientIsConnected(h())) {
            status = Component.translatable("ferrovoz.settings.status.connected");
            color = GREEN;
        } else {
            status = Component.translatable("ferrovoz.settings.status.connecting");
            color = GREY;
        }
        g.drawString(this.font, status, (this.width - this.font.width(status)) / 2, 22, color, false);

        // Red: ping, pérdida, bitrate y túnel
        if (!isPreview() && handle != 0 && FerrovozNative.clientIsConnected(handle)) {
            float[] net = FerrovozNative.clientNetStats(handle);
            if (net != null && net.length >= 4) {
                Component line = Component.translatable("ferrovoz.settings.network",
                        (int) net[0], (int) net[1], (int) (net[2] / 1000),
                        Component.translatable(net[3] > 0.5f ? "ferrovoz.settings.transport.tunnel" : "ferrovoz.settings.transport.udp"));
                int netColor = net[1] >= 8 ? RED : net[1] >= 3 ? YELLOW : GREY;
                g.drawString(this.font, line, (this.width - this.font.width(line)) / 2, this.height - 12, netColor, false);
            }
        }

        // Vúmetro con la marca del umbral
        float level = handle == 0 ? 0f : Math.min(1f, FerrovozNative.clientMicLevel(h()) * 5f);
        float threshold = Math.min(1f, config.vadThreshold * 5f);
        boolean above = level >= threshold;
        int filled = (int) (meterW * level);
        int mark = meterX + (int) (meterW * threshold);
        g.fill(meterX, meterY, meterX + meterW, meterY + 8, DARK);
        g.fill(meterX, meterY, meterX + filled, meterY + 8, above ? GREEN : GREY);
        // La marca se dibuja siempre para poder ajustarla antes de cambiar de modo;
        // en gris con pulsar para hablar.
        g.fill(mark - 1, meterY - 2, mark + 1, meterY + 10, config.mode == 1 ? YELLOW : GREY);
        boolean talking = handle != 0 && FerrovozNative.clientIsTalking(h());
        Component hint = Component.translatable(talking ? "ferrovoz.settings.meter.talking"
                : config.mode == 1 ? "ferrovoz.settings.meter.idle" : "ferrovoz.settings.meter.ptt");
        g.drawString(this.font, hint, meterX, meterY + 11, talking ? GREEN : GREY, false);
    }

    /**
     * Cliente sobre el que actuar. El real se pide cada vez: si se desconecta con la
     * pantalla abierta se libera, y guardar el handle aquí lo dejaría colgando.
     */
    private long h() {
        long live = FerrovozClient.handle();
        return live != 0 ? live : preview;
    }

    private boolean isPreview() {
        return FerrovozClient.handle() == 0 && preview != 0;
    }

    @Override
    public void onClose() {
        FerrovozNative.clientSetMonitor(h(), false);
        if (preview != 0) {
            FerrovozNative.clientStop(preview);
            preview = 0;
        }
        config.save();
        this.minecraft.setScreen(parent);
    }

    // --- Utilidades ---

    /** Botón cuyo texto se recalcula tras cada clic. */
    private static Button cycle(int x, int y, int w, Supplier<Component> label, Consumer<Button> onClick) {
        return Button.builder(label.get(), b -> {
            onClick.accept(b);
            b.setMessage(label.get());
        }).bounds(x, y, w, 20).build();
    }

    /** Deslizador "Nombre: valor" entre {@code min} y {@code max}. */
    private void stepper(int x, int y, int width, String key, Supplier<Float> get, Consumer<Float> set,
                         float min, float max, float step, float def, java.util.function.Function<Float, String> fmt) {
        addRenderableWidget(new VoiceSlider(x, y, width, min, max, step, get.get(),
                v -> Component.translatable(key, fmt.apply(v)), set));
    }

    private static String percent(float v) {
        return Math.round(v * 100f) + "%";
    }

    private static Component onOff(boolean v) {
        return Component.translatable(v ? "options.on" : "options.off");
    }

    /** "" es el dispositivo predeterminado y va siempre primero. */
    private static List<String> withDefault(String[] names) {
        List<String> list = new ArrayList<>();
        list.add("");
        if (names != null) {
            for (String n : names) {
                if (n != null && !n.isEmpty() && !list.contains(n)) list.add(n);
            }
        }
        return list;
    }

    private static String next(List<String> options, String current) {
        int i = options.indexOf(current);
        return options.get((i + 1) % options.size()); // si ya no está, i = -1 -> primero
    }

    private Component deviceLabel(String name) {
        if (name.isEmpty()) return Component.translatable("ferrovoz.settings.default_device");
        // Recortar nombres largos.
        int max = WIDTH - 110;
        String s = name;
        if (this.font.width(s) > max) {
            while (s.length() > 1 && this.font.width(s + "…") > max) s = s.substring(0, s.length() - 1);
            s = s + "…";
        }
        return Component.literal(s);
    }
}
