package dev.ferrovoz;

import dev.ferrovoz.api.event.GroupCreateEvent;
import dev.ferrovoz.api.event.GroupJoinEvent;
import dev.ferrovoz.api.event.PlayerMuteEvent;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collection;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;

/**
 * Comando {@code /voz} (alias {@code /voice}), común al mod y al plugin. Cada
 * plataforma implementa {@link Sender} y {@link Host}; aquí no hay nada de Minecraft.
 *
 * <p>Subcomandos en español o inglés ({@code grupo crear} = {@code group create});
 * las respuestas van en el idioma del jugador (ver {@link Messages}).
 *
 * <pre>
 *   /voz                                 ayuda
 *   /voz grupo crear &lt;nombre&gt; [contraseña]
 *   /voz grupo unirse &lt;nombre&gt; [contraseña]
 *   /voz grupo salir
 *   /voz grupo lista
 *   /voz grupo expulsar &lt;jugador&gt;      (dueño del grupo o admin)
 *   /voz grupo fijar | desfijar          (dueño o admin: grupo permanente, se guarda)
 *   /voz grupo borrar &lt;nombre&gt;         (dueño o admin)
 *   /voz estado                          (admin)
 *   /voz silenciar &lt;jugador&gt;            (admin)
 *   /voz desilenciar &lt;jugador&gt;          (admin)
 * </pre>
 *
 * Permisos ({@link #PERM_SPEAK}, etc.): en LuckPerms son {@code ferrovoz.speak},
 * {@code ferrovoz.admin}... tanto en Paper como en Fabric (LuckPerms traduce
 * {@code ferrovoz:speak} de Fabric a ese nodo).
 */
public final class VoiceCommands {
    /** Hablar (por defecto: todos). */
    public static final String PERM_SPEAK = "speak";
    /** Gritar (por defecto: todos). */
    public static final String PERM_SHOUT = "shout";
    /** Usar grupos (por defecto: todos). */
    public static final String PERM_GROUPS = "groups";
    /** Comandos de moderación (por defecto: operadores). */
    public static final String PERM_ADMIN = "admin";
    /**
     * Voz siempre por separado aunque haya multitud, para el anfitrión de un evento
     * (por defecto: operadores).
     */
    public static final String PERM_PRIORITY = "priority";

    /** Permisos que por defecto solo tienen los operadores. */
    public static boolean opByDefault(String node) {
        return PERM_ADMIN.equals(node) || PERM_PRIORITY.equals(node);
    }

    private static final String ERR = "§c";

    /** Nombre en español e inglés -> subcomando interno. */
    private static final Map<String, String> ALIASES = Map.ofEntries(
            Map.entry("grupo", "group"), Map.entry("group", "group"),
            Map.entry("estado", "status"), Map.entry("status", "status"),
            Map.entry("silenciar", "mute"), Map.entry("mute", "mute"),
            Map.entry("desilenciar", "unmute"), Map.entry("unmute", "unmute"),
            Map.entry("crear", "create"), Map.entry("create", "create"),
            Map.entry("unirse", "join"), Map.entry("join", "join"),
            Map.entry("salir", "leave"), Map.entry("leave", "leave"),
            Map.entry("lista", "list"), Map.entry("list", "list"),
            Map.entry("expulsar", "kick"), Map.entry("kick", "kick"),
            Map.entry("fijar", "pin"), Map.entry("pin", "pin"),
            Map.entry("desfijar", "unpin"), Map.entry("unpin", "unpin"),
            Map.entry("borrar", "delete"), Map.entry("delete", "delete"));

    private static final List<String> TOP_ES = List.of("grupo", "estado", "silenciar", "desilenciar");
    private static final List<String> TOP_EN = List.of("group", "status", "mute", "unmute");
    private static final List<String> GROUP_ES = List.of("crear", "unirse", "salir", "lista", "expulsar", "fijar", "desfijar", "borrar");
    private static final List<String> GROUP_EN = List.of("create", "join", "leave", "list", "kick", "pin", "unpin", "delete");

    /** Quien ejecuta el comando. */
    public interface Sender {
        /** null si es la consola. */
        UUID player();

        /** {@code node} es el sufijo ("speak", "admin"...). Cada plataforma pone su prefijo y su valor por defecto. */
        boolean hasPermission(String node);

        void reply(String message);

        /** Idioma del jugador ({@code es_es}, {@code en_us}...); en consola, el del sistema. */
        String locale();
    }

    /** Servidor. */
    public interface Host {
        long voiceServer();

        /** UUID del jugador conectado con ese nombre (sin distinguir mayúsculas), o null. */
        UUID findOnline(String name);

        String nameOf(UUID player);

        Collection<String> onlineNames();

