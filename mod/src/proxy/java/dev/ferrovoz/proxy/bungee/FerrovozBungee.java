package dev.ferrovoz.proxy.bungee;

import dev.ferrovoz.Protocol;
import dev.ferrovoz.proxy.ProxyCore;
import net.md_5.bungee.api.CommandSender;
import net.md_5.bungee.api.chat.TextComponent;
import net.md_5.bungee.api.connection.ProxiedPlayer;
import net.md_5.bungee.api.connection.Server;
import net.md_5.bungee.api.event.PlayerDisconnectEvent;
import net.md_5.bungee.api.event.PluginMessageEvent;
import net.md_5.bungee.api.event.ServerConnectedEvent;
import net.md_5.bungee.api.plugin.Command;
import net.md_5.bungee.api.plugin.Listener;
import net.md_5.bungee.api.plugin.Plugin;
import net.md_5.bungee.api.plugin.TabExecutor;
import net.md_5.bungee.event.EventHandler;

import java.net.InetSocketAddress;
import java.net.SocketAddress;
import java.util.List;
import java.util.Locale;
import java.util.UUID;

/** Ferrovoz como plugin de BungeeCord y derivados (descriptor: {@code bungee.yml}). */
public final class FerrovozBungee extends Plugin implements Listener {
    private final ProxyCore core = new ProxyCore("BungeeCord");

    @Override
    public void onEnable() {
        core.start(getDataFolder().toPath());
        getProxy().registerChannel(Protocol.CHANNEL);
        getProxy().getPluginManager().registerListener(this, this);
        core.setPlayers(new ProxyCore.Players() {
            @Override
            public ProxyCore.NetPlayer byName(String name) {
                ProxiedPlayer p = getProxy().getPlayer(name);
                return p != null ? wrap(p) : null;
            }

            @Override
            public ProxyCore.NetPlayer byId(UUID id) {
                ProxiedPlayer p = getProxy().getPlayer(id);
                return p != null ? wrap(p) : null;
            }
        });
        // Grupos de red: /vozred (subcomandos en español) y /voicenet (en inglés).
        getProxy().getPluginManager().registerCommand(this, new NetCommand("vozred", true));
        getProxy().getPluginManager().registerCommand(this, new NetCommand("voicenet", false));
    }

    private final class NetCommand extends Command implements TabExecutor {
        private final boolean spanish;

        NetCommand(String name, boolean spanish) {
            super(name); // sin permiso: cualquiera puede usar los grupos de red
            this.spanish = spanish;
        }

        @Override
        public void execute(CommandSender sender, String[] args) {
            core.command(sender instanceof ProxiedPlayer p ? wrap(p) : null, args);
        }

        @Override
        public Iterable<String> onTabComplete(CommandSender sender, String[] args) {
            if (args.length <= 1) {
                String prefix = args.length == 0 ? "" : args[0].toLowerCase(Locale.ROOT);
                return ProxyCore.subcommands(spanish).stream().filter(s -> s.startsWith(prefix)).toList();
            }
            return List.of();
        }
    }

    private static ProxyCore.NetPlayer wrap(ProxiedPlayer p) {
        return new ProxyCore.NetPlayer() {
            @Override
            public UUID id() {
                return p.getUniqueId();
            }

            @Override
            public String name() {
                return p.getName();
            }

            @Override
            public String locale() {
                Locale l = p.getLocale();
                return l != null ? l.toString() : "en";
            }

            @Override
            public void tell(String legacyText) {
                p.sendMessage(TextComponent.fromLegacy(legacyText));
            }

            @Override
            public boolean isAdmin() {
                return p.hasPermission("ferrovoz.admin");
            }
        };
    }

    @Override
    public void onDisable() {
        core.stop();
    }

    @EventHandler
    public void onPluginMessage(PluginMessageEvent e) {
        if (!Protocol.CHANNEL.equals(e.getTag())) return;
        if (!(e.getSender() instanceof Server server) || !(e.getReceiver() instanceof ProxiedPlayer player)) return;
        SocketAddress addr = server.getInfo().getSocketAddress();
        if (!(addr instanceof InetSocketAddress inet)) return; // sockets unix: no hay UDP que reenviar
        byte[] data = e.getData();
        byte[] out = core.onSetup(player.getUniqueId(), server.getInfo().getName(), inet, data);
        if (out != data) {
            e.setCancelled(true);
            player.sendData(Protocol.CHANNEL, out);
        }
    }

    @EventHandler
    public void onServerConnected(ServerConnectedEvent e) {
        core.onServerConnected(e.getPlayer().getUniqueId(), e.getServer().getInfo().getName());
    }

    @EventHandler
    public void onDisconnect(PlayerDisconnectEvent e) {
        core.onDisconnect(e.getPlayer().getUniqueId());
    }
}
