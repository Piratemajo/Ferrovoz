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
     * Por defecto {@code ghost}, para que los espectadores no molesten.
     */
    public int spectators = RULE_GHOST;
    public int dead = RULE_NORMAL;
    /**
     * Voces por separado como mucho en cada zona de 8 bloques. Si a una zona le
     * llegan más, las más lejanas se mezclan en el servidor en un murmullo de fondo.
     * 0 = sin límite.
     */
    public int maxVoices = 24;
    /**
     * Eco en cuevas y salas. Con {@code false} no lo oye nadie, aunque lo tenga
     * activado en sus ajustes.
     */
    public boolean caveEcho = true;
    /**
     * Efecto bajo el agua (voz tapada si alguien está sumergido). Con {@code false} no lo
     * oye nadie, aunque lo tenga activado.
     */
    public boolean underwaterEffect = true;

    public static final int RULE_NORMAL = 0, RULE_LISTEN = 1, RULE_GHOST = 2, RULE_OWN = 3, RULE_OFF = 4;
    private static final String[] RULE_NAMES = {"normal", "listen", "ghost", "own", "off"};

    public static FerrovozConfig load(Path file) {
        FerrovozConfig c = new FerrovozConfig();
        Properties p = new Properties();
        // Con algún valor mal el archivo no se reescribe, para no perder el resto.
        boolean ok = true;
        if (Files.exists(file)) {
            try (Reader r = Files.newBufferedReader(file)) {
                p.load(r);
            } catch (IOException | IllegalArgumentException e) {
                FerrovozLog.LOG.warn("Could not read {}, using defaults (the file is left untouched)", file, e);
                return c;
            }
            c.bindHost = p.getProperty("bind_host", c.bindHost).trim();
            c.publicHost = p.getProperty("public_host", c.publicHost).trim();
            Integer port = parseInt(p, "port", c.port, 1, 65535);
            Float max = parseFloat(p, "max_distance", c.maxDistance);
            Float whisper = parseFloat(p, "whisper_distance", c.whisperDistance);
            Float shout = parseFloat(p, "shout_distance", c.shoutDistance);
            Integer maxVoices = parseInt(p, "max_voices", c.maxVoices, 0, 256);
            ok = port != null && max != null && whisper != null && shout != null && maxVoices != null;
            if (maxVoices != null) c.maxVoices = maxVoices;
            Boolean caveEcho = parseBool(p, "cave_echo", c.caveEcho);
            ok &= caveEcho != null;
            if (caveEcho != null) c.caveEcho = caveEcho;
            Boolean underwater = parseBool(p, "underwater_effect", c.underwaterEffect);
            ok &= underwater != null;
            if (underwater != null) c.underwaterEffect = underwater;
            if (port != null) c.port = port;
            if (max != null) c.maxDistance = max;
            if (whisper != null) c.whisperDistance = whisper;
            if (shout != null) c.shoutDistance = shout;
            c.transport = parseTransport(p.getProperty("transport", "auto"));
            Integer spectators = parseRuleOrNull(p.getProperty("spectators", "ghost"));
            Integer dead = parseRuleOrNull(p.getProperty("dead", "normal"));
            ok &= spectators != null && dead != null;
            c.spectators = spectators != null ? spectators : RULE_GHOST;
            c.dead = dead != null ? dead : RULE_NORMAL;
        }
        if (ok) {
            c.save(file); // crea el archivo o añade las claves nuevas
        } else {
            FerrovozLog.LOG.warn("{} has invalid values (see above); they use their defaults and the file is left untouched", file);
        }
        return c;
    }

    /** Número entero en [min, max], o null (con aviso) si no lo es. */
    static Integer parseInt(Properties p, String key, int def, int min, int max) {
        String raw = p.getProperty(key);
        if (raw == null) return def;
        try {
            int v = Integer.parseInt(raw.trim());
            if (v >= min && v <= max) return v;
        } catch (NumberFormatException ignored) {
            // se avisa abajo
        }
        FerrovozLog.LOG.warn("ferrovoz.properties: {}='{}' is not valid (expected {}-{}); using {}", key, raw, min, max, def);
        return null;
    }

    /** {@code true} o {@code false}; null (con aviso) si no vale. */
    static Boolean parseBool(Properties p, String key, boolean def) {
        String raw = p.getProperty(key);
        if (raw == null) return def;
        String v = raw.trim().toLowerCase(java.util.Locale.ROOT);
        if (v.equals("true")) return true;
        if (v.equals("false")) return false;
        FerrovozLog.LOG.warn("ferrovoz.properties: {}='{}' is not valid (expected true or false); using {}", key, raw, def);
        return null;
    }

    /** Distancia en bloques (&gt; 0); admite coma decimal ("48,5"). null (con aviso) si no vale. */
    static Float parseFloat(Properties p, String key, float def) {
        String raw = p.getProperty(key);
        if (raw == null) return def;
        try {
            float v = Float.parseFloat(raw.trim().replace(',', '.'));
            if (Float.isFinite(v) && v > 0f && v <= 10_000f) return v;
        } catch (NumberFormatException ignored) {
            // se avisa abajo
        }
        FerrovozLog.LOG.warn("ferrovoz.properties: {}='{}' is not a valid distance in blocks; using {}", key, raw, def);
        return null;
    }

    static int parseTransport(String s) {
        return switch (s.trim().toLowerCase(Locale.ROOT)) {
            case "udp" -> Protocol.TRANSPORT_UDP;
            case "tunnel", "tunel", "túnel" -> Protocol.TRANSPORT_TUNNEL;
            default -> Protocol.TRANSPORT_AUTO;
        };
    }

    static int parseRule(String s) {
        Integer r = parseRuleOrNull(s);
        return r != null ? r : RULE_NORMAL;
    }

    /** Regla por nombre, o null (con aviso) si no existe. */
    static Integer parseRuleOrNull(String s) {
        String v = s.trim().toLowerCase(Locale.ROOT);
        for (int i = 0; i < RULE_NAMES.length; i++) {
            if (RULE_NAMES[i].equals(v)) return i;
        }
        FerrovozLog.LOG.warn("ferrovoz.properties: unknown rule '{}' (use normal, listen, ghost, own or off); using the default", s);
        return null;
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
        p.setProperty("max_voices", String.valueOf(maxVoices));
        p.setProperty("cave_echo", String.valueOf(caveEcho));
        p.setProperty("underwater_effect", String.valueOf(underwaterEffect));
        p.setProperty("spectators", ruleName(spectators));
        p.setProperty("dead", ruleName(dead));
        try {
            Files.createDirectories(file.getParent());
            try (Writer w = Files.newBufferedWriter(file)) {
                p.store(w, "Ferrovoz - proximity voice chat. port = UDP port; transport = auto | udp | tunnel\n"
                        + " spectators / dead = normal (like anyone) | listen (hear, can't be heard)"
                        + " | ghost (talk among themselves, hear the living) | own (only among themselves) | off\n"
                        + " max_voices = voices heard separately per area in a crowd; the rest is mixed into"
                        + " a background murmur (0 = no limit)\n"
                        + " cave_echo = echo in caves and rooms for everyone (false turns it off even if"
                        + " players have it on)\n"
                        + " underwater_effect = muffled voice underwater for everyone (false turns it off even"
                        + " if players have it on)");
            }
        } catch (IOException e) {
            FerrovozLog.LOG.warn("Could not save ferrovoz.properties", e);
        }
    }
}
