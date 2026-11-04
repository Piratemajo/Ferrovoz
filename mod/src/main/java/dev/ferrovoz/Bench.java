package dev.ferrovoz;

import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;

import java.io.IOException;
import java.lang.management.ManagementFactory;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Random;
import java.util.UUID;

/**
 * Benchmark en un servidor de Fabric real. Solo con {@code -Dferrovoz.bench=<jugadores>}.
 *
 * <p>Mide lo que Ferrovoz cuesta al servidor de Minecraft, no solo al relé:
 * <ol>
 *   <li>Reposo (20 s): mundo cargado y sin voz, como referencia.</li>
 *   <li>Registra N jugadores virtuales repartidos sobre el terreno y escribe sus
 *       claves en {@code ferrovoz-bench/keys.txt}. Un proceso aparte
 *       ({@code native/examples/bench_clients.rs}) hace de clientes y envía voz
 *       cifrada por UDP.</li>
 *   <li>Medición ({@code -Dferrovoz.bench.seconds}, 60 por defecto): cada tick los
 *       jugadores virtuales se mueven y pasan por el mismo código que los reales
 *       (posiciones, permisos, avisos de grupo, oclusión con el mundo).</li>
 * </ol>
 *
 * Al acabar escribe {@code ferrovoz-bench/report.md} y para el servidor.
 *
 * <p>No incluye lo que cuesta a Minecraft tener jugadores reales (entidades, chunks,
 * red); eso está con o sin Ferrovoz.
 */
final class Bench {
    static final int PLAYERS = Integer.getInteger("ferrovoz.bench", 0);
    private static final int SECONDS = Integer.getInteger("ferrovoz.bench.seconds", 60);
    /** {@code spread}: grupos de 5 por el mapa; {@code crowd}: todos juntos. */
    private static final String LAYOUT = System.getProperty("ferrovoz.bench.layout", "spread");
    /** Porcentaje de jugadores hablando a la vez. */
    private static final int TALKING = Integer.getInteger("ferrovoz.bench.talking", 20);
    private static final int IDLE_SECONDS = 20;
    private static final int WARMUP_SECONDS = 10;
    private static final int CONNECT_TIMEOUT_SECONDS = 180;
    private static final Path DIR = Path.of("ferrovoz-bench");

    private enum Phase { IDLE, WAIT_CLIENTS, WARMUP, MEASURE, DONE }

    private static final class Virtual {
        final UUID id;
        final double homeX, homeZ;
        double x, y, z;

        Virtual(UUID id, double x, double y, double z) {
            this.id = id;
            this.homeX = x;
            this.homeZ = z;
            this.x = x;
            this.y = y;
            this.z = z;
        }
    }

    private static Phase phase;
    private static long phaseStart;
    private static long tickStart;
    private static ServerLevel level;
    private static int dimension;
    private static final List<Virtual> players = new ArrayList<>();
    private static final Map<UUID, Virtual> byId = new HashMap<>();
    private static final Random random = new Random(42);
    private static int ticks;

    /** Duración de cada tick (ns) y la parte de Ferrovoz, por fase. */
    private static final Samples idleTicks = new Samples(), measureTicks = new Samples(), measureFerrovoz = new Samples();
    private static long pendingFerrovozNs;

    // Contadores al empezar la medición
    private static long[] status0;
    private static long cpu0, relayCpu0, wall0;
    private static long idleHeap, idleRss;

    private Bench() {}

    static boolean enabled() {
        return PLAYERS > 0;
    }

