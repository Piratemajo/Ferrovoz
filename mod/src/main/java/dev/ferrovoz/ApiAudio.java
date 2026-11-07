package dev.ferrovoz;

import dev.ferrovoz.api.Position;
import dev.ferrovoz.api.Subscription;
import dev.ferrovoz.api.TalkMode;
import dev.ferrovoz.api.audio.AudioChannel;
import dev.ferrovoz.api.audio.AudioFormat;
import dev.ferrovoz.api.audio.AudioService;
import dev.ferrovoz.api.audio.ChannelBuilder;
import dev.ferrovoz.api.audio.FrameSource;
import dev.ferrovoz.api.audio.MicrophoneFrame;
import dev.ferrovoz.api.audio.MicrophoneListener;
import dev.ferrovoz.api.audio.OpusDecoder;
import dev.ferrovoz.api.audio.OpusEncoder;
import dev.ferrovoz.api.audio.Playback;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.Collection;
import java.util.List;
import java.util.Objects;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/**
 * Audio de la API: canales, reproducción a ritmo, escucha del micro y códec. Los hilos
 * ({@code ferrovoz-audio} para reproducir y {@code ferrovoz-microphone} para escuchar)
 * solo se crean si algún plugin los usa.
 */
final class ApiAudio implements AudioService {
    private static final int DEFAULT_BITRATE = 48_000;
    private static final double MAX_RANGE = 30_000;

    private final ApiCore core;
    private final Set<Channel> channels = ConcurrentHashMap.newKeySet();
    private final Set<PlaybackImpl> playing = ConcurrentHashMap.newKeySet();
    private final List<MicrophoneListener> listeners = new CopyOnWriteArrayList<>();
    private ScheduledExecutorService player;
    private ScheduledExecutorService microphone;

    ApiAudio(ApiCore core) {
        this.core = core;
    }

    // --- Ciclo de vida (lo llama ApiCore) ---

    synchronized void start(long server) {
        if (!listeners.isEmpty()) startMicrophone(server);
    }

    /** El relé se para: se cierra todo lo que dependía de él. */
    synchronized void stop() {
        for (Channel c : channels) c.closeLocally();
        channels.clear();
        for (PlaybackImpl p : playing) p.finish(null);
        playing.clear();
        if (player != null) {
            player.shutdownNow();
            player = null;
        }
        if (microphone != null) {
            microphone.shutdownNow();
            microphone = null;
        }
    }

    // --- Canales ---

    @Override
    public ChannelBuilder channel() {
        return new Builder();
    }

    private final class Builder implements ChannelBuilder {
        private int bitrate = DEFAULT_BITRATE;
        private boolean music;

        @Override
        public ChannelBuilder bitrate(int bitsPerSecond) {
            if (bitsPerSecond < 6_000 || bitsPerSecond > 256_000) {
                throw new IllegalArgumentException("bitrate must be within 6000-256000: " + bitsPerSecond);
            }
            bitrate = bitsPerSecond;
            return this;
        }

        @Override
        public ChannelBuilder music() {
            music = true;
            return this;
        }

        @Override
        public AudioChannel at(Position position, double range) {
            Channel c = create(range);
            c.moveTo(position);
            return c;
        }

        @Override
        public AudioChannel following(UUID player, double range) {
            Channel c = create(range);
            c.follow(player);
            return c;
        }

        @Override
        public AudioChannel to(Collection<UUID> listeners) {
            Channel c = create(1);
            c.sendTo(listeners);
            return c;
        }

        @Override
        public AudioChannel toGroup(String group) {
            Channel c = create(1);
            c.sendToGroup(group);
            return c;
        }

        private Channel create(double range) {
            checkRange(range);
            long server = core.requireRunning();
            long[] id = FerrovozNative.serverChannelCreate(server, (float) range, bitrate, music);
            if (id == null || id.length != 2) {
                throw new IllegalStateException("could not create the channel (are 1024 already open?)");
            }
            Channel c = new Channel(server, new UUID(id[0], id[1]));
            channels.add(c);
            return c;
        }
    }

    private static void checkRange(double range) {
        if (!(range >= 1 && range <= MAX_RANGE)) throw new IllegalArgumentException("range must be within 1-30000: " + range);
    }

    private final class Channel implements AudioChannel {
        private final long server;
        private final UUID id;
        private final long msb, lsb;
        private volatile boolean open = true;
        private volatile float volume = 1f;
        private volatile PlaybackImpl current;

        Channel(long server, UUID id) {
            this.server = server;
            this.id = id;
            this.msb = id.getMostSignificantBits();
            this.lsb = id.getLeastSignificantBits();
        }

        @Override
        public UUID id() {
            return id;
        }

