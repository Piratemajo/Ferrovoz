package dev.ferrovoz;

import dev.ferrovoz.api.FerrovozApi;
import dev.ferrovoz.api.PermissionOverride;
import dev.ferrovoz.api.Position;
import dev.ferrovoz.api.Subscription;
import dev.ferrovoz.api.TalkMode;
import dev.ferrovoz.api.VoiceGroup;
import dev.ferrovoz.api.VoiceGroupException;
import dev.ferrovoz.api.VoicePermission;
import dev.ferrovoz.api.VoicePlayer;
import dev.ferrovoz.api.VoiceWorld;
import dev.ferrovoz.api.audio.AudioChannel;
import dev.ferrovoz.api.audio.FrameSource;
import dev.ferrovoz.api.audio.MicrophoneFrame;
import dev.ferrovoz.api.audio.OpusDecoder;
import dev.ferrovoz.api.audio.OpusEncoder;
import dev.ferrovoz.api.audio.Playback;
import dev.ferrovoz.api.audio.Wav;
import dev.ferrovoz.api.event.EventPriority;
import dev.ferrovoz.api.event.GroupCreateEvent;
import dev.ferrovoz.api.event.GroupDeletedEvent;
import dev.ferrovoz.api.event.GroupMemberEvent;
import dev.ferrovoz.api.event.PlayerMuteEvent;
import dev.ferrovoz.api.event.PlayerStartedSpeakingEvent;
import dev.ferrovoz.api.event.VoiceEvent;
import dev.ferrovoz.api.event.VoiceServerStartedEvent;
import dev.ferrovoz.api.event.VoiceServerStoppingEvent;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Prueba de la API de Java contra la librería nativa de verdad, sin Minecraft. La lanza
 * {@code tools/api_test.sh}. Sale con código 1 si algo falla.
 *
 * <p>Lo que pasa por la red (voz, canales que se oyen) lo prueban los tests de Rust
 * ({@code native/tests/api.rs}); aquí, el pegamento de Java: vistas, permisos, grupos,
 * eventos, canales, códec y WAV.
 */
public final class ApiSelfTest {
    private static int checks, failures;

    private static void check(boolean ok, String what) {
        checks++;
        if (!ok) {
            failures++;
            System.out.println("FALLA: " + what);
        }
    }

    private static void expectThrows(Class<? extends Throwable> type, Runnable r, String what) {
        try {
            r.run();
            check(false, what + " (no lanzó nada)");
        } catch (Throwable t) {
            check(type.isInstance(t), what + " (lanzó " + t + ")");
        }
    }

    private static final Map<UUID, String> NAMES = new ConcurrentHashMap<>();

    private static final ApiCore.Bridge BRIDGE = new ApiCore.Bridge() {
        @Override
        public String platform() {
            return "Test";
        }

        @Override
        public String nameOf(UUID player) {
            return NAMES.get(player);
        }

        @Override
        public VoiceWorld world(Object platformWorld) {
            if (!(platformWorld instanceof String s)) throw new IllegalArgumentException("no es un mundo");
            return new ApiCore.World(s, s.hashCode());
        }
    };

    public static void main(String[] args) throws Exception {
        if (!FerrovozNative.load(true)) {
            System.out.println("No se ha podido cargar la librería nativa (-Dferrovoz.native.path)");
            System.exit(2);
        }
        eventBus();
        wav();
        microphoneFormat();
        codec();
        lifecycle();
        System.out.println(checks + " comprobaciones, " + failures + " fallos");
        System.exit(failures == 0 ? 0 : 1);
    }

    // --- Bus de eventos ---

    private record Ping(int n) implements VoiceEvent {}

    private static void eventBus() {
        VoiceEventBus bus = new VoiceEventBus();
        List<String> order = new ArrayList<>();
        bus.subscribe(Ping.class, EventPriority.LATE, e -> order.add("late"));
        bus.subscribe(Ping.class, e -> order.add("normal"));
        bus.subscribe(Ping.class, EventPriority.EARLY, e -> order.add("early"));
        Subscription all = bus.subscribe(VoiceEvent.class, e -> order.add("all"));
        bus.subscribe(Ping.class, e -> {
            throw new IllegalStateException("un suscriptor roto no para a los demás");
        });
        bus.post(new Ping(1));
        check(order.equals(List.of("early", "normal", "all", "late")), "orden de prioridades: " + order);
        all.unsubscribe();
        all.unsubscribe(); // dos veces no pasa nada
        order.clear();
        bus.post(new Ping(2));
        check(!order.contains("all"), "tras cancelar la suscripción no llega");
        check(bus.wants(Ping.class) && !bus.wants(VoiceServerStartedEvent.class), "wants() mira los tipos");
        // Suscribirse desde dentro de un suscriptor no rompe la vuelta en curso.
        bus.subscribe(Ping.class, e -> bus.subscribe(Ping.class, x -> {}));
        bus.post(new Ping(3));
    }