    /** Al arrancar el servidor, con el relé ya en marcha. */
    static void start(MinecraftServer srv) {
        if (!enabled()) return;
        level = srv.overworld();
        dimension = level.dimension().toString().hashCode();
        // Carga y fija los chunks de la zona antes del reposo, para que la referencia
        // ya incluya su coste. Como /forceload (máx. 256 chunks; aquí 15x15 como mucho).
        int radius = LAYOUT.equals("crowd") ? 32 : 112;
        srv.getCommands().performPrefixedCommand(srv.createCommandSourceStack(),
                "forceload add " + (-radius) + " " + (-radius) + " " + radius + " " + radius);
        for (int cx = (-radius) >> 4; cx <= radius >> 4; cx++) {
            for (int cz = (-radius) >> 4; cz <= radius >> 4; cz++) {
                level.getChunk(cx, cz); // generar antes de medir
            }
        }
        try {
            Files.createDirectories(DIR);
            for (String f : new String[] {"keys.txt", "measure_start", "measure_end", "report.md", "result.properties"}) {
                Files.deleteIfExists(DIR.resolve(f));
            }
        } catch (IOException e) {
            Ferrovoz.LOG.error("[bench] cannot write to {}", DIR.toAbsolutePath(), e);
        }
        warnIfServerPauses();
        startWatchdog();
        phase = Phase.IDLE;
        phaseStart = System.nanoTime();
        Ferrovoz.LOG.info("[bench] {} virtual players, layout {}, {}% talking, {} s measurement. Idle {} s…",
                PLAYERS, LAYOUT, TALKING, SECONDS, IDLE_SECONDS);
    }

    /**
     * Desde 1.21.2 un servidor vacío deja de hacer ticks a los 60 s
     * ({@code pause-when-empty-seconds}). Los jugadores virtuales no cuentan, así que
     * hay que ponerlo a 0.
     */
    private static void warnIfServerPauses() {
        try {
            java.util.Properties p = new java.util.Properties();
            try (var r = Files.newBufferedReader(Path.of("server.properties"))) {
                p.load(r);
            }
            String v = p.getProperty("pause-when-empty-seconds", "60").trim();
            if (Integer.parseInt(v) > 0) {
                Ferrovoz.LOG.error("[bench] pause-when-empty-seconds={} in server.properties: the server will PAUSE after {} s "
                        + "without real players and the benchmark will never finish. Set pause-when-empty-seconds=0.", v, v);
            }
        } catch (IOException | RuntimeException ignored) {
            // sin server.properties legible; el watchdog avisará si se para
        }
    }

    private static volatile long lastTickNanos = System.nanoTime();

    /** Si el servidor deja de hacer ticks, avisar en el log. */
    private static void startWatchdog() {
        Thread t = new Thread(() -> {
            boolean warned = false;
            while (phase != Phase.DONE) {
                try {
                    Thread.sleep(2000);
                } catch (InterruptedException e) {
                    return;
                }
                boolean stalled = System.nanoTime() - lastTickNanos > 10_000_000_000L;
                if (stalled && !warned && phase != null) {
                    Ferrovoz.LOG.error("[bench] the server has not ticked for over 10 s (phase {}). "
                            + "Did it pause because it is empty? Set pause-when-empty-seconds=0 in server.properties.", phase);
                }
                warned = stalled;
            }
        }, "ferrovoz-bench-watchdog");
        t.setDaemon(true);
        t.start();
    }

    static void onStartTick() {
        lastTickNanos = System.nanoTime();
        if (phase != null) tickStart = System.nanoTime();
    }

    /** Tiempo de {@code Ferrovoz.tick} en este tick (incluye los jugadores virtuales). */
    static void ferrovozTime(long nanos) {
        pendingFerrovozNs += nanos;
    }

    /** Jugadores virtuales registrados (desde el fin del reposo). */
    static boolean active() {
        return phase != null && phase != Phase.IDLE && phase != Phase.DONE;
    }

    /** Cada tick, dentro de {@code Ferrovoz.tick}. */
    static void tickPlayers(long s, boolean refreshPermissions) {
        if (!active()) return;
        for (Virtual v : players) {
            // Paseo aleatorio alrededor de su sitio (~2 bloques/s como mucho).
            v.x = clamp(v.x + (random.nextDouble() - 0.5) * 0.2, v.homeX - 6, v.homeX + 6);
            v.z = clamp(v.z + (random.nextDouble() - 0.5) * 0.2, v.homeZ - 6, v.homeZ + 6);
            FerrovozNative.serverUpdatePlayer(s, v.id.getMostSignificantBits(), v.id.getLeastSignificantBits(),
                    v.x, v.y, v.z, dimension, false, FerrovozNative.ROLE_ALIVE);
            if (refreshPermissions) {
                FerrovozNative.serverSetPermissions(s, v.id.getMostSignificantBits(), v.id.getLeastSignificantBits(), true, true);
            }
        }
    }

