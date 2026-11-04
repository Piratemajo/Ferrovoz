package dev.ferrovoz;

import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Properties;

/**
 * Configuración del servidor: {@code config/ferrovoz.properties} (mod) o
 * {@code plugins/Ferrovoz/ferrovoz.properties} (plugin).
 */
public final class FerrovozConfig {
    /** Interfaz del UDP de voz (vacío = todas). */
    public String bindHost = "";
    /** Puerto UDP (hay que abrirlo en el firewall). */
    public int port = 24454;
    /** Host anunciado a los clientes (vacío = el que usan para entrar al juego). */
    public String publicHost = "";
    /** Alcance de la voz normal en bloques. */
    public float maxDistance = 48f;
    /** Alcance al susurrar. */
    public float whisperDistance = 8f;
    /** Alcance al gritar. */
    public float shoutDistance = 96f;
    /**
     * {@link Protocol#TRANSPORT_AUTO}: UDP y, si no llega, túnel.
     * {@link Protocol#TRANSPORT_UDP}: solo UDP. {@link Protocol#TRANSPORT_TUNNEL}: siempre
     * túnel (hosts sin puertos; algo más de latencia).
     */
    public int transport = Protocol.TRANSPORT_AUTO;
    /**
     * Espectadores y muertos: {@code normal}, {@code listen} (oyen, no se les oye),
     * {@code ghost} (hablan entre ellos y oyen a los vivos, que no les oyen), {@code own}
     * (solo entre ellos) u {@code off}.
     */
    /** Por defecto {@code ghost}, para que los espectadores no molesten. */
    public int spectators = RULE_GHOST;
    public int dead = RULE_NORMAL;

    public static final int RULE_NORMAL = 0, RULE_LISTEN = 1, RULE_GHOST = 2, RULE_OWN = 3, RULE_OFF = 4;
    private static final String[] RULE_NAMES = {"normal", "listen", "ghost", "own", "off"};

    public static FerrovozConfig load(Path file) {
        FerrovozConfig c = new FerrovozConfig();
        Properties p = new Properties();
        if (Files.exists(file)) {
            try (Reader r = Files.newBufferedReader(file)) {
                p.load(r);
                c.bindHost = p.getProperty("bind_host", c.bindHost).trim();
                c.port = Integer.parseInt(p.getProperty("port", String.valueOf(c.port)).trim());
                c.publicHost = p.getProperty("public_host", c.publicHost).trim();
                c.maxDistance = Float.parseFloat(p.getProperty("max_distance", String.valueOf(c.maxDistance)).trim());
                c.whisperDistance = Float.parseFloat(p.getProperty("whisper_distance", String.valueOf(c.whisperDistance)).trim());
                c.shoutDistance = Float.parseFloat(p.getProperty("shout_distance", String.valueOf(c.shoutDistance)).trim());
                c.transport = parseTransport(p.getProperty("transport", "auto"));
                c.spectators = parseRule(p.getProperty("spectators", "ghost"));
                c.dead = parseRule(p.getProperty("dead", "normal"));
            } catch (IOException | NumberFormatException e) {
                FerrovozLog.LOG.warn("ferrovoz.properties is invalid, using defaults", e);
            }
        }
        c.save(file);
        return c;
    }

    static int parseTransport(String s) {
        return switch (s.trim().toLowerCase(Locale.ROOT)) {
            case "udp" -> Protocol.TRANSPORT_UDP;
            case "tunnel", "tunel", "túnel" -> Protocol.TRANSPORT_TUNNEL;
            default -> Protocol.TRANSPORT_AUTO;
        };
    }

    static int parseRule(String s) {
        String v = s.trim().toLowerCase(Locale.ROOT);
        for (int i = 0; i < RULE_NAMES.length; i++) {
            if (RULE_NAMES[i].equals(v)) return i;
        }
        FerrovozLog.LOG.warn("ferrovoz.properties: unknown rule '{}' (use normal, listen, ghost, own or off); using normal", s);
        return RULE_NORMAL;
    }

    public static String ruleName(int rule) {
        return rule >= 0 && rule < RULE_NAMES.length ? RULE_NAMES[rule] : "normal";
    }

    public String transportName() {
        return switch (transport) {
            case Protocol.TRANSPORT_UDP -> "udp";
            case Protocol.TRANSPORT_TUNNEL -> "tunnel";
            default -> "auto";
        };
    }

    private void save(Path file) {
        Properties p = new Properties();
        p.setProperty("bind_host", bindHost);
        p.setProperty("port", String.valueOf(port));
        p.setProperty("public_host", publicHost);
        p.setProperty("max_distance", String.valueOf(maxDistance));
        p.setProperty("whisper_distance", String.valueOf(whisperDistance));
        p.setProperty("shout_distance", String.valueOf(shoutDistance));
        p.setProperty("transport", transportName());
        p.setProperty("spectators", ruleName(spectators));
        p.setProperty("dead", ruleName(dead));
        try {
            Files.createDirectories(file.getParent());
            try (Writer w = Files.newBufferedWriter(file)) {
                p.store(w, "Ferrovoz - proximity voice chat. port = UDP port; transport = auto | udp | tunnel\n"
                        + " spectators / dead = normal (like anyone) | listen (hear, can't be heard)"
                        + " | ghost (talk among themselves, hear the living) | own (only among themselves) | off");
            }
        } catch (IOException e) {
            FerrovozLog.LOG.warn("Could not save ferrovoz.properties", e);
        }
    }
}
