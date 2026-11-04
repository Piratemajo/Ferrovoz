package dev.ferrovoz.bukkit;

import org.bukkit.entity.Player;
import org.bukkit.plugin.java.JavaPlugin;

import java.util.UUID;

/**
 * Dónde se ejecuta el código de cada jugador.
 *
 * <ul>
 *   <li>Paper, Purpur y Folia: scheduler de entidad. En Folia cada jugador está en el
 *       hilo de su región y solo ahí se puede leer su posición; en Paper corre en el
 *       hilo principal.</li>
 *   <li>Spigot: sin scheduler de entidad, una tarea global en el hilo principal.</li>
 * </ul>
 *
 * La implementación de Paper está en su propia clase y solo se carga si existe, para
 * que Spigot no falle al no encontrar las clases de Paper.
 */
interface PlayerScheduling {
    /** Ejecuta {@code task} en el hilo del jugador lo antes posible. */
    void runFor(Player player, Runnable task);

    /** Ejecuta {@code task} cada tick en el hilo del jugador hasta que salga o se llame a {@link #untrack}. */
    void trackEveryTick(Player player, Runnable task);

    void untrack(UUID player);

    static PlayerScheduling create(JavaPlugin plugin) {
        if (classExists("io.papermc.paper.threadedregions.scheduler.EntityScheduler")) {
            return new PaperScheduling(plugin);
        }
        return new BukkitScheduling(plugin);
    }

    /** Folia tiene esta clase; Paper y Spigot no. Solo para el log. */
    static boolean isFolia() {
        return classExists("io.papermc.paper.threadedregions.RegionizedServer");
    }

    private static boolean classExists(String name) {
        try {
            Class.forName(name, false, PlayerScheduling.class.getClassLoader());
            return true;
        } catch (ClassNotFoundException | LinkageError e) {
            return false;
        }
    }
}