    /** Posición de los ojos de un jugador virtual, o null. */
    static Ferrovoz.Located locate(UUID id) {
        Virtual v = byId.get(id);
        return v == null ? null : new Ferrovoz.Located(level, v.x, v.y, v.z);
    }

    static void onEndTick(MinecraftServer srv, long voiceServer) {
        if (phase == null || phase == Phase.DONE) return;
        long now = System.nanoTime();
        long tick = now - tickStart;
        long ferrovoz = pendingFerrovozNs;
        pendingFerrovozNs = 0;
        ticks++;
        double inPhase = (now - phaseStart) / 1e9;

        switch (phase) {
            case IDLE -> {
                idleTicks.add(tick);
                if (inPhase >= IDLE_SECONDS) {
                    idleHeap = heapAfterGc();
                    idleRss = rssKb();
                    registerPlayers(voiceServer);
                    next(Phase.WAIT_CLIENTS);
                    Ferrovoz.LOG.info("[bench] keys written to {}; waiting for clients…", DIR.resolve("keys.txt").toAbsolutePath());
                }
            }
            case WAIT_CLIENTS -> {
                int connected = FerrovozNative.serverConnectedCount(voiceServer);
                if (connected >= PLAYERS || inPhase >= CONNECT_TIMEOUT_SECONDS) {
                    Ferrovoz.LOG.info("[bench] {} of {} clients connected; warming up {} s", connected, PLAYERS, WARMUP_SECONDS);
                    next(Phase.WARMUP);
                }
            }
            case WARMUP -> {
                if (inPhase >= WARMUP_SECONDS) {
                    status0 = FerrovozNative.serverStatus(voiceServer);
                    cpu0 = processCpuNs();
                    relayCpu0 = relayCpuNs(status0);
                    wall0 = System.nanoTime();
                    // Los clientes leen de aquí la duración.
                    writeAtomically("measure_start", Integer.toString(SECONDS));
                    next(Phase.MEASURE);
                    Ferrovoz.LOG.info("[bench] measuring {} s…", SECONDS);
                }
            }
            case MEASURE -> {
                measureTicks.add(tick);
                measureFerrovoz.add(ferrovoz);
                if (inPhase >= SECONDS) {
                    touch("measure_end");
                    report(voiceServer);
                    next(Phase.DONE);
                    Ferrovoz.LOG.info("[bench] done: {}", DIR.resolve("report.md").toAbsolutePath());
                    srv.halt(false);
                }
            }
            default -> { }
        }
    }

    private static void next(Phase p) {
        phase = p;
        phaseStart = System.nanoTime();
    }

    // --- Jugadores virtuales ---

    private static void registerPlayers(long s) {
        StringBuilder keys = new StringBuilder();
        keys.append("port ").append(boundPort()).append('\n');
        int talkEvery = Math.max(1, Math.round(100f / Math.max(1, TALKING)));
        for (int i = 0; i < PLAYERS; i++) {
            double x, z;
            if (LAYOUT.equals("crowd")) {
                x = (i % 10) * 2.0 - 10;
                z = (i / 10) * 2.0 - 10;
            } else {
                // Grupos de 5 a 3 bloques entre ellos; grupos a 40 bloques en rejilla de 5 columnas.
                int cluster = i / 5;
                x = (cluster % 5) * 40.0 - 80 + (i % 5) * 3.0;
                z = (cluster / 5) * 40.0 - 80;
            }
            double y = surface((int) Math.floor(x), (int) Math.floor(z)) + 1.62;
            UUID id = new UUID(0xFE0000000000L | i, 0xBE4C40000000L | i);
            Virtual v = new Virtual(id, x, y, z);
            players.add(v);
            byId.put(id, v);
            byte[] key = FerrovozNative.serverAddPlayer(s, id.getMostSignificantBits(), id.getLeastSignificantBits());
            FerrovozNative.serverUpdatePlayer(s, id.getMostSignificantBits(), id.getLeastSignificantBits(), x, y, z, dimension, false, FerrovozNative.ROLE_ALIVE);
            FerrovozNative.serverSetPermissions(s, id.getMostSignificantBits(), id.getLeastSignificantBits(), true, true);
            boolean talking = i % talkEvery == 0;
            keys.append(String.format(Locale.ROOT, "%016x%016x %s %d%n",
                    id.getMostSignificantBits(), id.getLeastSignificantBits(), hex(key), talking ? 1 : 0));
        }
        // 1 de cada 20 jugadores crea un grupo y otro se une, para que también cuenten.
        for (int i = 0; i + 1 < PLAYERS; i += 20) {
            UUID a = players.get(i).id, b = players.get(i + 1).id;
            FerrovozNative.serverCreateGroup(s, a.getMostSignificantBits(), a.getLeastSignificantBits(), "bench" + i, null);
            FerrovozNative.serverJoinGroup(s, b.getMostSignificantBits(), b.getLeastSignificantBits(), "bench" + i, null);
        }
        writeAtomically("keys.txt", keys.toString());
    }

