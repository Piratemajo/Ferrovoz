package dev.ferrovoz.proxy;

import dev.ferrovoz.FerrovozLog;
import dev.ferrovoz.FerrovozNative;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.SetupMessage;

import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.net.InetSocketAddress;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Properties;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Ferrovoz en un proxy (Velocity, BungeeCord), común a ambos.
 *
 * <p>Cada servidor de detrás tiene su relé (el mod o plugin normal). El proxy:
 * <ol>
 *   <li>Arranca un reenviador en un único puerto UDP público.</li>
 *   <li>Lee el mensaje de setup ({@link Protocol#CHANNEL}) que el servidor envía al
 *       jugador, apunta a qué relé va (IP del servidor + puerto de voz) y lo cambia
 *       para que el cliente se conecte al proxy.</li>
 *   <li>Al cambiar de servidor o salir, borra la ruta.</li>
 * </ol>
 * El túnel pasa por el proxy como cualquier plugin message.
 *
 * <p>Grupos de red ({@code /vozred}, {@code /voicenet}): viven en el proxy, así que
 * sus miembros se oyen desde servidores distintos. La voz la reparte el reenviador
 * (ver {@code forwarder.rs}) sin que los servidores se enteren. Requieren UDP.
 */
public final class ProxyCore {
    /** Configuración en {@code ferrovoz-proxy.properties}. */
    public static final class Config {
        /** Puerto UDP público. */
        public int port = 24454;
        /** Interfaz (vacío = todas). */
        public String bindHost = "";
        /** Host anunciado a los clientes (vacío = el que usan para entrar al proxy). */
        public String publicHost = "";

        static Config load(Path file) {
            Config c = new Config();
            Properties p = new Properties();
            if (Files.exists(file)) {
                try (Reader r = Files.newBufferedReader(file)) {
                    p.load(r);
                    c.port = Integer.parseInt(p.getProperty("port", String.valueOf(c.port)).trim());
                    c.bindHost = p.getProperty("bind_host", c.bindHost).trim();
                    c.publicHost = p.getProperty("public_host", c.publicHost).trim();
                } catch (IOException | NumberFormatException e) {
                    FerrovozLog.LOG.warn("{} is invalid, using defaults", file, e);
                }
            }
            try {
                Files.createDirectories(file.getParent());
                try (Writer w = Files.newBufferedWriter(file)) {
                    w.write("""
                            # Ferrovoz on the proxy (Velocity, BungeeCord...)
                            # Players send their voice to THIS UDP port and the proxy forwards it to the
                            # server they are on. Open it in the proxy's firewall/router.
                            # On each backend server, use a different voice port if they share a machine
                            # (e.g. 24455, 24456...); those do not need to be reachable from outside.
                            port=%d
                            # Interface to listen on (empty = all)
                            bind_host=%s
                            # Host announced to clients (empty = the same one they use to join the proxy)
                            public_host=%s
                            """.formatted(c.port, c.bindHost, c.publicHost));
                }
            } catch (IOException e) {
                FerrovozLog.LOG.warn("Could not write {}", file, e);
            }
            return c;
        }
    }

    private record Route(UUID voiceId, String server) {}

    /** Jugador conectado al proxy (cada plataforma lo adapta). */
    public interface NetPlayer {
        UUID id();

        String name();

        /** Idioma del cliente ({@code es_es}, {@code en_us}...). */
        String locale();

        /** Mensaje con códigos de color {@code §}. */
        void tell(String legacyText);

        /** {@code ferrovoz.admin}: gestionar cualquier grupo de red. */
        boolean isAdmin();
    }

    /** Búsqueda de jugadores conectados. */
    public interface Players {
        NetPlayer byName(String name);

        NetPlayer byId(UUID id);
    }

    private static final class NetGroup {
        final int id;
        final String name;
        final String password; // null = sin contraseña
        UUID owner;
        final Set<UUID> members = new LinkedHashSet<>();

        NetGroup(int id, String name, String password, UUID owner) {
            this.id = id;
            this.name = name;
            this.password = password;
            this.owner = owner;
        }
    }

    /** Nombre en minúsculas -> grupo. */
    private final Map<String, NetGroup> netGroups = new ConcurrentHashMap<>();
    /** Jugador -> grupo de red. */
    private final Map<UUID, NetGroup> netMember = new ConcurrentHashMap<>();
    private int nextNetGroup = 1;
    private Players players;

    private final String platform;
    private Config config;
    private volatile long handle;
    /** UUID en el proxy -> ruta (la UUID de voz la decide el servidor). */
    private final Map<UUID, Route> routes = new ConcurrentHashMap<>();

    public ProxyCore(String platform) {
        this.platform = platform;
    }

    /** Para los grupos de red. */
    public void setPlayers(Players players) {
        this.players = players;
    }

    /** @return false si no hay librería nativa o el puerto está ocupado (el proxy sigue, sin voz). */
    public boolean start(Path dataDir) {
        config = Config.load(dataDir.resolve("ferrovoz-proxy.properties"));
        if (!FerrovozNative.load(true)) {
            FerrovozLog.LOG.error("Ferrovoz: no native library for {}; the proxy will not forward voice "
                    + "(players can still use the tunnel through the game connection).", FerrovozNative.platform());
            return false;
        }
        handle = FerrovozNative.proxyStart(config.bindHost, config.port);
        if (handle == 0) {
            FerrovozLog.LOG.error("Ferrovoz: could not open proxy UDP port {}. Is it in use?", config.port);
            return false;
        }
        FerrovozLog.LOG.info("Ferrovoz ({}): voice forwarder on UDP port {}", platform, config.port);
        return true;
    }

    public void stop() {
        long h = handle;
        handle = 0;
        if (h != 0) FerrovozNative.proxyStop(h);
        routes.clear();
        netGroups.clear();
        netMember.clear();
    }

    /**
     * Un servidor envía el mensaje de setup a un jugador.
     *
     * @param server        nombre del servidor en el proxy
     * @param serverAddress dirección del servidor según la configuración del proxy
     * @return los bytes a enviar al jugador, o {@code data} sin cambios si no hay que
     *         tocarlo (sin reenviador, solo túnel, mensaje desconocido...)
     */
    public byte[] onSetup(UUID player, String server, InetSocketAddress serverAddress, byte[] data) {
        long h = handle;
        if (h == 0) return data;
        SetupMessage.Setup s = SetupMessage.decodeSetup(data);
        if (s == null || s.transport() == Protocol.TRANSPORT_TUNNEL) return data;
        // Relé: IP del servidor (o la que anuncie) y su puerto de voz.
        String host = !s.host().isEmpty() ? s.host()
                : serverAddress.getAddress() != null ? serverAddress.getAddress().getHostAddress() : serverAddress.getHostString();
        if (!FerrovozNative.proxyRoute(h, s.idMsb(), s.idLsb(), s.key(), host, s.port())) {
            FerrovozLog.LOG.warn("Ferrovoz: cannot reach the voice relay of {} at {}:{}; that player will connect directly",
                    server, host, s.port());
            return data;
        }
        Route old = routes.put(player, new Route(new UUID(s.idMsb(), s.idLsb()), server));
        if (old != null && !old.voiceId().equals(new UUID(s.idMsb(), s.idLsb()))) remove(old);
        // Nuevo servidor = nueva UUID de voz; el grupo de red tiene que saberlo.
        synchronized (this) {
            NetGroup g = netMember.get(player);
            if (g != null) pushGroup(g);
        }
        return SetupMessage.encode(config.publicHost, config.port, s.key(), s.transport(), new UUID(s.idMsb(), s.idLsb()));
    }

    /** El jugador ha cambiado de servidor: la ruta anterior ya no vale. */
    public void onServerConnected(UUID player, String server) {
        Route r = routes.get(player);
        // Si el setup del nuevo servidor llegó antes que este evento, no se toca.
        if (r != null && !r.server().equals(server) && routes.remove(player, r)) remove(r);
    }

    public void onDisconnect(UUID player) {
        Route r = routes.remove(player);
        if (r != null) remove(r);
        synchronized (this) {
            NetGroup g = netMember.get(player);
            if (g != null) leave(player, g, null);
        }
    }

    private void remove(Route r) {
        long h = handle;
        if (h != 0) FerrovozNative.proxyRemove(h, r.voiceId().getMostSignificantBits(), r.voiceId().getLeastSignificantBits());
    }

    // --- Grupos de red ---

    /** Las dos formas de cada subcomando (sirven con cualquiera de los dos comandos). */
    private static String sub(String arg) {
        return switch (arg.toLowerCase(Locale.ROOT)) {
            case "crear", "create" -> "create";
            case "unirse", "join" -> "join";
            case "salir", "leave" -> "leave";
            case "lista", "list" -> "list";
            case "expulsar", "kick" -> "kick";
            case "borrar", "delete" -> "delete";
            default -> "help";
        };
    }

    /** Subcomandos para autocompletar, en el idioma del comando. */
    public static List<String> subcommands(boolean spanish) {
        return spanish ? List.of("crear", "unirse", "salir", "lista", "expulsar", "borrar")
                : List.of("create", "join", "leave", "list", "kick", "delete");
    }

    /**
     * {@code /vozred ...} o {@code /voicenet ...}.
     *
     * @param sender quien lo ejecuta, o null para la consola
     */
    public synchronized void command(NetPlayer sender, String[] args) {
        String locale = sender != null ? sender.locale() : "en";
        java.util.function.Consumer<String> reply = sender != null ? sender::tell
                : msg -> FerrovozLog.LOG.info(msg.replaceAll("§.", ""));
        String action = args.length == 0 ? "help" : sub(args[0]);
        if (action.equals("help")) {
            reply.accept(tr(locale, "net.help1"));
            reply.accept(tr(locale, "net.help2"));
            return;
        }
        if (action.equals("list")) {
            if (netGroups.isEmpty()) {
                reply.accept(tr(locale, "net.list_empty"));
                return;
            }
            reply.accept(tr(locale, "net.list_header"));
            netGroups.values().stream().sorted((a, b) -> a.name.compareToIgnoreCase(b.name)).forEach(g ->
                    reply.accept(tr(locale, g.password != null ? "net.list_entry_locked" : "net.list_entry", g.name, g.members.size())));
            return;
        }
        if (sender == null) {
            reply.accept(tr(locale, "net.players_only"));
            return;
        }
        if (handle == 0) {
            sender.tell(tr(locale, "net.unavailable"));
            return;
        }
        UUID me = sender.id();
        switch (action) {
            case "create" -> {
                if (args.length < 2 || args.length > 3) {
                    sender.tell(tr(locale, "net.usage_create"));
                    return;
                }
                String name = args[1];
                if (!validName(name)) {
                    sender.tell(tr(locale, "net.invalid_name"));
                    return;
                }
                if (netGroups.containsKey(name.toLowerCase(Locale.ROOT))) {
                    sender.tell(tr(locale, "net.exists", name));
                    return;
                }
                NetGroup old = netMember.get(me);
                if (old != null) leave(me, old, null);
                NetGroup g = new NetGroup(nextNetGroup++, name, args.length == 3 ? args[2] : null, me);
                netGroups.put(name.toLowerCase(Locale.ROOT), g);
                g.members.add(me);
                netMember.put(me, g);
                pushGroup(g);
                sender.tell(tr(locale, g.password != null ? "net.created_locked" : "net.created", name));
            }
            case "join" -> {
                if (args.length < 2 || args.length > 3) {
                    sender.tell(tr(locale, "net.usage_join"));
                    return;
                }
                NetGroup g = netGroups.get(args[1].toLowerCase(Locale.ROOT));
                if (g == null) {
                    sender.tell(tr(locale, "net.not_found", args[1]));
                    return;
                }
                if (g.members.contains(me)) {
                    sender.tell(tr(locale, "net.already", g.name));
                    return;
                }
                if (g.password != null && (args.length < 3 || !g.password.equals(args[2])) && !sender.isAdmin()) {
                    sender.tell(tr(locale, "net.wrong_password"));
                    return;
                }
                NetGroup old = netMember.get(me);
                if (old != null) leave(me, old, null);
                g.members.add(me);
                netMember.put(me, g);
                pushGroup(g);
                sender.tell(tr(locale, "net.joined", g.name));
                notifyMembers(g, me, "net.member_joined", sender.name());
            }
            case "leave" -> {
                NetGroup g = netMember.get(me);
                if (g == null) {
                    sender.tell(tr(locale, "net.not_in_group"));
                    return;
                }
                leave(me, g, null);
                sender.tell(tr(locale, "net.left", g.name));
            }
            case "kick" -> {
                if (args.length != 2) {
                    sender.tell(tr(locale, "net.usage_kick"));
                    return;
                }
                NetPlayer target = players != null ? players.byName(args[1]) : null;
                NetGroup g = target != null ? netMember.get(target.id()) : null;
                if (g == null) {
                    sender.tell(tr(locale, "net.not_member", args[1]));
                    return;
                }
                if (!me.equals(g.owner) && !sender.isAdmin()) {
                    sender.tell(tr(locale, "net.not_owner"));
                    return;
                }
                leave(target.id(), g, sender.name());
                target.tell(tr(target.locale(), "net.you_were_kicked", g.name));
                sender.tell(tr(locale, "net.kicked", target.name(), g.name));
            }
            case "delete" -> {
                if (args.length != 2) {
                    sender.tell(tr(locale, "net.usage_delete"));
                    return;
                }
                NetGroup g = netGroups.get(args[1].toLowerCase(Locale.ROOT));
                if (g == null) {
                    sender.tell(tr(locale, "net.not_found", args[1]));
                    return;
                }
                if (!me.equals(g.owner) && !sender.isAdmin()) {
                    sender.tell(tr(locale, "net.not_owner"));
                    return;
                }
                deleteGroup(g);
                sender.tell(tr(locale, "net.deleted", g.name));
            }
            default -> sender.tell(tr(locale, "net.help1"));
        }
    }

    /** Nombre de grupo: 1-24 caracteres, sin espacios ni caracteres de control. */
    static boolean validName(String name) {
        return !name.isEmpty() && name.codePointCount(0, name.length()) <= 24
                && name.codePoints().noneMatch(c -> Character.isISOControl(c) || Character.isWhitespace(c) || c == '§');
    }

    /** Sale del grupo (si era el dueño, pasa al siguiente); si se vacía, se borra. */
    private void leave(UUID player, NetGroup g, String kickedBy) {
        g.members.remove(player);
        netMember.remove(player, g);
        if (g.members.isEmpty()) {
            netGroups.remove(g.name.toLowerCase(Locale.ROOT), g);
            pushDelete(g);
            return;
        }
        if (player.equals(g.owner)) g.owner = g.members.iterator().next();
        pushGroup(g);
        // Al expulsado se le avisa aparte.
        NetPlayer p = players != null ? players.byId(player) : null;
        String name = p != null ? p.name() : player.toString().substring(0, 8);
        notifyMembers(g, null, kickedBy != null ? "net.member_kicked" : "net.member_left", name);
    }

    private void deleteGroup(NetGroup g) {
        netGroups.remove(g.name.toLowerCase(Locale.ROOT), g);
        for (UUID m : g.members) {
            netMember.remove(m, g);
            NetPlayer p = players != null ? players.byId(m) : null;
            if (p != null) p.tell(tr(p.locale(), "net.deleted_notice", g.name));
        }
        g.members.clear();
        pushDelete(g);
    }

    private void notifyMembers(NetGroup g, UUID except, String key, Object... args) {
        if (players == null) return;
        for (UUID m : g.members) {
            if (m.equals(except)) continue;
            NetPlayer p = players.byId(m);
            if (p != null) p.tell(tr(p.locale(), key, args));
        }
    }

    /** Envía al reenviador los miembros del grupo (UUID de voz del servidor actual). */
    private void pushGroup(NetGroup g) {
        long h = handle;
        if (h == 0) return;
        List<long[]> ids = new ArrayList<>();
        List<String> names = new ArrayList<>();
        for (UUID m : g.members) {
            Route r = routes.get(m);
            if (r == null) continue; // sin voz todavía, o en un servidor sin Ferrovoz
            ids.add(new long[] {r.voiceId().getMostSignificantBits(), r.voiceId().getLeastSignificantBits()});
            NetPlayer p = players != null ? players.byId(m) : null;
            names.add(p != null ? p.name() : "?");
        }
        long[] flat = new long[ids.size() * 2];
        for (int i = 0; i < ids.size(); i++) {
            flat[2 * i] = ids.get(i)[0];
            flat[2 * i + 1] = ids.get(i)[1];
        }
        Route owner = g.owner != null ? routes.get(g.owner) : null;
        // Sin nadie con voz se envía vacío y el reenviador lo olvida hasta que lleguen.
        FerrovozNative.proxySetNetGroup(h, g.id, g.name,
                owner != null ? owner.voiceId().getMostSignificantBits() : 0,
                owner != null ? owner.voiceId().getLeastSignificantBits() : 0,
                flat, names.toArray(new String[0]));
    }

    private void pushDelete(NetGroup g) {
        long h = handle;
        if (h != 0) FerrovozNative.proxySetNetGroup(h, g.id, "", 0, 0, new long[0], new String[0]);
    }

    private static String tr(String locale, String key, Object... args) {
        return dev.ferrovoz.Messages.prefixed(locale, key, args);
    }

    /** Grupo de red de un jugador, o null. */
    public String netGroupOf(UUID player) {
        NetGroup g = netMember.get(player);
        return g != null ? g.name : null;
    }

    /** {@code [rutas, subidos, bajados, rechazados, replay, voz de grupos de red]} o null. */
    public long[] status() {
        long h = handle;
        return h == 0 ? null : FerrovozNative.proxyStatus(h);
    }
}
