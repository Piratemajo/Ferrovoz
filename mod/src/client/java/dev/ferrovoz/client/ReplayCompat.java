package dev.ferrovoz.client;

import dev.ferrovoz.Ferrovoz;
import net.minecraft.network.ConnectionProtocol;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.common.ClientboundCustomPayloadPacket;

import java.lang.reflect.Field;
import java.lang.reflect.Method;

/**
 * Integración con Flashback y Replay Mod para grabar la voz en las repeticiones.
 * Por reflexión: sin dependencia en tiempo de compilación, y si cambian sus clases
 * internas solo se desactiva (con aviso en el log).
 *
 * <ul>
 *   <li>Flashback: {@code Flashback.RECORDER.writePacketAsync(paquete, PLAY)} y
 *       {@code Flashback.isInReplay()}.</li>
 *   <li>Replay Mod: {@code ReplayModRecording.instance.getConnectionEventHandler()
 *       .getPacketListener().save(paquete)} y
 *       {@code ReplayModReplay.instance.getReplayHandler() != null}.</li>
 * </ul>
 */
final class ReplayCompat {
    private ReplayCompat() {}

    // --- Flashback ---
    private static boolean flashbackOff;
    private static Field fbRecorder;
    private static Method fbReady, fbWrite, fbInReplay;

    // --- Replay Mod ---
    private static boolean replayModOff;
    private static Object rmRecording, rmReplay;
    private static Method rmHandler, rmListener, rmGetReplayHandler;
    private static Method rmSave;

    private static boolean flashback() {
        if (flashbackOff) return false;
        if (fbRecorder != null) return true;
        try {
            Class<?> fb = Class.forName("com.moulberry.flashback.Flashback");
            fbRecorder = fb.getField("RECORDER");
            fbInReplay = fb.getMethod("isInReplay");
            Class<?> recorder = fbRecorder.getType();
            fbReady = recorder.getMethod("readyToWrite");
            fbWrite = recorder.getMethod("writePacketAsync", Packet.class, ConnectionProtocol.class);
            Ferrovoz.LOG.info("Ferrovoz: Flashback found, voice will be saved in its replays");
            return true;
        } catch (ClassNotFoundException e) {
            flashbackOff = true; // no instalado
        } catch (ReflectiveOperationException | LinkageError e) {
            disableFlashback(e);
        }
        return false;
    }

    private static boolean replayMod() {
        if (replayModOff) return false;
        if (rmHandler != null) return true;
        try {
            Class<?> recording = Class.forName("com.replaymod.recording.ReplayModRecording");
            rmRecording = recording.getField("instance").get(null);
            rmHandler = recording.getMethod("getConnectionEventHandler");
            rmListener = rmHandler.getReturnType().getMethod("getPacketListener");
            for (Method m : rmListener.getReturnType().getMethods()) {
                if (m.getName().equals("save") && m.getParameterCount() == 1
                        && m.getParameterTypes()[0].isAssignableFrom(ClientboundCustomPayloadPacket.class)) {
                    rmSave = m;
                }
            }
            if (rmSave == null) throw new NoSuchMethodException("PacketListener.save(Packet)");
            Class<?> replay = Class.forName("com.replaymod.replay.ReplayModReplay");
            rmReplay = replay.getField("instance").get(null);
            rmGetReplayHandler = replay.getMethod("getReplayHandler");
            Ferrovoz.LOG.info("Ferrovoz: Replay Mod found, voice will be saved in its replays");
            return true;
        } catch (ClassNotFoundException e) {
            replayModOff = true;
        } catch (ReflectiveOperationException | LinkageError e) {
            disableReplayMod(e);
        }
        return false;
    }

    private static void disableFlashback(Throwable e) {
        flashbackOff = true;
        fbRecorder = null;
        Ferrovoz.LOG.warn("Ferrovoz: could not hook into this Flashback version; voice won't be saved in its replays", e);
    }

    private static void disableReplayMod(Throwable e) {
        replayModOff = true;
        rmHandler = null;
        Ferrovoz.LOG.warn("Ferrovoz: could not hook into this Replay Mod version; voice won't be saved in its replays", e);
    }

    /** Grabador de Flashback, si está grabando. */
    private static Object flashbackRecorder() {
        if (!flashback()) return null;
        try {
            Object r = fbRecorder.get(null);
            return r != null && (boolean) fbReady.invoke(r) ? r : null;
        } catch (ReflectiveOperationException | RuntimeException e) {
            disableFlashback(e);
            return null;
        }
    }

    /** Grabador de Replay Mod, si está grabando. */
    private static Object replayModListener() {
        if (!replayMod() || rmRecording == null) return null;
        try {
            Object handler = rmHandler.invoke(rmRecording);
            return handler != null ? rmListener.invoke(handler) : null;
        } catch (ReflectiveOperationException | RuntimeException e) {
            disableReplayMod(e);
            return null;
        }
    }

    /** ¿Graba alguno? */
    static boolean recording() {
        return flashbackRecorder() != null || replayModListener() != null;
    }

    /** Añade un paquete a las grabaciones en curso como si viniera del servidor. Hilo del juego. */
    static void record(Packet<?> packet) {
        Object fb = flashbackRecorder();
        if (fb != null) {
            try {
                fbWrite.invoke(fb, packet, ConnectionProtocol.PLAY);
            } catch (ReflectiveOperationException | RuntimeException e) {
                disableFlashback(e);
            }
        }
        Object rm = replayModListener();
        if (rm != null) {
            try {
                rmSave.invoke(rm, packet);
            } catch (ReflectiveOperationException | RuntimeException e) {
                disableReplayMod(e);
            }
        }
    }

    /** ¿Reproduciendo una repetición? (no hay relé al que conectar) */
    static boolean inReplay() {
        if (flashback()) {
            try {
                if ((boolean) fbInReplay.invoke(null)) return true;
            } catch (ReflectiveOperationException | RuntimeException e) {
                disableFlashback(e);
            }
        }
        if (replayMod() && rmReplay != null) {
            try {
                return rmGetReplayHandler.invoke(rmReplay) != null;
            } catch (ReflectiveOperationException | RuntimeException e) {
                disableReplayMod(e);
            }
        }
        return false;
    }
}
