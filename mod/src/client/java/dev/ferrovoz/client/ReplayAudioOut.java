package dev.ferrovoz.client;

import org.lwjgl.openal.AL10;
import org.lwjgl.openal.ALC10;
import org.lwjgl.system.MemoryUtil;

import java.nio.ShortBuffer;
import java.util.ArrayDeque;

/**
 * Salida de audio al reproducir una repetición, por OpenAL (el motor de sonido de
 * Minecraft) en vez de directamente a la tarjeta. Así Flashback la incluye al exportar
 * vídeo y va al ritmo de la repetición.
 *
 * <p>Una fuente estéreo sin posición (el 3D ya viene hecho) con buffers de 50 ms,
 * uno por tick.
 */
final class ReplayAudioOut {
    private static final int RATE = 48_000;
    /** Más que esto en cola es retraso: se descarta. */
    private static final int MAX_QUEUED = 8;

    private long context;
    private int source;
    private int queued;
    private final ArrayDeque<Integer> free = new ArrayDeque<>();

    /** ¿Hace falta un buffer extra para arrancar sin cortes? */
    boolean wantsPrebuffer() {
        return context != ALC10.alcGetCurrentContext() || queued < 2;
    }

    /** Encola un buffer (estéreo intercalado, 48 kHz). */
    void push(float[] stereo) {
        if (stereo == null || stereo.length == 0) return;
        long ctx = ALC10.alcGetCurrentContext();
        if (ctx == 0) return;
        if (ctx != context) {
            // Primera vez, o Minecraft ha reiniciado el sonido (Flashback lo hace al
            // exportar con audio) y las fuentes viejas ya no valen.
            context = ctx;
            free.clear();
            queued = 0;
            source = AL10.alGenSources();
            AL10.alSourcei(source, AL10.AL_SOURCE_RELATIVE, AL10.AL_TRUE);
            AL10.alSource3f(source, AL10.AL_POSITION, 0f, 0f, 0f);
            AL10.alSourcef(source, AL10.AL_ROLLOFF_FACTOR, 0f);
            AL10.alSourcef(source, AL10.AL_GAIN, 1f);
        }
        int processed = AL10.alGetSourcei(source, AL10.AL_BUFFERS_PROCESSED);
        while (processed-- > 0) {
            free.add(AL10.alSourceUnqueueBuffers(source));
            queued--;
        }
        if (queued >= MAX_QUEUED) return;
        int buffer = free.isEmpty() ? AL10.alGenBuffers() : free.poll();
        ShortBuffer pcm = MemoryUtil.memAllocShort(stereo.length);
        try {
            for (float s : stereo) {
                pcm.put((short) Math.round(Math.max(-1f, Math.min(1f, s)) * 32767f));
            }
            pcm.flip();
            AL10.alBufferData(buffer, AL10.AL_FORMAT_STEREO16, pcm, RATE);
        } finally {
            MemoryUtil.memFree(pcm);
        }
        AL10.alSourceQueueBuffers(source, buffer);
        queued++;
        if (AL10.alGetSourcei(source, AL10.AL_SOURCE_STATE) != AL10.AL_PLAYING) {
            AL10.alSourcePlay(source);
        }
    }

    void close() {
        if (context != 0 && context == ALC10.alcGetCurrentContext()) {
            AL10.alSourceStop(source); // al parar, todos los buffers pasan a procesados
            int processed = AL10.alGetSourcei(source, AL10.AL_BUFFERS_PROCESSED);
            while (processed-- > 0) free.add(AL10.alSourceUnqueueBuffers(source));
            AL10.alDeleteSources(source);
            for (int b : free) AL10.alDeleteBuffers(b);
        }
        context = 0;
        free.clear();
        queued = 0;
    }
}