        FerrovozConfig config();

        /** Mensaje de chat a un jugador conectado (si no lo está, nada). */
        void message(UUID player, String message);

        /** Idioma de un jugador conectado (null si no se sabe: inglés). */
        String localeOf(UUID player);
    }

    private VoiceCommands() {}

    /** Idioma de la consola: el del sistema. */
    public static String consoleLocale() {
        return Locale.getDefault().toString();
    }

    private static String canonical(String arg) {
        return ALIASES.getOrDefault(arg.toLowerCase(Locale.ROOT), "");
    }

    public static void execute(Sender s, Host h, String[] args) {
        String lang = s.locale();
        if (h.voiceServer() == 0) {
            s.reply(Messages.prefixed(lang, "not_running"));
            return;
        }
        String sub = args.length == 0 ? "" : canonical(args[0]);
        switch (sub) {
            case "group" -> group(s, h, Arrays.copyOfRange(args, 1, args.length));
            case "status" -> status(s, h);
            case "mute" -> mute(s, h, args, true);
            case "unmute" -> mute(s, h, args, false);
            default -> help(s);
        }
    }

    private static void help(Sender s) {
        String lang = s.locale();
        s.reply(Messages.prefixed(lang, "help.title"));
        if (s.hasPermission(PERM_GROUPS)) {
            for (String k : List.of("help.groups1", "help.groups2", "help.groups3")) s.reply("  " + Messages.tr(lang, k));
        }
        if (s.hasPermission(PERM_ADMIN)) {
            for (String k : List.of("help.admin1", "help.admin2")) s.reply("  " + Messages.tr(lang, k));
        }
    }

    private static String err(String lang, String key, Object... args) {
        return Messages.tr(lang, "prefix") + " " + ERR + Messages.tr(lang, key, args);
    }

    // --- Grupos ---

    private static void group(Sender s, Host h, String[] a) {
        String lang = s.locale();
        if (!s.hasPermission(PERM_GROUPS)) {
            s.reply(err(lang, "groups.no_permission"));
            return;
        }
        String sub = a.length == 0 ? "" : canonical(a[0]);
        if (sub.equals("list")) {
            listGroups(s, h);
            return;
        }
        UUID me = s.player();
        if (me == null) {
            s.reply(err(lang, "groups.players_only"));
            return;
        }
        long srv = h.voiceServer();
        long msb = me.getMostSignificantBits(), lsb = me.getLeastSignificantBits();
        boolean admin = s.hasPermission(PERM_ADMIN);
        switch (sub) {
            case "create" -> {
                if (a.length < 2) {
                    s.reply(err(lang, "usage.create"));
                    return;
                }
                String pass = a.length >= 3 ? a[2] : null;
                if (!ApiCore.allowed(new GroupCreateEvent(me, a[1], pass != null))) return;
                int r = FerrovozNative.serverCreateGroup(srv, msb, lsb, a[1], pass);
                s.reply(r == 0 ? Messages.prefixed(lang, pass != null ? "group.created_locked" : "group.created", a[1])
                        : groupError(lang, r, a[1]));
            }
            case "join" -> {
                if (a.length < 2) {
                    s.reply(err(lang, "usage.join"));
                    return;
                }
                if (!ApiCore.allowed(new GroupJoinEvent(me, a[1]))) return;
                int r = FerrovozNative.serverJoinGroup(srv, msb, lsb, a[1], a.length >= 3 ? a[2] : null);
                s.reply(r == 0 ? Messages.prefixed(lang, "group.joined", FerrovozNative.serverGroupOf(srv, msb, lsb))
                        : groupError(lang, r, a[1]));
            }
            case "leave" -> s.reply(FerrovozNative.serverLeaveGroup(srv, msb, lsb)
                    ? Messages.prefixed(lang, "group.left") : err(lang, "group.not_in_group"));
            case "kick" -> {
                if (a.length < 2) {
                    s.reply(err(lang, "usage.kick"));
                    return;
                }
                UUID target = h.findOnline(a[1]);
                if (target == null) {
                    s.reply(err(lang, "player_not_found", a[1]));
                    return;
                }
                if (target.equals(me)) {
                    s.reply(err(lang, "group.kick_self"));
                    return;
                }
                int r = FerrovozNative.serverKickFromGroup(srv, msb, lsb,
                        target.getMostSignificantBits(), target.getLeastSignificantBits(), admin);
                s.reply(r == 0 ? Messages.prefixed(lang, "group.kicked", h.nameOf(target)) : groupError(lang, r, a[1]));
            }
            case "pin", "unpin" -> {
                boolean pin = sub.equals("pin");
                int r = FerrovozNative.serverSetGroupPersistent(srv, msb, lsb, pin, admin);
                s.reply(r == 0 ? Messages.prefixed(lang, pin ? "group.pinned" : "group.unpinned") : groupError(lang, r, ""));
            }
            case "delete" -> {
                if (a.length < 2) {
                    s.reply(err(lang, "usage.delete"));
                    return;
                }
                int r = FerrovozNative.serverDeleteGroup(srv, msb, lsb, a[1], admin);
                s.reply(r == 0 ? Messages.prefixed(lang, "group.deleted", a[1]) : groupError(lang, r, a[1]));
            }
            default -> s.reply(err(lang, "usage.group"));
        }
    }

