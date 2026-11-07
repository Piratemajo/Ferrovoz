package dev.ferrovoz.bukkit;

import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.plugin.java.JavaPlugin;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/** Spigot: todo en el hilo principal con una única tarea global por tick. */
final class BukkitScheduling implements PlayerScheduling {
    private final JavaPlugin plugin;
    private final Map<UUID, Runnable> tracking = new ConcurrentHashMap<>();

    BukkitScheduling(JavaPlugin plugin) {
        this.plugin = plugin;
        Bukkit.getScheduler().runTaskTimer(plugin, () -> tracking.values().forEach(Runnable::run), 1L, 1L);
    }

    @Override
    public void runFor(Player player, Runnable task) {
        if (Bukkit.isPrimaryThread()) {
            task.run();
        } else {
            Bukkit.getScheduler().runTask(plugin, task);
        }
    }

    @Override
    public void trackEveryTick(Player player, Runnable task) {
        tracking.put(player.getUniqueId(), task);
    }

    @Override
    public void repeatGlobally(Runnable task, long periodTicks) {
        Bukkit.getScheduler().runTaskTimer(plugin, task, periodTicks, periodTicks);
    }

    @Override
    public void untrack(UUID player) {
        tracking.remove(player);
    }
}