    // --- WAV ---

    private static byte[] wavBytes(int format, int channels, int rate, int bits, byte[] data) {
        ByteBuffer b = ByteBuffer.allocate(44 + data.length).order(ByteOrder.LITTLE_ENDIAN);
        b.putInt(0x46464952).putInt(36 + data.length).putInt(0x45564157);
        b.putInt(0x20746d66).putInt(16).putShort((short) format).putShort((short) channels).putInt(rate)
                .putInt(rate * channels * bits / 8).putShort((short) (channels * bits / 8)).putShort((short) bits);
        b.putInt(0x61746164).putInt(data.length).put(data);
        return b.array();
    }

    private static void wav() throws IOException {
        // 1 s de estéreo a 44,1 kHz, 16 bits: izquierda 0,5, derecha 0 -> mono 0,25.
        ByteBuffer pcm = ByteBuffer.allocate(44_100 * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int i = 0; i < 44_100; i++) pcm.putShort((short) 16384).putShort((short) 0);
        short[] out = Wav.read(new ByteArrayInputStream(wavBytes(1, 2, 44_100, 16, pcm.array())));
        check(Math.abs(out.length - 48_000) <= 1, "44,1 kHz pasa a 48 kHz: " + out.length);
        check(Math.abs(out[1000] - 8192) <= 2, "estéreo a mono: " + out[1000]);

        // 24 bits a 48 kHz, negativo.
        ByteBuffer p24 = ByteBuffer.allocate(3 * 10);
        for (int i = 0; i < 10; i++) p24.put((byte) 0).put((byte) 0).put((byte) 0xC0); // -0,5
        short[] o24 = Wav.read(new ByteArrayInputStream(wavBytes(1, 1, 48_000, 24, p24.array())));
        check(o24.length == 10 && Math.abs(o24[3] + 16384) <= 2, "24 bits con signo: " + (o24.length > 3 ? o24[3] : -1));

        // Float de 32 bits.
        ByteBuffer pf = ByteBuffer.allocate(4 * 4).order(ByteOrder.LITTLE_ENDIAN);
        for (int i = 0; i < 4; i++) pf.putFloat(0.25f);
        short[] of = Wav.read(new ByteArrayInputStream(wavBytes(3, 1, 48_000, 32, pf.array())));
        check(of.length == 4 && Math.abs(of[0] - 8192) <= 2, "float de 32 bits");

        boolean threw = false;
        try {
            Wav.read(new ByteArrayInputStream("esto no es un wav".getBytes()));
        } catch (IOException e) {
            threw = true;
        }
        check(threw, "un archivo que no es WAV da IOException");

        // FrameSource.of trocea y rellena la última trama.
        FrameSource src = FrameSource.of(new short[960 + 10]);
        check(src.next().length == 960 && src.next().length == 960 && src.next() == null, "FrameSource.of trocea en tramas de 960");
    }

    // --- Formato del micro ---

    private static void microphoneFormat() {
        UUID player = UUID.fromString("01234567-89ab-cdef-0123-456789abcdef");
        ByteBuffer b = ByteBuffer.allocate(2 * 24 + 3 + 2).order(ByteOrder.LITTLE_ENDIAN);
        for (int k = 0; k < 2; k++) {
            ByteBuffer id = ByteBuffer.allocate(16).putLong(player.getMostSignificantBits()).putLong(player.getLeastSignificantBits());
            b.put(id.array()).putInt(41 + k).put((byte) 2).put((byte) k).putShort((short) (k == 0 ? 3 : 2));
            b.put(k == 0 ? new byte[] {1, 2, 3} : new byte[] {4, 5});
        }
        List<MicrophoneFrame> frames = ApiAudio.unpack(b.array());
        check(frames.size() == 2, "dos tramas del micro: " + frames.size());
        MicrophoneFrame f = frames.get(0);
        check(f.player().equals(player) && f.sequence() == 41 && f.mode() == TalkMode.SHOUT && !f.group() && f.opus().length == 3,
                "la trama del micro se lee bien: " + f);
        check(frames.get(1).group(), "flag de grupo");
        check(ApiAudio.unpack(new byte[30]).size() == 1 && ApiAudio.unpack(new byte[10]).isEmpty(), "lo cortado se ignora");
    }

