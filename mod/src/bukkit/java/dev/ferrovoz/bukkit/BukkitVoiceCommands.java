package dev.ferrovoz.bukkit;

import dev.ferrovoz.FerrovozConfig;
import dev.ferrovoz.VoiceCommands;
import org.bukkit.Bukkit;
import org.bukkit.command.Command;
import org.bukkit.command.CommandExecutor;
import org.bukkit.command.CommandSender;
import org.bukkit.command.TabCompleter;
import org.bukkit.entity.Player;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.UUID;
import java.util.function.LongSupplier;
import java.util.function.Supplier;

/**
 * {@code /voz} en Paper/Spigot/Folia. Los permisos son nodos normales de Bukkit
 * ({@code ferrovoz.speak}, {@code ferrovoz.admin}...) con sus valores por defecto
 * en {@code plugin.yml}, así que LuckPerms (o cualquier plugin de permisos) los gestiona.
 */
final class BukkitVoiceCommands implements CommandExecutor, TabCompleter {
    private final LongSupplier server;
    private final Supplier<FerrovozConfig> config;
    private final java.util.Map<UUID, Player> online;
    private final Supplier<VoiceCommands.Reload> reload;

    BukkitVoiceCommands(LongSupplier server, Supplier<FerrovozConfig> config, java.util.Map<UUID, Player> online,
            Supplier<VoiceCommands.Reload> reload) {
        this.server = server;
        this.config = config;
        this.online = online;
        this.reload = reload;
    }

    static String node(String suffix) {
        return "ferrovoz." + suffix;
    }

    /** Idioma de Minecraft del jugador ({@code es_es}...). {@code getLocale} existe en Spigot y en Paper. */
    @SuppressWarnings("deprecation")
    static String localeOf(Player p) {
        try {
            return p == null ? null : p.getLocale();
        } catch (RuntimeException | LinkageError e) {
            return null;
        }
    }

    @Override
    public boolean onCommand(CommandSender sender, Command command, String label, String[] args) {
        VoiceCommands.execute(wrap(sender), host(), args);
        return true;
    }

    @Override
    public List<String> onTabComplete(CommandSender sender, Command command, String label, String[] args) {
        return VoiceCommands.complete(wrap(sender), host(), args);
    }

    private static VoiceCommands.Sender wrap(CommandSender sender) {
        return new VoiceCommands.Sender() {
            @Override
            public UUID player() {
                return sender instanceof Player p ? p.getUniqueId() : null;
            }

            @Override
            public boolean hasPermission(String suffix) {
                return sender.hasPermission(node(suffix));
            }

            @Override
            public void reply(String message) {
                sender.sendMessage(message);
            }

            @Override
            public String locale() {
                return sender instanceof Player p ? localeOf(p) : VoiceCommands.consoleLocale();
            }
        };
    }

    VoiceCommands.Host host() {
        return new VoiceCommands.Host() {
            @Override
            public long voiceServer() {
                return server.getAsLong();
            }

            @Override
            public VoiceCommands.Reload reload() {
                return BukkitVoiceCommands.this.reload.get();
            }

            @Override
            public UUID findOnline(String name) {
                Player p = Bukkit.getPlayerExact(name);
                if (p != null) return p.getUniqueId();
                for (Player o : Bukkit.getOnlinePlayers()) {
                    if (o.getName().equalsIgnoreCase(name)) return o.getUniqueId();
                }
                return null;
            }

            @Override
            public String nameOf(UUID id) {
                Player p = online.get(id); // mapa concurrente: seguro desde el hilo de avisos
                if (p == null) p = Bukkit.getPlayer(id);
                if (p != null) return p.getName();
                org.bukkit.OfflinePlayer off = Bukkit.getOfflinePlayer(id);
                return off.getName() != null ? off.getName() : id.toString().substring(0, 8);
            }

            @Override
            public Collection<String> onlineNames() {
                List<String> names = new ArrayList<>();
                for (Player p : Bukkit.getOnlinePlayers()) names.add(p.getName());
                return names;
            }

            @Override
            public FerrovozConfig config() {
                return config.get();
            }

            @Override
            public void message(UUID id, String message) {
                Player p = online.get(id);
                if (p != null) p.sendMessage(message);
            }

            @Override
            public String localeOf(UUID id) {
                return BukkitVoiceCommands.localeOf(online.get(id));
            }
        };
    }
}
