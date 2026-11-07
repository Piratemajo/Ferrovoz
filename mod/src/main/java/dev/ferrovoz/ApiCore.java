package dev.ferrovoz;

import dev.ferrovoz.api.FerrovozApi;
import dev.ferrovoz.api.PermissionOverride;
import dev.ferrovoz.api.TalkMode;
import dev.ferrovoz.api.VoiceGroup;
import dev.ferrovoz.api.VoiceGroupException;
import dev.ferrovoz.api.VoiceGroups;
import dev.ferrovoz.api.VoicePermission;
import dev.ferrovoz.api.VoicePlayer;
import dev.ferrovoz.api.VoiceSettings;
import dev.ferrovoz.api.VoiceWorld;
import dev.ferrovoz.api.audio.AudioService;
import dev.ferrovoz.api.event.EventBus;
import dev.ferrovoz.api.event.GroupDeletedEvent;
import dev.ferrovoz.api.event.GroupMemberEvent;
import dev.ferrovoz.api.event.PlayerConnectedEvent;
import dev.ferrovoz.api.event.PlayerDisconnectedEvent;
import dev.ferrovoz.api.event.PlayerStartedSpeakingEvent;
import dev.ferrovoz.api.event.PlayerStoppedSpeakingEvent;
import dev.ferrovoz.api.event.VoiceEvent;
import dev.ferrovoz.api.event.VoiceServerStartedEvent;
import dev.ferrovoz.api.event.VoiceServerStoppingEvent;
import dev.ferrovoz.api.internal.ApiAccess;

