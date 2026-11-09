package dev.ferrovoz.api;

import dev.ferrovoz.api.audio.AudioService;
import dev.ferrovoz.api.event.EventBus;
import dev.ferrovoz.api.internal.ApiAccess;

import java.util.Collection;
import java.util.Optional;
import java.util.UUID;
import java.util.function.Consumer;

/**
 * Entry point of the Ferrovoz server API. It is the same on Fabric, NeoForge, Forge and
 * Paper/Spigot/Folia, and on every Minecraft version Ferrovoz supports: it uses no game
 * classes.
 *
 * <pre>{@code
 * FerrovozApi.whenReady(api -> {
 *     api.events().subscribe(PlayerStartedSpeakingEvent.class, e ->
 *             log.info(e.player().name() + " is talking"));
 * });
 * }</pre>
 *
 * <p>Everything can be called from any thread. Events are delivered on the server
 * thread (on Folia, on the global region scheduler); voice received through
 * {@link AudioService#listen} arrives on a thread of its own.
 *
 * <p>While the voice relay is stopped ({@link #isRunning()} is false), queries return
 * empty results, changes do nothing and creating an audio channel throws
 * {@link IllegalStateException}.
 */
public interface FerrovozApi {
    /** Version of this API. It goes up when something changes incompatibly. */
    int API_VERSION = 1;

    /** @throws IllegalStateException if Ferrovoz has not loaded yet */
    static FerrovozApi get() {
        return ApiAccess.require();
    }

    /** Empty if Ferrovoz is not loaded (or has no native library for this machine). */
    static Optional<FerrovozApi> find() {
        return Optional.ofNullable(ApiAccess.current());
    }

    /**
     * Runs {@code action} as soon as the API is available: right away if it already is,
     * or when Ferrovoz finishes loading. The load order of mods and plugins does not
     * matter.
     */
    static void whenReady(Consumer<FerrovozApi> action) {
        ApiAccess.whenReady(action);
    }

    /** Ferrovoz version, e.g. {@code "0.12.0"}. */
    String version();

    /**
     * {@code "Fabric"}, {@code "NeoForge"}, {@code "Forge"}, {@code "Paper"}, {@code "Spigot"} or {@code "Folia"}.
     * On Quilt it is {@code "Fabric"}: Quilt runs Fabric mods and Fabric API.
     */
    String platform();

    /** The voice relay is running. */
    boolean isRunning();

    /** A player with Ferrovoz on this server. Empty if they don't have the mod or aren't online. */
    Optional<VoicePlayer> player(UUID id);

    /** Players with Ferrovoz who are online now. */
    Collection<VoicePlayer> players();

    VoiceGroups groups();

    /** Ranges, crowds, echo and hush. Changes are not written to the config file. */
    VoiceSettings settings();

    /** Audio channels, microphone listening and the Opus codec. */
    AudioService audio();

    EventBus events();

    /**
     * The Ferrovoz world for a game world: an {@code org.bukkit.World} on Paper or a
     * {@code ServerLevel} on mod loaders.
     *
     * @throws IllegalArgumentException if it is not a world of this platform
     */
    VoiceWorld world(Object platformWorld);
}
