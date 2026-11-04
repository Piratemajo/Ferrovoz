package dev.ferrovoz;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.Collection;

/**
 * Canal servidor -> cliente, compartido por el mod y el plugin. Sin dependencias de
 * Minecraft.
 *
 * <p>Formato del mensaje (codificación estándar de Minecraft, big endian):
 * <pre>
 *   VarInt longitud + UTF-8   host       (vacío = el mismo que el del juego)
 *   VarInt                    puerto UDP
 *   VarInt longitud + bytes   clave de 32 bytes
 *   VarInt                    transporte: 0 = automático, 1 = solo UDP, 2 = solo túnel
 *   long                      UUID, mitad alta
 *   long                      UUID, mitad baja
 * </pre>
 *
 * <p>{@link #TUNNEL}: en ambos sentidos, VarInt longitud + bytes de un paquete de
 * voz cifrado. Se usa cuando el puerto UDP no está disponible.
 *
 * <p>{@link #VERSION_CHANNEL} (servidor -> cliente, desde 0.6): nombre y formato
 * fijos, para que cualquier versión pueda decirle a otra cuál es:
 * <pre>
 *   VarInt                    protocolo del servidor ({@link #VERSION})
 *   VarInt longitud + UTF-8   versión del mod ("0.6.0")
 *   boolean                   el relé está en marcha
 * </pre>
 * Los clientes anteriores a 0.6 no lo conocen; el servidor los detecta por el número
 * de los canales que registran ({@code setup_vN}) y les avisa por chat.
 */
public final class Protocol {
    /** Subir cuando cambie el formato. Va en el nombre del canal. */
    public static final int VERSION = 4;
    public static final String NAMESPACE = "ferrovoz";
    public static final String PATH = "setup_v" + VERSION;
    public static final String CHANNEL = NAMESPACE + ":" + PATH;
    public static final String TUNNEL_PATH = "voice_v" + VERSION;
    public static final String TUNNEL = NAMESPACE + ":" + TUNNEL_PATH;
    /** No cambia nunca (ver arriba). */
    public static final String VERSION_PATH = "version";
    public static final String VERSION_CHANNEL = NAMESPACE + ":" + VERSION_PATH;

    public static final int TRANSPORT_AUTO = 0;
    public static final int TRANSPORT_UDP = 1;
    public static final int TRANSPORT_TUNNEL = 2;

    private static volatile String modVersion;

    private Protocol() {}

    /** Versión del mod/plugin ({@code 0.6.0}). */
    public static String modVersion() {
        String v = modVersion;
        if (v == null) {
            v = "?";
            try (InputStream in = Protocol.class.getResourceAsStream("/ferrovoz/version.txt")) {
                if (in != null) v = new String(in.readAllBytes(), StandardCharsets.UTF_8).trim();
            } catch (IOException ignored) {
                // se queda en "?"
            }
            modVersion = v;
        }
        return v;
    }

    /**
     * Protocolo de un canal de Ferrovoz ({@code ferrovoz:setup_v3} -> 3,
     * {@code ferrovoz:voice_v4} -> 4), o -1 si no lo es.
     */
    public static int protocolOfChannel(String channel) {
        if (channel == null || !channel.startsWith(NAMESPACE + ":")) return -1;
        String path = channel.substring(NAMESPACE.length() + 1);
        for (String prefix : new String[] {"setup_v", "voice_v"}) {
            if (path.startsWith(prefix)) {
                try {
                    return Integer.parseInt(path.substring(prefix.length()));
                } catch (NumberFormatException e) {
                    return -1;
                }
            }
        }
        return -1;
    }

    /** Protocolo más alto que anuncian estos canales, o -1 si ninguno es de Ferrovoz. */
    public static int protocolOf(Collection<String> channels) {
        int best = -1;
        for (String c : channels) best = Math.max(best, protocolOfChannel(c));
        return best;
    }

    /** Qué hacer con un jugador según los canales de su cliente. */
    public enum ClientKind {
        /** Mismo protocolo. */
        COMPATIBLE,
        /** 0.6 o posterior con otro protocolo: se le envía la versión y su cliente lo explica. */
        HANDSHAKE,
        /** Anterior a 0.6 con otro protocolo: aviso por chat desde el servidor. */
        LEGACY_OLDER,
        LEGACY_NEWER,
        /** No tiene Ferrovoz. */
        NONE
    }

    public static ClientKind classify(Collection<String> clientChannels) {
        if (clientChannels.contains(CHANNEL)) return ClientKind.COMPATIBLE;
        if (clientChannels.contains(VERSION_CHANNEL)) return ClientKind.HANDSHAKE;
        int p = protocolOf(clientChannels);
        if (p < 0) return ClientKind.NONE;
        return p < VERSION ? ClientKind.LEGACY_OLDER : ClientKind.LEGACY_NEWER;
    }

    /** Mensaje (en el idioma del jugador) para un cliente antiguo con otro protocolo, o null. */
    public static String legacyNotice(ClientKind kind, String locale) {
        return switch (kind) {
            case LEGACY_OLDER -> Messages.prefixed(locale, "version.client_older", modVersion());
            case LEGACY_NEWER -> Messages.prefixed(locale, "version.client_newer", modVersion());
            default -> null;
        };
    }
}