        @Override
        public boolean isOpen() {
            return open;
        }

        private void requireOpen() {
            if (!open) throw new IllegalStateException("channel is closed");
        }

        @Override
        public void moveTo(Position position) {
            Objects.requireNonNull(position, "position");
            requireOpen();
            int dimension = core.dimensionOf(position.world());
            FerrovozNative.serverChannelPoint(server, msb, lsb, position.x(), position.y(), position.z(), dimension);
        }

        @Override
        public void follow(UUID player) {
            Objects.requireNonNull(player, "player");
            requireOpen();
            FerrovozNative.serverChannelFollow(server, msb, lsb, player.getMostSignificantBits(), player.getLeastSignificantBits());
        }

        @Override
        public void sendTo(Collection<UUID> listeners) {
            Objects.requireNonNull(listeners, "listeners");
            requireOpen();
            FerrovozNative.serverChannelListeners(server, msb, lsb, ApiCore.pack(listeners));
        }

        @Override
        public void sendToGroup(String group) {
            Objects.requireNonNull(group, "group");
            requireOpen();
            FerrovozNative.serverChannelGroup(server, msb, lsb, group);
        }

        @Override
        public void setRange(double blocks) {
            checkRange(blocks);
            requireOpen();
            FerrovozNative.serverChannelRange(server, msb, lsb, (float) blocks);
        }

        @Override
        public void setVolume(float volume) {
            if (!(volume >= 0 && volume <= 4)) throw new IllegalArgumentException("volume must be within 0-4: " + volume);
            this.volume = volume;
        }

        @Override
        public float volume() {
            return volume;
        }

        @Override
        public boolean send(short[] frame) {
            if (frame == null || frame.length != AudioFormat.FRAME_SAMPLES) {
                throw new IllegalArgumentException("a frame is " + AudioFormat.FRAME_SAMPLES + " samples");
            }
            return open && FerrovozNative.serverChannelSendPcm(server, msb, lsb, frame, volume);
        }

        @Override
        public boolean sendOpus(byte[] frame) {
            Objects.requireNonNull(frame, "frame");
            return open && FerrovozNative.serverChannelSendOpus(server, msb, lsb, frame);
        }

        @Override
        public Playback play(FrameSource source) {
            Objects.requireNonNull(source, "source");
            requireOpen();
            PlaybackImpl p = new PlaybackImpl(this, source);
            PlaybackImpl old = current;
            current = p;
            if (old != null) old.stop();
            startPlayback(p);
            return p;
        }

        @Override
        public void stop() {
            PlaybackImpl p = current;
            if (p != null) p.stop();
        }

        @Override
        public void close() {
            if (!open) return;
            closeLocally();
            channels.remove(this);
            FerrovozNative.serverChannelRemove(server, msb, lsb);
        }

        /** Sin tocar el relé (que ya se ha parado o lo hará quien llama). */
        void closeLocally() {
            open = false;
            stop();
        }

        @Override
        public String toString() {
            return "AudioChannel[" + id + (open ? "]" : ", cerrado]");
        }
    }

    // --- Reproducción ---

    private final class PlaybackImpl implements Playback {
        private final Channel channel;
        private final FrameSource source;
        private final CompletableFuture<Void> finished = new CompletableFuture<>();

        PlaybackImpl(Channel channel, FrameSource source) {
            this.channel = channel;
            this.source = source;
        }

        /** Cada 20 ms en el hilo de reproducción. */
        void tick() {
            if (finished.isDone()) return;
            if (!channel.isOpen()) {
                finish(null);
                return;
            }
            short[] frame;
            try {
                frame = source.next();
                if (frame == null) {
                    finish(null);
                    return;
                }
                channel.send(frame);
            } catch (RuntimeException e) {
                finish(e);
            }
        }

        void finish(Throwable error) {
            playing.remove(this);
            if (error == null) {
                finished.complete(null);
            } else {
                FerrovozLog.LOG.warn("Ferrovoz: audio source on channel {} failed", channel.id(), error);
                finished.completeExceptionally(error);
            }
        }

        @Override
        public boolean isPlaying() {
            return !finished.isDone();
        }

        @Override
        public void stop() {
            finish(null);
        }

        @Override
        public CompletableFuture<Void> finished() {
            return finished;
        }
    }

    private synchronized void startPlayback(PlaybackImpl p) {
        playing.add(p);
        if (player == null) {
            player = Executors.newSingleThreadScheduledExecutor(daemon("ferrovoz-audio"));
            player.scheduleAtFixedRate(() -> {
                for (PlaybackImpl each : playing) each.tick();
            }, 0, AudioFormat.FRAME_MILLIS, TimeUnit.MILLISECONDS);
        }
    }