    private static void listGroups(Sender s, Host h) {
        String lang = s.locale();
        String[] groups = FerrovozNative.serverListGroups(h.voiceServer());
        if (groups == null || groups.length == 0) {
            s.reply(Messages.prefixed(lang, "list.empty"));
            return;
        }
        s.reply(Messages.prefixed(lang, "list.title", groups.length));
        for (String g : groups) {
            String[] f = g.split("\\|");
            if (f.length < 4) continue;
            String members = f[1].equals("1") ? Messages.tr(lang, "list.members_one") : Messages.tr(lang, "list.members", f[1]);
            long[] owner = FerrovozNative.serverGroupOwner(h.voiceServer(), f[0]);
            String ownerName = owner != null && owner.length == 2
                    ? h.nameOf(new UUID(owner[0], owner[1])) : Messages.tr(lang, "list.nobody");
            s.reply("  " + (f[3].equals("1") ? "§6★ " : "§f") + f[0] + "§7 · " + members + " · "
                    + Messages.tr(lang, "list.owner", ownerName)
                    + (f[2].equals("1") ? " · " + Messages.tr(lang, "list.locked") : ""));
        }
    }

    static String groupError(String lang, int code, String name) {
        return code >= 1 && code <= 8 ? err(lang, "error." + code, name) : err(lang, "error.unknown", code);
    }

    // --- Administración ---

    /** Para calcular la CPU del relé entre dos {@code /voz estado}. */
    private static long lastBusyNs = -1, lastStatusNanos;

    private static void status(Sender s, Host h) {
        String lang = s.locale();
        if (!s.hasPermission(PERM_ADMIN)) {
            s.reply(err(lang, "no_permission"));
            return;
        }
        long[] st = FerrovozNative.serverStatus(h.voiceServer());
        FerrovozConfig c = h.config();
        s.reply(Messages.prefixed(lang, "status.title"));
        s.reply("  " + Messages.tr(lang, "status.version", Protocol.modVersion(), Protocol.VERSION));
        s.reply("  " + Messages.tr(lang, "status.players", st[0], st[1], st[2]));
        s.reply("  " + Messages.tr(lang, "status.network", c.port, c.transportName(),
                (int) c.whisperDistance + "/" + (int) c.maxDistance + "/" + (int) c.shoutDistance));
        s.reply("  " + Messages.tr(lang, "status.packets", st[3], st[4], st[5]));
        if (st.length >= 8) {
            s.reply("  " + Messages.tr(lang, "status.security", st[6], st[7]));
        }
        if (st.length >= 9) {
            long now = System.nanoTime();
            synchronized (VoiceCommands.class) {
                if (lastBusyNs >= 0 && now > lastStatusNanos && st[8] >= lastBusyNs) {
                    double pct = 100.0 * (st[8] - lastBusyNs) / (now - lastStatusNanos);
                    s.reply("  " + Messages.tr(lang, "status.cpu", String.format(Locale.ROOT, "%.1f", pct)));
                }
                lastBusyNs = st[8];
                lastStatusNanos = now;
            }
        }
    }

    private static void mute(Sender s, Host h, String[] a, boolean mute) {
        String lang = s.locale();
        if (!s.hasPermission(PERM_ADMIN)) {
            s.reply(err(lang, "no_permission"));
            return;
        }
        if (a.length < 2) {
            s.reply(err(lang, mute ? "usage.mute" : "usage.unmute"));
            return;
        }
        UUID target = h.findOnline(a[1]);
        if (target == null) {
            s.reply(err(lang, "player_not_found", a[1]));
            return;
        }
        long srv = h.voiceServer();
        long msb = target.getMostSignificantBits(), lsb = target.getLeastSignificantBits();
        boolean was = FerrovozNative.serverIsMuted(srv, msb, lsb);
        if (was != mute && !ApiCore.allowed(new PlayerMuteEvent(target, mute, s.player()))) return;
        FerrovozNative.serverSetMuted(srv, msb, lsb, mute);
        String name = h.nameOf(target);
        if (was == mute) {
            s.reply(Messages.prefixed(lang, mute ? "mute.already" : "mute.not_muted", name));
        } else {
            s.reply(Messages.prefixed(lang, mute ? "mute.done" : "unmute.done", name));
        }
    }

