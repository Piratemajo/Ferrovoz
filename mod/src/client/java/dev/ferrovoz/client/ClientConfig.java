package dev.ferrovoz.client;

import dev.ferrovoz.Ferrovoz;
import dev.ferrovoz.FerrovozNative;

import java.io.IOException;
import java.io.Reader;
import java.io.Writer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.Properties;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/** Ajustes de voz del jugador, en {@code config/ferrovoz-client.properties}. */
final class ClientConfig {
    private static Path FILE;

    /** Micro; vacío = predeterminado. */
    String inputDevice = "";
    /** Salida; vacío = predeterminado. */
    String outputDevice = "";
    /** Volumen de las voces de los demás (1.0 = 100 %). */
    float volume = 1.0f;
    /** Volumen de la voz de grupo (1.0 = 100 %). */
    float groupVolume = 1.0f;
    /** Ganancia del micro (1.0 = 100 %). */
    float micGain = 1.0f;
    /** RMS a partir del cual transmite la activación por voz. */
    float vadThreshold = 0.02f;
    /** 0 = pulsar para hablar, 1 = activación por voz. */
    int mode = 0;
    boolean noiseSuppression = true;
    /** Reverb en cuevas y salas. */
    boolean caveEcho = true;
    /** Efecto submarino. */
    boolean underwaterEffect = true;
    /** AEC (para altavoces). */
    boolean echoCancel = true;
    /** AGC. */
    boolean autoGain = true;
    /** HRTF. Con altavoces es mejor el paneo simple. */
    boolean hrtf = true;
    /** Voz desactivada: ni oír ni ser oído. */
    boolean voiceDisabled = false;
    /** Volumen por jugador (solo los distintos de 100 %). */
    final Map<UUID, Float> playerVolumes = new ConcurrentHashMap<>();
    private static final String PLAYER_PREFIX = "player_volume.";

    /** @param configDir carpeta {@code config/} del juego */
    static ClientConfig load(Path configDir) {
        FILE = configDir.resolve("ferrovoz-client.properties");
        ClientConfig c = new ClientConfig();
        if (Files.exists(FILE)) {
            Properties p = new Properties();
            try (Reader r = Files.newBufferedReader(FILE)) {
                p.load(r);
                c.inputDevice = p.getProperty("input_device", c.inputDevice);
                c.outputDevice = p.getProperty("output_device", c.outputDevice);
                c.volume = Float.parseFloat(p.getProperty("volume", String.valueOf(c.volume)));
                c.groupVolume = Float.parseFloat(p.getProperty("group_volume", String.valueOf(c.groupVolume)));
                c.micGain = Float.parseFloat(p.getProperty("mic_gain", String.valueOf(c.micGain)));
                c.vadThreshold = Float.parseFloat(p.getProperty("vad_threshold", String.valueOf(c.vadThreshold)));
                c.mode = Integer.parseInt(p.getProperty("mode", String.valueOf(c.mode)));
                c.noiseSuppression = Boolean.parseBoolean(p.getProperty("noise_suppression", String.valueOf(c.noiseSuppression)));
                c.caveEcho = Boolean.parseBoolean(p.getProperty("cave_echo", String.valueOf(c.caveEcho)));
                c.underwaterEffect = Boolean.parseBoolean(p.getProperty("underwater_effect", String.valueOf(c.underwaterEffect)));
                c.echoCancel = Boolean.parseBoolean(p.getProperty("echo_cancel", String.valueOf(c.echoCancel)));
                c.autoGain = Boolean.parseBoolean(p.getProperty("auto_gain", String.valueOf(c.autoGain)));
                c.hrtf = Boolean.parseBoolean(p.getProperty("hrtf", String.valueOf(c.hrtf)));
                c.voiceDisabled = Boolean.parseBoolean(p.getProperty("voice_disabled", String.valueOf(c.voiceDisabled)));
                for (String k : p.stringPropertyNames()) {
                    if (!k.startsWith(PLAYER_PREFIX)) continue;
                    try {
                        c.playerVolumes.put(UUID.fromString(k.substring(PLAYER_PREFIX.length())), Float.parseFloat(p.getProperty(k)));
                    } catch (IllegalArgumentException ignored) {
                        // entrada corrupta: se ignora
                    }
                }
            } catch (IOException | NumberFormatException e) {
                Ferrovoz.LOG.warn("config/ferrovoz-client.properties is invalid, using defaults", e);
            }
        }
        return c;
    }

    void save() {
        Properties p = new Properties();
        p.setProperty("input_device", inputDevice);
        p.setProperty("output_device", outputDevice);
        p.setProperty("volume", String.valueOf(volume));
        p.setProperty("mic_gain", String.valueOf(micGain));
        p.setProperty("group_volume", String.valueOf(groupVolume));
        p.setProperty("vad_threshold", String.valueOf(vadThreshold));
        p.setProperty("mode", String.valueOf(mode));
        p.setProperty("noise_suppression", String.valueOf(noiseSuppression));
        p.setProperty("cave_echo", String.valueOf(caveEcho));
        p.setProperty("underwater_effect", String.valueOf(underwaterEffect));
        p.setProperty("echo_cancel", String.valueOf(echoCancel));
        p.setProperty("auto_gain", String.valueOf(autoGain));
        p.setProperty("hrtf", String.valueOf(hrtf));
        p.setProperty("voice_disabled", String.valueOf(voiceDisabled));
        playerVolumes.forEach((id, v) -> p.setProperty(PLAYER_PREFIX + id, String.valueOf(v)));
        try {
            Files.createDirectories(FILE.getParent());
            try (Writer w = Files.newBufferedWriter(FILE)) {
                p.store(w, "Ferrovoz - client voice settings");
            }
        } catch (IOException e) {
            Ferrovoz.LOG.warn("Could not save config/ferrovoz-client.properties", e);
        }
    }

    /** Aplica los ajustes a un cliente (conectado o de prueba). */
    void applyTo(long handle) {
        if (handle == 0) return;
        FerrovozNative.clientSetDevices(handle, inputDevice, outputDevice);
        FerrovozNative.clientSetVolume(handle, volume);
        FerrovozNative.clientSetMicGain(handle, micGain);
        FerrovozNative.clientSetGroupVolume(handle, groupVolume);
        FerrovozNative.clientSetVadThreshold(handle, vadThreshold);
        FerrovozNative.clientSetMode(handle, mode);
        FerrovozNative.clientSetNoiseSuppression(handle, noiseSuppression);
        FerrovozNative.clientSetCaveEcho(handle, caveEcho);
        FerrovozNative.clientSetUnderwaterEffect(handle, underwaterEffect);
        FerrovozNative.clientSetMicProcessing(handle, echoCancel, autoGain);
        FerrovozNative.clientSetHrtf(handle, hrtf);
        playerVolumes.forEach((id, v) ->
                FerrovozNative.clientSetPlayerVolume(handle, id.getMostSignificantBits(), id.getLeastSignificantBits(), v));
    }

    float volumeOf(UUID id) {
        return playerVolumes.getOrDefault(id, 1f);
    }

    void setVolume(UUID id, float v) {
        if (Math.abs(v - 1f) < 0.001f) playerVolumes.remove(id);
        else playerVolumes.put(id, v);
    }
}