    /** Altura del bloque sólido más alto de la columna. */
    private static int surface(int x, int z) {
        for (int y = 200; y > -60; y--) {
            if (level.getBlockState(new BlockPos(x, y, z)).isSolidRender()) return y + 1;
        }
        return 64;
    }

    private static int boundPort() {
        return Ferrovoz.config().port;
    }

    // --- Medidas ---

    private static final class Samples {
        private long[] v = new long[4096];
        private int n;

        void add(long x) {
            if (n == v.length) v = Arrays.copyOf(v, n * 2);
            v[n++] = x;
        }

        double avgMs() {
            long sum = 0;
            for (int i = 0; i < n; i++) sum += v[i];
            return n == 0 ? 0 : sum / 1e6 / n;
        }

        double pctMs(double p) {
            if (n == 0) return 0;
            long[] s = Arrays.copyOf(v, n);
            Arrays.sort(s);
            return s[Math.min(n - 1, (int) Math.ceil(p / 100.0 * n) - 1)] / 1e6;
        }

        int count() {
            return n;
        }
    }

    private static long processCpuNs() {
        if (ManagementFactory.getOperatingSystemMXBean() instanceof com.sun.management.OperatingSystemMXBean os) {
            return os.getProcessCpuTime();
        }
        return -1;
    }

    /**
     * CPU del relé: su hilo y los auxiliares ("ferrovoz-fanout-N", Linux corta el
     * nombre a 15 caracteres). De /proc en Linux; si no, lo que cuenta el propio relé.
     */
    private static long relayCpuNs(long[] status) {
        long total = 0;
        boolean found = false;
        try (var tasks = Files.list(Path.of("/proc/self/task"))) {
            for (Path t : (Iterable<Path>) tasks::iterator) {
                String name = Files.readString(t.resolve("comm")).trim();
                if (!name.equals("ferrovoz-server") && !name.startsWith("ferrovoz-fanout")) continue;
                String stat = Files.readString(t.resolve("stat"));
                String[] f = stat.substring(stat.lastIndexOf(')') + 2).split(" ");
                total += (Long.parseLong(f[11]) + Long.parseLong(f[12])) * 10_000_000L; // USER_HZ = 100
                found = true;
            }
        } catch (IOException | RuntimeException ignored) {
            // no es Linux
        }
        if (found) return total;
        return status.length >= 9 ? status[8] : -1;
    }

    private static long heapAfterGc() {
        System.gc();
        Runtime r = Runtime.getRuntime();
        return r.totalMemory() - r.freeMemory();
    }

    private static long rssKb() {
        try {
            for (String line : Files.readAllLines(Path.of("/proc/self/status"))) {
                if (line.startsWith("VmRSS:")) return Long.parseLong(line.replaceAll("\\D+", ""));
            }
        } catch (IOException | RuntimeException ignored) {
            // no es Linux
        }
        return -1;
    }