    // --- Escucha del micro ---

    @Override
    public synchronized Subscription listen(MicrophoneListener listener) {
        Objects.requireNonNull(listener, "listener");
        listeners.add(listener);
        long server = core.server();
        if (server != 0) startMicrophone(server);
        return () -> {
            synchronized (ApiAudio.this) {
                if (listeners.remove(listener) && listeners.isEmpty()) stopMicrophone();
            }
        };
    }

    private void startMicrophone(long server) {
        FerrovozNative.serverSetMicTap(server, true);
        if (microphone != null) return;
        microphone = Executors.newSingleThreadScheduledExecutor(daemon("ferrovoz-microphone"));
        microphone.scheduleWithFixedDelay(() -> pollMicrophone(server), 0, AudioFormat.FRAME_MILLIS, TimeUnit.MILLISECONDS);
    }

    private void stopMicrophone() {
        long server = core.server();
        if (server != 0) FerrovozNative.serverSetMicTap(server, false);
        if (microphone != null) {
            microphone.shutdownNow();
            microphone = null;
        }
    }

    private void pollMicrophone(long server) {
        byte[] packed = FerrovozNative.serverPollMicrophone(server);
        if (packed == null) return;
        for (MicrophoneFrame frame : unpack(packed)) {
            for (MicrophoneListener l : listeners) {
                try {
                    l.onFrame(frame);
                } catch (RuntimeException | LinkageError e) {
                    FerrovozLog.LOG.warn("A Ferrovoz microphone listener failed", e);
                }
            }
        }
    }

    /** Formato de {@link FerrovozNative#serverPollMicrophone}. Lo que no cuadre se ignora. */
    static List<MicrophoneFrame> unpack(byte[] packed) {
        ByteBuffer b = ByteBuffer.wrap(packed).order(ByteOrder.LITTLE_ENDIAN);
        List<MicrophoneFrame> out = new java.util.ArrayList<>();
        while (b.remaining() >= 24) {
            // Los UUID van como 16 bytes en orden de red, igual que uuid_to_java.
            long msb = Long.reverseBytes(b.getLong());
            long lsb = Long.reverseBytes(b.getLong());
            int seq = b.getInt();
            int mode = b.get() & 0xFF;
            int flags = b.get() & 0xFF;
            int len = b.getShort() & 0xFFFF;
            if (len > b.remaining()) break;
            byte[] opus = new byte[len];
            b.get(opus);
            out.add(new MicrophoneFrame(new UUID(msb, lsb), seq, TalkMode.fromWire(mode), (flags & 1) != 0, opus));
        }
        return out;
    }

    // --- Códec ---

    @Override
    public OpusEncoder createEncoder(int bitrate, boolean music) {
        if (bitrate < 6_000 || bitrate > 256_000) throw new IllegalArgumentException("bitrate must be within 6000-256000: " + bitrate);
        long h = FerrovozNative.opusEncoderCreate(bitrate, music);
        if (h == 0) throw new IllegalStateException("Opus could not create the encoder");
        return new OpusEncoder() {
            private long handle = h;

            @Override
            public synchronized byte[] encode(short[] frame) {
                if (frame == null || frame.length != AudioFormat.FRAME_SAMPLES) {
                    throw new IllegalArgumentException("a frame is " + AudioFormat.FRAME_SAMPLES + " samples");
                }
                if (handle == 0) throw new IllegalStateException("encoder is closed");
                byte[] out = FerrovozNative.opusEncode(handle, frame);
                if (out == null) throw new IllegalStateException("Opus could not encode the frame");
                return out;
            }

            @Override
            public synchronized void close() {
                if (handle != 0) FerrovozNative.opusEncoderClose(handle);
                handle = 0;
            }
        };
    }

    @Override
    public OpusDecoder createDecoder() {
        long h = FerrovozNative.opusDecoderCreate();
        if (h == 0) throw new IllegalStateException("Opus could not create the decoder");
        return new OpusDecoder() {
            private long handle = h;

            @Override
            public synchronized short[] decode(byte[] frame) {
                if (handle == 0) throw new IllegalStateException("decoder is closed");
                short[] out = FerrovozNative.opusDecode(handle, frame);
                if (out == null) throw new IllegalStateException("decoder is closed");
                return out;
            }

            @Override
            public synchronized void close() {
                if (handle != 0) FerrovozNative.opusDecoderClose(handle);
                handle = 0;
            }
        };
    }

    private static java.util.concurrent.ThreadFactory daemon(String name) {
        return r -> {
            Thread t = new Thread(r, name);
            t.setDaemon(true);
            return t;
        };
    }
}