    // --- Eventos del relé ---

    /**
     * Recoge los eventos del relé: avisa por chat a cada afectado de un grupo, en su
     * idioma, y los pasa a la API. Llamar periódicamente, desde el hilo donde deban
     * llegar los eventos de la API.
     */
    public static void dispatchEvents(Host h) {
        long srv = h.voiceServer();
        if (srv == 0) return;
        String[] events = FerrovozNative.serverPollEvents(srv);
        if (events == null) return;
        ApiCore api = ApiCore.activeCore();
        for (String e : events) {
            // tipo|uuid|detalle: el detalle va el último porque puede llevar '|'.
            String[] f = e.split("\\|", 3);
            if (f.length < 3) continue;
            UUID who;
            try {
                who = UUID.fromString(f[1]);
            } catch (IllegalArgumentException ex) {
                continue;
            }
            groupNotice(h, srv, f[0], who, f[2]);
            if (api != null) api.dispatch(f[0], who, f[2]);
        }
    }

    private static void groupNotice(Host h, long srv, String kind, UUID who, String group) {
        String toMembers = switch (kind) {
            case "join" -> "event.join";
            case "leave" -> "event.leave";
            case "kick" -> "event.kick";
            case "rejoin" -> "event.rejoin";
            case "owner" -> "event.owner";
            default -> null;
        };
        String toThem = switch (kind) {
            case "kick" -> "event.kicked_you";
            case "rejoin" -> "event.rejoined_you";
            case "owner" -> "event.owner_you";
            case "dissolve" -> "event.deleted_you";
            default -> null;
        };
        if (toThem != null) {
            String l = h.localeOf(who);
            String text = Messages.tr(l, toThem, group);
            h.message(who, Messages.tr(l, "prefix") + " " + (kind.equals("kick") ? ERR : "") + text);
        }
        if (toMembers == null) return;
        String name = "§b" + h.nameOf(who) + "§r";
        long[] members = FerrovozNative.serverGroupMembers(srv, group);
        for (int i = 0; members != null && i + 1 < members.length; i += 2) {
            UUID m = new UUID(members[i], members[i + 1]);
            if (m.equals(who)) continue;
            String l = h.localeOf(m);
            h.message(m, Messages.tr(l, "prefix") + " §7[" + group + "]§r " + Messages.tr(l, toMembers, name));
        }
    }

    // --- Autocompletado ---

    public static List<String> complete(Sender s, Host h, String[] args) {
        boolean es = Messages.language(s.locale()).equals("es");
        List<String> options = new ArrayList<>();
        String first = args.length >= 1 ? canonical(args[0]) : "";
        String second = args.length >= 2 ? canonical(args[1]) : "";
        if (args.length <= 1) {
            List<String> top = es ? TOP_ES : TOP_EN;
            if (s.hasPermission(PERM_GROUPS)) options.add(top.get(0));
            if (s.hasPermission(PERM_ADMIN)) options.addAll(top.subList(1, 4));
        } else if (args.length == 2 && first.equals("group")) {
            if (s.hasPermission(PERM_GROUPS)) options.addAll(es ? GROUP_ES : GROUP_EN);
        } else if (args.length == 3 && first.equals("group") && second.equals("kick")
                && s.player() != null && h.voiceServer() != 0) {
            UUID me = s.player();
            String mine = FerrovozNative.serverGroupOf(h.voiceServer(), me.getMostSignificantBits(), me.getLeastSignificantBits());
            if (mine != null) {
                long[] m = FerrovozNative.serverGroupMembers(h.voiceServer(), mine);
                for (int i = 0; m != null && i + 1 < m.length; i += 2) {
                    UUID id = new UUID(m[i], m[i + 1]);
                    if (!id.equals(me)) options.add(h.nameOf(id));
                }
            }
        } else if (args.length == 3 && first.equals("group") && (second.equals("join") || second.equals("delete"))
                && h.voiceServer() != 0) {
            String[] groups = FerrovozNative.serverListGroups(h.voiceServer());
            if (groups != null) {
                for (String g : groups) options.add(g.split("\\|")[0]);
            }
        } else if (args.length == 2 && s.hasPermission(PERM_ADMIN) && (first.equals("mute") || first.equals("unmute"))) {
            options.addAll(h.onlineNames());
        }
        String prefix = args.length == 0 ? "" : args[args.length - 1].toLowerCase(Locale.ROOT);
        options.removeIf(o -> !o.toLowerCase(Locale.ROOT).startsWith(prefix));
        return options;
    }
}
