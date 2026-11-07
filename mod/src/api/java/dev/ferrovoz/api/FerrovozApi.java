package dev.ferrovoz.api;

import dev.ferrovoz.api.audio.AudioService;
import dev.ferrovoz.api.event.EventBus;
import dev.ferrovoz.api.internal.ApiAccess;

import java.util.Collection;
import java.util.Optional;
import java.util.UUID;
import java.util.function.Consumer;

/**
 * Entrada a la API de Ferrovoz en el servidor. Es la misma en Fabric, NeoForge, Forge
 * y Paper/Spigot/Folia, y en todas las versiones de Minecraft que soporta Ferrovoz:
 * no usa ninguna clase del juego.
 *
 * <pre>{@code
 * FerrovozApi.whenReady(api -> {
 *     api.events().subscribe(PlayerStartedSpeakingEvent.class, e ->
 *             log.info(e.player().name() + " habla"));
 * });
 * }</pre>
 *
 * <p>Todo se puede llamar desde cualquier hilo. Los eventos llegan en el hilo del
 * servidor (en Folia, en el planificador global); la voz que se escucha con
 * {@link AudioService#listen}, en un hilo propio.
 *
 * <p>Con el relé de voz parado ({@link #isRunning()} false) las consultas devuelven
 * vacío, los cambios no hacen nada y crear un canal de audio lanza
 * {@link IllegalStateException}.
 */
public interface FerrovozApi {
    /** Versión de esta API. Sube al cambiar algo de forma incompatible. */
    int API_VERSION = 1;

    /** @throws IllegalStateException si Ferrovoz no está cargado todavía. */
    static FerrovozApi get() {
        return ApiAccess.require();
    }

    /** Vacío si Ferrovoz no está cargado (o no tiene librería nativa para esta máquina). */
    static Optional<FerrovozApi> find() {
        return Optional.ofNullable(ApiAccess.current());
    }

    /**
     * Ejecuta {@code action} en cuanto la API esté disponible: ahora mismo si ya lo
     * está, o cuando Ferrovoz termine de cargar. No hace falta preocuparse del orden en
     * que cargan los mods o plugins.
     */
    static void whenReady(Consumer<FerrovozApi> action) {
        ApiAccess.whenReady(action);
    }

    /** Versión de Ferrovoz, p. ej. {@code "0.12.0"}. */
    String version();

    /** {@code "Fabric"}, {@code "NeoForge"}, {@code "Forge"} o {@code "Paper"}. */
    String platform();

    /** El relé de voz está en marcha. */
    boolean isRunning();

    /** Un jugador con Ferrovoz en este servidor. Vacío si no tiene el mod o no está. */
    Optional<VoicePlayer> player(UUID id);

    /** Los jugadores con Ferrovoz conectados ahora. */
    Collection<VoicePlayer> players();

    VoiceGroups groups();

    /** Alcances, multitudes, eco y silencio general. Los cambios no se guardan en el archivo. */
    VoiceSettings settings();

    /** Canales de audio, escucha del micro y códec Opus. */
    AudioService audio();

    EventBus events();

    /**
     * El mundo de Ferrovoz que corresponde a uno del juego: un {@code org.bukkit.World}
     * en Paper o un {@code ServerLevel} en los mods.
     *
     * @throws IllegalArgumentException si no es un mundo de esta plataforma
     */
    VoiceWorld world(Object platformWorld);
}
