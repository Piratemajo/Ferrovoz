package dev.ferrovoz.bukkit;

import io.papermc.paper.threadedregions.scheduler.ScheduledTask;
import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.plugin.java.JavaPlugin;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Planificador por entidad de Paper/Folia. Es seguro llamarlo desde cualquier
 * hilo y siempre ejecuta en el hilo que posee al jugador. Cuando el jugador
 * sale, sus tareas se retiran solas.
 */
final class PaperScheduling implements PlayerScheduling {
    private final JavaPlugin plugin;
    private final Map<UUID, ScheduledTask> tracking = new ConcurrentHashMap<>();

    PaperScheduling(JavaPlugin plugin) {
        this.plugin = plugin;
    }

    @Override
    public void runFor(Player player, Runnable task) {
        player.getScheduler().run(plugin, t -> task.run(), null);
    }

    @Override
    public void trackEveryTick(Player player, Runnable task) {
        UUID id = player.getUniqueId();
        ScheduledTask scheduled = player.getScheduler().runAtFixedRate(plugin, t -> task.run(),
                () -> tracking.remove(id), 1L, 1L);
        if (scheduled != null) {
            ScheduledTask old = tracking.put(id, scheduled);
            if (old != null) old.cancel();
        }
    }

    @Override
    public void repeatGlobally(Runnable task, long periodTicks) {
        Bukkit.getGlobalRegionScheduler().runAtFixedRate(plugin, t -> task.run(), periodTicks, periodTicks);
    }

    @Override
    public void untrack(UUID player) {
        ScheduledTask task = tracking.remove(player);
        if (task != null) task.cancel();
    }
}
