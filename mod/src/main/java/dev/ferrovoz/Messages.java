package dev.ferrovoz;

import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.Locale;
import java.util.Map;
import java.util.Properties;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Mensajes del servidor en el idioma de cada jugador, desde
 * {@code /ferrovoz/lang/messages_<idioma>.properties} (UTF-8). Cualquier variante de
 * un idioma usa el archivo base ({@code es_mx} -> {@code es}); si no hay, inglés.
 */
public final class Messages {
    public static final String FALLBACK = "en";
    private static final Map<String, Properties> BUNDLES = new ConcurrentHashMap<>();
    private static final Properties MISSING = new Properties();

    private Messages() {}

    /** {@code "es_ES"}, {@code "es-mx"}, {@code "es"} -> {@code "es"}; null o vacío -> inglés. */
    public static String language(String locale) {
        if (locale == null || locale.isBlank()) return FALLBACK;
        String l = locale.toLowerCase(Locale.ROOT);
        int cut = l.indexOf('_') >= 0 ? l.indexOf('_') : l.indexOf('-');
        return cut > 0 ? l.substring(0, cut) : l;
    }

    /** Texto traducido con {@code {0}}, {@code {1}}... sustituidos. */
    public static String tr(String locale, String key, Object... args) {
        String text = lookup(language(locale), key);
        if (text == null) text = lookup(FALLBACK, key);
        if (text == null) return key;
        for (int i = 0; i < args.length; i++) {
            text = text.replace("{" + i + "}", String.valueOf(args[i]));
        }
        return text;
    }

    /** Prefijo "[Voz]" seguido del mensaje. */
    public static String prefixed(String locale, String key, Object... args) {
        return tr(locale, "prefix") + " " + tr(locale, key, args);
    }

    /** ¿Hay traducción propia para este idioma? */
    public static boolean has(String locale) {
        return bundle(language(locale)) != MISSING;
    }

    private static String lookup(String lang, String key) {
        return bundle(lang).getProperty(key);
    }

    private static Properties bundle(String lang) {
        if (!lang.matches("[a-z]{2,3}")) return MISSING;
        return BUNDLES.computeIfAbsent(lang, l -> {
            try (InputStream in = Messages.class.getResourceAsStream("/ferrovoz/lang/messages_" + l + ".properties")) {
                if (in == null) return MISSING;
                Properties p = new Properties();
                p.load(new InputStreamReader(in, StandardCharsets.UTF_8));
                return p;
            } catch (IOException e) {
                return MISSING;
            }
        });
    }
}