    private static void report(long s) {
        long[] st = FerrovozNative.serverStatus(s);
        double wall = (System.nanoTime() - wall0) / 1e9;
        long cpu = processCpuNs() - cpu0;
        long relay = relayCpuNs(st) - relayCpu0;
        long heap = heapAfterGc();
        long rss = rssKb();
        int cores = Runtime.getRuntime().availableProcessors();
        double pin = (st[3] - status0[3]) / wall, pout = (st[4] - status0[4]) / wall;
        double tps = measureTicks.count() / wall;

        String r = String.format(Locale.ROOT, """
                # Benchmark de Ferrovoz en un servidor de Fabric

                %d jugadores virtuales (%s), %d %% hablando, %d s de medición. Máquina: %d núcleos, Java %s.

                | | Reposo (sin voz) | Con Ferrovoz |
                |---|---|---|
                | Tick medio | %.2f ms | %.2f ms |
                | Tick p95 / p99 | %.2f / %.2f ms | %.2f / %.2f ms |
                | Tick máximo | %.2f ms | %.2f ms |
                | TPS | | %.1f |

                **Lo que Ferrovoz añade al tick** (posiciones, permisos, avisos, paredes): media **%.3f ms**, p95 %.3f ms, p99 %.3f ms, máximo %.3f ms (de 50 ms por tick).

                | Recurso | Valor |
                |---|---|
                | CPU del relé de voz (sus hilos) | %.1f %% de un núcleo |
                | CPU de todo el proceso del servidor | %.1f %% de un núcleo |
                | Paquetes de voz recibidos / enviados por segundo | %.0f / %.0f |
                | Rechazados / limitados durante la medición | %d / %d |
                | Memoria Java (tras GC) | %.1f MB → %.1f MB |
                | Memoria del proceso (RSS) | %s |
                """,
                PLAYERS, LAYOUT, TALKING, SECONDS, cores, System.getProperty("java.version"),
                idleTicks.avgMs(), measureTicks.avgMs(),
                idleTicks.pctMs(95), idleTicks.pctMs(99), measureTicks.pctMs(95), measureTicks.pctMs(99),
                idleTicks.pctMs(100), measureTicks.pctMs(100),
                tps,
                measureFerrovoz.avgMs(), measureFerrovoz.pctMs(95), measureFerrovoz.pctMs(99), measureFerrovoz.pctMs(100),
                relay < 0 ? -1.0 : 100.0 * relay / 1e9 / wall,
                cpu < 0 ? -1.0 : 100.0 * cpu / 1e9 / wall,
                pin, pout, st[5] - status0[5], st[7] - status0[7],
                idleHeap / 1048576.0, heap / 1048576.0,
                idleRss < 0 ? "n/d" : String.format(Locale.ROOT, "%.0f MB → %.0f MB", idleRss / 1024.0, rss / 1024.0));
        writeAtomically("report.md", r);

        String props = String.format(Locale.ROOT, """
                players=%d
                layout=%s
                ferrovoz_tick_avg_ms=%.4f
                ferrovoz_tick_p99_ms=%.4f
                tick_idle_avg_ms=%.3f
                tick_avg_ms=%.3f
                tick_p95_ms=%.3f
                relay_cpu_percent=%.2f
                packets_out=%d
                """,
                PLAYERS, LAYOUT, measureFerrovoz.avgMs(), measureFerrovoz.pctMs(99),
                idleTicks.avgMs(), measureTicks.avgMs(), measureTicks.pctMs(95),
                relay < 0 ? -1.0 : 100.0 * relay / 1e9 / wall, st[4] - status0[4]);
        writeAtomically("result.properties", props);
    }

    // --- Utilidades ---

    private static void touch(String name) {
        writeAtomically(name, Long.toString(System.currentTimeMillis()));
    }

    private static void writeAtomically(String name, String content) {
        try {
            Path tmp = DIR.resolve(name + ".tmp");
            Files.writeString(tmp, content);
            Files.move(tmp, DIR.resolve(name), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        } catch (IOException e) {
            Ferrovoz.LOG.error("[bench] could not write {}", name, e);
        }
    }

    private static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder(b.length * 2);
        for (byte x : b) sb.append(String.format("%02x", x));
        return sb.toString();
    }

    private static double clamp(double v, double lo, double hi) {
        return Math.max(lo, Math.min(hi, v));
    }
}
