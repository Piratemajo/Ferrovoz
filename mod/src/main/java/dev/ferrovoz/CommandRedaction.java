package dev.ferrovoz;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Tapa la contraseña de {@code /voz grupo crear|unirse <nombre> <contraseña>} en una línea
 * de log. Paper apunta en la consola cada comando de jugador tal cual
 * ("Pepe issued server command: ..."), así que sin esto las contraseñas acaban en
 * {@code latest.log}. Vale para los alias en inglés y para {@code /ferrovoz:voz}.
 */
public final class CommandRedaction {
    /** Lo que se pone en lugar de la contraseña. */
    public static final String HIDDEN = "[hidden]";

    private static final Pattern WITH_PASSWORD = Pattern.compile(
            "(?i)^(.*\\bissued server command: /(?:ferrovoz:)?(?:voz|voice)\\s+(?:grupo|group)\\s+"
                    + "(?:crear|create|unirse|join)\\s+\\S+\\s+)(\\S.*)$",
            Pattern.DOTALL);

    private CommandRedaction() {}

    /** La línea con la contraseña tapada, o null si no lleva ninguna (o ya está tapada). */
    public static String redact(String line) {
        if (line == null || line.indexOf("issued server command") < 0) return null;
        Matcher m = WITH_PASSWORD.matcher(line);
        if (!m.matches() || m.group(2).equals(HIDDEN)) return null;
        return m.group(1) + HIDDEN;
    }
}