    // --- Códec ---

    private static void codec() {
        ApiCore core = new ApiCore(BRIDGE);
        try (OpusEncoder enc = core.audio().createEncoder(48_000, false); OpusDecoder dec = core.audio().createDecoder()) {
            double in = 0, out = 0;
            for (int n = 0; n < 20; n++) {
                short[] frame = new short[960];
                for (int i = 0; i < 960; i++) frame[i] = (short) (12000 * Math.sin(2 * Math.PI * 300 * (n * 960 + i) / 48_000.0));
                short[] back = dec.decode(enc.encode(frame));
                check(back.length == 960, "960 muestras al decodificar");
                if (n >= 5) {
                    for (int i = 0; i < 960; i++) {
                        in += frame[i] * (double) frame[i];
                        out += back[i] * (double) back[i];
                    }
                }
            }
            check(out > in * 0.5 && out < in * 1.5, "Opus conserva la energía: " + out / in);
            check(dec.decode(null).length == 960, "una trama perdida se rellena");
            expectThrows(IllegalArgumentException.class, () -> enc.encode(new short[100]), "trama de longitud mala");
        }
        OpusEncoder closed = core.audio().createEncoder(24_000, true);
        closed.close();
        closed.close();
        expectThrows(IllegalStateException.class, () -> closed.encode(new short[960]), "codificador cerrado");
    }

    // --- Ciclo de vida, jugadores, grupos, comandos y canales ---

    private static final class Commands implements VoiceCommands.Host {
        final long server;
        final List<String> chat = new ArrayList<>();

        Commands(long server) {
            this.server = server;
        }

        @Override
        public long voiceServer() {
            return server;
        }

        @Override
        public UUID findOnline(String name) {
            return NAMES.entrySet().stream().filter(e -> e.getValue().equalsIgnoreCase(name)).map(Map.Entry::getKey).findFirst().orElse(null);
        }

        @Override
        public String nameOf(UUID player) {
            return NAMES.getOrDefault(player, "?");
        }

        @Override
        public Collection<String> onlineNames() {
            return NAMES.values();
        }

        @Override
        public FerrovozConfig config() {
            return new FerrovozConfig();
        }

        @Override
        public void message(UUID player, String message) {
            chat.add(nameOf(player) + ": " + message);
        }

        @Override
        public String localeOf(UUID player) {
            return "es_es";
        }

        VoiceCommands.Sender as(UUID player, boolean admin) {
            return new VoiceCommands.Sender() {
                @Override
                public UUID player() {
                    return player;
                }

                @Override
                public boolean hasPermission(String node) {
                    return admin || !VoiceCommands.opByDefault(node);
                }

                @Override
                public void reply(String message) {
                    chat.add("respuesta: " + message);
                }

                @Override
                public String locale() {
                    return "es_es";
                }
            };
        }

        void run(VoiceCommands.Sender s, String line) {
            VoiceCommands.execute(s, this, line.split(" "));
        }
    }