import java.util.ArrayList;
import java.util.Collection;
import java.util.EnumMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.OptionalDouble;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Implementación de {@link FerrovozApi}, común a todas las plataformas: no toca
 * Minecraft. Cada plataforma pone un {@link Bridge} y avisa de cuándo arranca y se para
 * el relé, de los permisos de cada jugador ({@link #syncPlayer}) y de cuándo recoger
 * los eventos ({@link VoiceCommands#dispatchEvents}).
 */
public final class ApiCore implements FerrovozApi {
    /** Lo que solo sabe la plataforma. */
    public interface Bridge {
        String platform();

        /** Nombre de un jugador conectado, o null. */
        String nameOf(UUID player);

        /** @throws IllegalArgumentException si no es un mundo de esta plataforma */
        VoiceWorld world(Object platformWorld);
    }

    /** Un mundo: su nombre y el entero con que lo distingue el relé. */
    public record World(String name, int dimension) implements VoiceWorld {}

    private static final double MAX_RANGE = 30_000;
    /** La que está en uso, para los eventos cancelables de {@link VoiceCommands}. */
    private static volatile ApiCore active;

    private final Bridge bridge;
    private final VoiceEventBus bus = new VoiceEventBus();
    private final ApiAudio audio = new ApiAudio(this);
    private final Settings settings = new Settings();
    private final Groups groups = new Groups();
    private volatile long server;

    // Lo que fijan los plugins, por jugador; dura hasta que se para el relé.
    private final Map<UUID, Map<VoicePermission, PermissionOverride>> overrides = new ConcurrentHashMap<>();
    private final Map<UUID, Double> ranges = new ConcurrentHashMap<>();
    /** Últimos permisos que dio la plataforma: hablar, gritar, prioridad. */
    private final Map<UUID, boolean[]> granted = new ConcurrentHashMap<>();

    public ApiCore(Bridge bridge) {
        this.bridge = Objects.requireNonNull(bridge, "bridge");
    }

    /** Deja la API disponible para los demás mods o plugins. */
    public void register() {
        active = this;
        ApiAccess.register(this);
    }

    public void unregister() {
        ApiAccess.unregister(this);
        if (active == this) active = null;
    }

    /** La API en uso, o null si no hay. */
    static ApiCore activeCore() {
        return active;
    }

    /**
     * Lanza un evento cancelable de los comandos. true si nadie lo ha cancelado (o no hay
     * API).
     */
    static boolean allowed(dev.ferrovoz.api.event.Cancellable event) {
        ApiCore core = active;
        return core == null || !core.bus.post(event).isCancelled();
    }

    // --- Ciclo de vida, desde la plataforma ---

    /** El relé está en marcha con esta configuración. */
    public void started(long server, FerrovozConfig config) {
        settings.load(config);
        this.server = server;
        audio.start(server);
        bus.post(new VoiceServerStartedEvent());
    }

    /** El relé se va a parar (todavía responde). */
    public void stopping() {
        if (server == 0) return;
        bus.post(new VoiceServerStoppingEvent());
        audio.stop();
        server = 0;
        overrides.clear();
        ranges.clear();
        granted.clear();
    }

    /**
     * Permisos de un jugador según la plataforma, al entrar y cada pocos segundos. Aplica
     * lo que haya forzado un plugin y su alcance propio.
     */
    public void syncPlayer(UUID id, boolean speak, boolean shout, boolean priority) {
        granted.put(id, new boolean[] {speak, shout, priority});
        apply(id);
    }

    private void apply(UUID id) {
        long s = server;
        if (s == 0) return;
        long msb = id.getMostSignificantBits(), lsb = id.getLeastSignificantBits();
        FerrovozNative.serverSetPermissions(s, msb, lsb, effective(id, VoicePermission.SPEAK), effective(id, VoicePermission.SHOUT));
        FerrovozNative.serverSetPriority(s, msb, lsb, effective(id, VoicePermission.PRIORITY));
        Double range = ranges.get(id);
        FerrovozNative.serverSetRangeOverride(s, msb, lsb, range == null ? 0f : range.floatValue());
    }

    private boolean effective(UUID id, VoicePermission p) {
        boolean[] g = granted.get(id);
        boolean base = g != null ? g[p.ordinal()] : p != VoicePermission.PRIORITY;
        Map<VoicePermission, PermissionOverride> o = overrides.get(id);
        return (o == null ? PermissionOverride.INHERIT : o.getOrDefault(p, PermissionOverride.INHERIT)).apply(base);
    }

    /** Un evento del núcleo ({@code tipo}, jugador, detalle) a su evento de la API. */
    void dispatch(String kind, UUID who, String detail) {
        VoiceEvent event = switch (kind) {
            case "join" -> member(GroupMemberEvent.Type.JOINED, detail, who);
            case "leave" -> member(GroupMemberEvent.Type.LEFT, detail, who);
            case "kick" -> member(GroupMemberEvent.Type.KICKED, detail, who);
            case "rejoin" -> member(GroupMemberEvent.Type.REJOINED, detail, who);
            case "owner" -> member(GroupMemberEvent.Type.NEW_OWNER, detail, who);
            case "delete" -> bus.wants(GroupDeletedEvent.class)
                    ? new GroupDeletedEvent(detail, isNil(who) ? Optional.empty() : Optional.of(who)) : null;
            case "connect" -> bus.wants(PlayerConnectedEvent.class) ? new PlayerConnectedEvent(new Player(who)) : null;
            case "disconnect" -> bus.wants(PlayerDisconnectedEvent.class) ? new PlayerDisconnectedEvent(who) : null;
            case "speak" -> bus.wants(PlayerStartedSpeakingEvent.class) ? new PlayerStartedSpeakingEvent(new Player(who),
                    switch (detail) {
                        case "whisper" -> TalkMode.WHISPER;
                        case "shout" -> TalkMode.SHOUT;
                        default -> TalkMode.NORMAL;
                    }, detail.equals("group")) : null;
            case "silent" -> bus.wants(PlayerStoppedSpeakingEvent.class) ? new PlayerStoppedSpeakingEvent(new Player(who)) : null;
            default -> null; // de un núcleo más nuevo
        };
        if (event != null) bus.post(event);
    }

    private GroupMemberEvent member(GroupMemberEvent.Type type, String group, UUID who) {
        return bus.wants(GroupMemberEvent.class) ? new GroupMemberEvent(type, group, who) : null;
    }

    private static boolean isNil(UUID id) {
        return id.getMostSignificantBits() == 0 && id.getLeastSignificantBits() == 0;
    }

    long server() {
        return server;
    }

    long requireRunning() {
        long s = server;
        if (s == 0) throw new IllegalStateException("the voice relay is not running");
        return s;
    }

    int dimensionOf(VoiceWorld world) {
        if (world instanceof World w) return w.dimension();
        throw new IllegalArgumentException("world not obtained from FerrovozApi.world(): " + world);
    }

    static long[] pack(Collection<UUID> ids) {
        long[] out = new long[ids.size() * 2];
        int i = 0;
        for (UUID id : ids) {
            out[i++] = id.getMostSignificantBits();
            out[i++] = id.getLeastSignificantBits();
        }
        return out;
    }

    static List<UUID> unpack(long[] v) {
        List<UUID> out = new ArrayList<>();
        for (int i = 0; v != null && i + 1 < v.length; i += 2) out.add(new UUID(v[i], v[i + 1]));
        return out;
    }

    // --- FerrovozApi ---

    @Override
    public String version() {
        return Protocol.modVersion();
    }

    @Override
    public String platform() {
        return bridge.platform();
    }

    @Override
    public boolean isRunning() {
        return server != 0;
    }

    @Override
    public Optional<VoicePlayer> player(UUID id) {
        Objects.requireNonNull(id, "id");
        long s = server;
        if (s == 0 || (FerrovozNative.serverPlayerState(s, id.getMostSignificantBits(), id.getLeastSignificantBits())
                & FerrovozNative.STATE_REGISTERED) == 0) {
            return Optional.empty();
        }
        return Optional.of(new Player(id));
    }

    @Override
    public Collection<VoicePlayer> players() {
        long s = server;
        if (s == 0) return List.of();
        List<VoicePlayer> out = new ArrayList<>();
        for (UUID id : unpack(FerrovozNative.serverPlayers(s))) out.add(new Player(id));
        return out;
    }

    @Override
    public VoiceGroups groups() {
        return groups;
    }

    @Override
    public VoiceSettings settings() {
        return settings;
    }

    @Override
    public AudioService audio() {
        return audio;
    }

    @Override
    public EventBus events() {
        return bus;
    }

    @Override
    public VoiceWorld world(Object platformWorld) {
        Objects.requireNonNull(platformWorld, "platformWorld");
        return bridge.world(platformWorld);
    }

    // --- Jugadores ---

    private final class Player implements VoicePlayer {
        private final UUID id;
        private final long msb, lsb;

        Player(UUID id) {
            this.id = id;
            this.msb = id.getMostSignificantBits();
            this.lsb = id.getLeastSignificantBits();
        }

        private int state() {
            long s = server;
            return s == 0 ? 0 : FerrovozNative.serverPlayerState(s, msb, lsb);
        }

        @Override
        public UUID id() {
            return id;
        }

        @Override
        public String name() {
            String n = bridge.nameOf(id);
            return n != null ? n : id.toString();
        }

        @Override
        public boolean isConnected() {
            return (state() & FerrovozNative.STATE_CONNECTED) != 0;
        }

        @Override
        public boolean isSpeaking() {
            return (state() & FerrovozNative.STATE_SPEAKING) != 0;
        }

        @Override
        public boolean isMicrophoneMuted() {
            return (state() & FerrovozNative.STATE_SELF_MUTED) != 0;
        }

        @Override
        public boolean isDeafened() {
            return (state() & FerrovozNative.STATE_DEAFENED) != 0;
        }

        @Override
        public boolean isMuted() {
            long s = server;
            return s != 0 && FerrovozNative.serverIsMuted(s, msb, lsb);
        }

        @Override
        public void setMuted(boolean muted) {
            long s = server;
            if (s != 0) FerrovozNative.serverSetMuted(s, msb, lsb, muted);
        }

        @Override
        public Optional<VoiceGroup> group() {
            long s = server;
            String name = s == 0 ? null : FerrovozNative.serverGroupOf(s, msb, lsb);
            return name == null ? Optional.empty() : Optional.of(new Group(name));
        }

        @Override
        public int packetLoss() {
            long s = server;
            return s == 0 ? 0 : Math.max(0, FerrovozNative.serverLossPercent(s, msb, lsb));
        }

        @Override
        public void setRange(double blocks) {
            if (!(blocks >= 1 && blocks <= MAX_RANGE)) throw new IllegalArgumentException("range must be within 1-30000: " + blocks);
            ranges.put(id, blocks);
            apply(id);
        }

        @Override
        public void resetRange() {
            ranges.remove(id);
            apply(id);
        }

        @Override
        public OptionalDouble range() {
            Double r = ranges.get(id);
            return r == null ? OptionalDouble.empty() : OptionalDouble.of(r);
        }

        @Override
        public PermissionOverride permissionOverride(VoicePermission permission) {
            Objects.requireNonNull(permission, "permission");
            Map<VoicePermission, PermissionOverride> o = overrides.get(id);
            return o == null ? PermissionOverride.INHERIT : o.getOrDefault(permission, PermissionOverride.INHERIT);
        }

        @Override
        public void setPermission(VoicePermission permission, PermissionOverride value) {
            Objects.requireNonNull(permission, "permission");
            Objects.requireNonNull(value, "value");
            overrides.compute(id, (k, o) -> {
                Map<VoicePermission, PermissionOverride> m = o == null ? new EnumMap<>(VoicePermission.class) : new EnumMap<>(o);
                if (value == PermissionOverride.INHERIT) m.remove(permission);
                else m.put(permission, value);
                return m.isEmpty() ? null : m;
            });
            apply(id);
        }

        @Override
        public boolean hasPermission(VoicePermission permission) {
            Objects.requireNonNull(permission, "permission");
            return effective(id, permission);
        }

        @Override
        public boolean equals(Object o) {
            return o instanceof Player p && p.id.equals(id);
        }

        @Override
        public int hashCode() {
            return id.hashCode();
        }

        @Override
        public String toString() {
            return "VoicePlayer[" + name() + "]";
        }
    }

    // --- Grupos ---

    private static VoiceGroupException groupError(int code, String name) {
        VoiceGroupException.Reason reason = VoiceGroupException.reasonOf(code);
        return new VoiceGroupException(reason, reason.name().toLowerCase(Locale.ROOT).replace('_', ' ') + ": " + name);
    }

    private long serverForGroups() {
        long s = server;
        if (s == 0) throw new VoiceGroupException(VoiceGroupException.Reason.NOT_RUNNING, "the voice relay is not running");
        return s;
    }

    private final class Groups implements VoiceGroups {
        @Override
        public Optional<VoiceGroup> get(String name) {
            Objects.requireNonNull(name, "name");
            for (VoiceGroup g : all()) {
                if (g.name().equalsIgnoreCase(name.trim())) return Optional.of(g);
            }
            return Optional.empty();
        }

        @Override
        public List<VoiceGroup> all() {
            long s = server;
            String[] list = s == 0 ? null : FerrovozNative.serverListGroups(s);
            if (list == null) return List.of();
            List<VoiceGroup> out = new ArrayList<>(list.length);
            for (String entry : list) {
                int bar = entry.indexOf('|');
                out.add(new Group(bar < 0 ? entry : entry.substring(0, bar)));
            }
            return out;
        }

        @Override
        public VoiceGroup create(String name, String password, UUID owner) {
            Objects.requireNonNull(name, "name");
            Objects.requireNonNull(owner, "owner");
            int code = FerrovozNative.serverCreateGroup(serverForGroups(), owner.getMostSignificantBits(),
                    owner.getLeastSignificantBits(), name, password);
            if (code != 0) throw groupError(code, name);
            return new Group(name.trim());
        }
    }

    private final class Group implements VoiceGroup {
        private final String name;

        Group(String name) {
            this.name = name;
        }

        /** {@code [dueñoMsb, dueñoLsb, flags]} o null. */
        private long[] info() {
            long s = server;
            return s == 0 ? null : FerrovozNative.serverGroupInfo(s, name);
        }

        @Override
        public String name() {
            return name;
        }

        @Override
        public boolean exists() {
            return info() != null;
        }

        @Override
        public Optional<UUID> owner() {
            long[] i = info();
            return i != null && (i[2] & 1) != 0 ? Optional.of(new UUID(i[0], i[1])) : Optional.empty();
        }

        @Override
        public Set<UUID> members() {
            long s = server;
            return s == 0 ? Set.of() : new LinkedHashSet<>(unpack(FerrovozNative.serverGroupMembers(s, name)));
        }

        @Override
        public boolean hasPassword() {
            long[] i = info();
            return i != null && (i[2] & 2) != 0;
        }

        @Override
        public boolean isPersistent() {
            long[] i = info();
            return i != null && (i[2] & 4) != 0;
        }

        @Override
        public void setPersistent(boolean persistent) {
            int code = FerrovozNative.serverSetGroupPersistentByName(serverForGroups(), name, persistent);
            if (code != 0) throw groupError(code, name);
        }

        @Override
        public void add(UUID player) {
            Objects.requireNonNull(player, "player");
            int code = FerrovozNative.serverForceJoinGroup(serverForGroups(), player.getMostSignificantBits(),
                    player.getLeastSignificantBits(), name);
            if (code != 0) throw groupError(code, name);
        }

        @Override
        public boolean remove(UUID player) {
            Objects.requireNonNull(player, "player");
            long s = serverForGroups();
            String current = FerrovozNative.serverGroupOf(s, player.getMostSignificantBits(), player.getLeastSignificantBits());
            return name.equalsIgnoreCase(current)
                    && FerrovozNative.serverLeaveGroup(s, player.getMostSignificantBits(), player.getLeastSignificantBits());
        }

        @Override
        public void delete() {
            int code = FerrovozNative.serverDeleteGroup(serverForGroups(), 0, 0, name, true);
            if (code != 0) throw groupError(code, name);
        }

        @Override
        public boolean equals(Object o) {
            return o instanceof Group g && g.name.equalsIgnoreCase(name);
        }

        @Override
        public int hashCode() {
            return name.toLowerCase(Locale.ROOT).hashCode();
        }

        @Override
        public String toString() {
            return "VoiceGroup[" + name + "]";
        }
    }

    // --- Ajustes ---

    private final class Settings implements VoiceSettings {
        private volatile double normal = 48, whisper = 8, shout = 96;
        private volatile int maxVoices = 24;
        private volatile boolean caveEcho = true;

        void load(FerrovozConfig c) {
            normal = c.maxDistance;
            whisper = c.whisperDistance;
            shout = c.shoutDistance;
            maxVoices = c.maxVoices;
            caveEcho = c.caveEcho;
        }

        @Override
        public double range(TalkMode mode) {
            return switch (Objects.requireNonNull(mode, "mode")) {
                case WHISPER -> whisper;
                case NORMAL -> normal;
                case SHOUT -> shout;
            };
        }

        @Override
        public synchronized void setRange(TalkMode mode, double blocks) {
            Objects.requireNonNull(mode, "mode");
            if (!(blocks >= 1 && blocks <= MAX_RANGE)) throw new IllegalArgumentException("range must be within 1-30000: " + blocks);
            switch (mode) {
                case WHISPER -> whisper = blocks;
                case NORMAL -> normal = blocks;
                case SHOUT -> shout = blocks;
            }
            long s = server;
            if (s != 0) FerrovozNative.serverSetRanges(s, (float) normal, (float) whisper, (float) shout);
        }

        @Override
        public int maxVoices() {
            return maxVoices;
        }

        @Override
        public void setMaxVoices(int value) {
            if (value < 0 || value > 256) throw new IllegalArgumentException("max voices must be within 0-256: " + value);
            maxVoices = value;
            long s = server;
            if (s != 0) FerrovozNative.serverSetMaxVoices(s, value);
        }

        @Override
        public boolean caveEcho() {
            return caveEcho;
        }

        @Override
        public void setCaveEcho(boolean enabled) {
            caveEcho = enabled;
            long s = server;
            if (s != 0) FerrovozNative.serverSetCaveEcho(s, enabled);
        }

        @Override
        public boolean isHushed() {
            long s = server;
            return s != 0 && FerrovozNative.serverIsHushed(s);
        }

        @Override
        public void setHushed(boolean hushed) {
            long s = server;
            if (s != 0) FerrovozNative.serverSetHush(s, hushed);
        }
    }
}