    private static void lifecycle() throws Exception {
        ApiCore core = new ApiCore(BRIDGE);
        AtomicReference<FerrovozApi> early = new AtomicReference<>();
        FerrovozApi.whenReady(early::set);
        check(early.get() == null || early.get() != core, "whenReady espera a que haya API");
        core.register();
        check(early.get() == core && FerrovozApi.get() == core, "whenReady se ejecuta al registrar");

        // Parado.
        check(!core.isRunning() && core.players().isEmpty(), "parado: sin jugadores");
        expectThrows(IllegalStateException.class, () -> core.audio().channel().to(List.of()), "parado: no hay canales");
        try {
            core.groups().create("x", null, UUID.randomUUID());
            check(false, "parado: crear grupo lanza");
        } catch (VoiceGroupException e) {
            check(e.reason() == VoiceGroupException.Reason.NOT_RUNNING, "motivo NOT_RUNNING");
        }

        long server = FerrovozNative.serverStart("127.0.0.1", 0, 48f);
        check(server != 0, "arranca el relé");
        List<VoiceEvent> seen = new ArrayList<>();
        core.events().subscribe(VoiceEvent.class, EventPriority.LATE, seen::add);
        FerrovozConfig config = new FerrovozConfig();
        core.started(server, config);
        check(seen.size() == 1 && seen.get(0) instanceof VoiceServerStartedEvent, "evento de arranque");

        UUID ana = UUID.randomUUID(), luis = UUID.randomUUID();
        NAMES.put(ana, "Ana");
        NAMES.put(luis, "Luis");
        for (UUID id : List.of(ana, luis)) {
            check(FerrovozNative.serverAddPlayer(server, id.getMostSignificantBits(), id.getLeastSignificantBits()) != null, "registro");
            core.syncPlayer(id, true, true, false);
        }
        check(core.players().size() == 2, "dos jugadores: " + core.players().size());
        VoicePlayer a = core.player(ana).orElseThrow();
        check(a.name().equals("Ana") && !a.isConnected() && !a.isSpeaking(), "vista del jugador");
        check(core.player(UUID.randomUUID()).isEmpty(), "uno que no está");

        // Permisos forzados y alcance.
        check(a.hasPermission(VoicePermission.SPEAK) && !a.hasPermission(VoicePermission.PRIORITY), "permisos de la plataforma");
        a.setPermission(VoicePermission.PRIORITY, PermissionOverride.ALLOW);
        a.setPermission(VoicePermission.SPEAK, PermissionOverride.DENY);
        core.syncPlayer(ana, true, true, false); // la plataforma vuelve a mandar los suyos
        check(a.hasPermission(VoicePermission.PRIORITY) && !a.hasPermission(VoicePermission.SPEAK), "lo forzado manda");
        a.setPermission(VoicePermission.SPEAK, PermissionOverride.INHERIT);
        check(a.hasPermission(VoicePermission.SPEAK) && a.permissionOverride(VoicePermission.SPEAK) == PermissionOverride.INHERIT, "INHERIT vuelve");
        a.setRange(250);
        check(a.range().orElse(0) == 250, "alcance propio");
        a.resetRange();
        check(a.range().isEmpty(), "alcance quitado");
        expectThrows(IllegalArgumentException.class, () -> a.setRange(0), "alcance 0 no vale");

        // Ajustes y silencio general.
        core.settings().setHushed(true);
        check(core.settings().isHushed(), "silencio general");
        core.settings().setHushed(false);
        core.settings().setUnderwaterEffect(false);
        check(!core.settings().underwaterEffect(), "efecto bajo el agua desactivado");
        core.settings().setUnderwaterEffect(true);
        core.settings().setRange(TalkMode.SHOUT, 200);
        check(core.settings().range(TalkMode.SHOUT) == 200 && core.settings().range(TalkMode.NORMAL) == config.maxDistance, "alcances");

        // Grupos por la API.
        VoiceGroup g = core.groups().create("Escenario", "clave", ana);
        g.add(luis);
        check(g.members().equals(Set.of(ana, luis)) && g.hasPassword() && g.owner().orElseThrow().equals(ana), "grupo con dos");
        check(core.groups().get("ESCENARIO").orElseThrow().name().equals("Escenario"), "nombre sin mayúsculas");
        check(core.player(luis).orElseThrow().group().orElseThrow().equals(g), "grupo del jugador");
        g.setPersistent(true);
        check(g.isPersistent(), "permanente");
        try {
            core.groups().create("escenario", null, luis);
            check(false, "nombre repetido lanza");
        } catch (VoiceGroupException e) {
            check(e.reason() == VoiceGroupException.Reason.ALREADY_EXISTS, "motivo ALREADY_EXISTS");
        }

        // Eventos del relé a la API, a través de VoiceCommands.
        Commands cmd = new Commands(server);
        seen.clear();
        VoiceCommands.dispatchEvents(cmd);
        check(seen.stream().anyMatch(e -> e instanceof GroupMemberEvent m && m.type() == GroupMemberEvent.Type.JOINED
                && m.player().equals(luis) && m.group().equals("Escenario")), "evento de unirse: " + seen);
        check(cmd.chat.stream().anyMatch(m -> m.startsWith("Ana:") && m.contains("Luis")), "aviso por chat al grupo: " + cmd.chat);
        g.delete();
        seen.clear();
        VoiceCommands.dispatchEvents(cmd);
        check(!g.exists() && seen.stream().anyMatch(e -> e instanceof GroupDeletedEvent d && d.by().isEmpty()), "borrado por un plugin");

        // Comandos de jugadores: los eventos cancelables.
        Subscription veto = core.events().subscribe(GroupCreateEvent.class, e -> e.setCancelled(e.group().startsWith("prohibido")));
        cmd.run(cmd.as(luis, false), "grupo crear prohibido1");
        check(core.groups().get("prohibido1").isEmpty(), "un plugin impide crear un grupo");
        cmd.run(cmd.as(luis, false), "grupo crear permitido");
        check(core.groups().get("permitido").isPresent(), "sin cancelar sí se crea");
        veto.unsubscribe();
        core.events().subscribe(PlayerMuteEvent.class, e -> e.setCancelled(true));
        cmd.run(cmd.as(ana, true), "silenciar Luis");
        check(!core.player(luis).orElseThrow().isMuted(), "un plugin impide silenciar");

        // Eventos de hablar, tal como los manda el núcleo.
        AtomicReference<PlayerStartedSpeakingEvent> spoke = new AtomicReference<>();
        core.events().subscribe(PlayerStartedSpeakingEvent.class, spoke::set);
        core.dispatch("speak", ana, "shout");
        check(spoke.get() != null && spoke.get().mode() == TalkMode.SHOUT && !spoke.get().group(), "evento de gritar");
        core.dispatch("speak", ana, "group");
        check(spoke.get().group(), "evento de hablar al grupo");
        core.dispatch("de-un-nucleo-mas-nuevo", ana, "");

        // Canales.
        VoiceWorld world = core.world("overworld");
        expectThrows(IllegalArgumentException.class, () -> core.world(42), "un mundo que no es de la plataforma");
        AudioChannel at = core.audio().channel().music().at(new Position(world, 0, 64, 0), 32);
        AudioChannel follow = core.audio().channel().following(ana, 16);
        AudioChannel direct = core.audio().channel().bitrate(24_000).to(List.of(luis));
        AudioChannel group = core.audio().channel().toGroup("permitido");
        check(at.id().toString().startsWith("66657272-6f76-6f7a"), "id de canal con el prefijo: " + at.id());
        check(at.send(new short[960]) && follow.send(new short[960]) && direct.send(new short[960]) && group.send(new short[960]),
                "los cuatro tipos envían");
        expectThrows(IllegalArgumentException.class, () -> at.send(new short[10]), "trama corta");
        expectThrows(IllegalArgumentException.class, () -> core.audio().channel().at(new Position(world, 0, 0, 0), 0), "alcance 0");
        Playback p = at.play(FrameSource.of(new short[960 * 10]));
        p.finished().get(3, TimeUnit.SECONDS);
        check(!p.isPlaying(), "la reproducción acaba sola");
        Playback forever = direct.play(FrameSource.loop(new short[960]));
        direct.close();
        forever.finished().get(3, TimeUnit.SECONDS);
        check(!direct.isOpen() && !direct.send(new short[960]), "un canal cerrado no envía");
        expectThrows(IllegalStateException.class, () -> direct.moveTo(new Position(world, 0, 0, 0)), "mover un canal cerrado");

        // Escucha del micro: al suscribirse se activa; sin jugadores hablando no llega nada.
        List<MicrophoneFrame> heard = new ArrayList<>();
        Subscription mic = core.audio().listen(heard::add);
        Thread.sleep(100);
        mic.unsubscribe();
        check(heard.isEmpty(), "nadie habla, nada llega");

        // Parar: se avisa y se cierra todo.
        Playback looping = follow.play(FrameSource.loop(new short[960]));
        seen.clear();
        core.stopping();
        check(seen.stream().anyMatch(e -> e instanceof VoiceServerStoppingEvent), "evento de parada");
        check(!at.isOpen() && !follow.isOpen() && !group.isOpen(), "al parar se cierran los canales");
        looping.finished().get(3, TimeUnit.SECONDS);
        check(!core.isRunning() && core.player(ana).isEmpty(), "parado otra vez");
        FerrovozNative.serverStop(server);
        core.unregister();
        check(FerrovozApi.find().isEmpty(), "sin registrar");
    }
}
