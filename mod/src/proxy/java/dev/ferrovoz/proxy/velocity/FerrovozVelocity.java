package dev.ferrovoz.proxy.velocity;

import com.google.inject.Inject;
import com.velocitypowered.api.command.CommandSource;
import com.velocitypowered.api.command.SimpleCommand;
import com.velocitypowered.api.event.Subscribe;
import com.velocitypowered.api.event.connection.DisconnectEvent;
import com.velocitypowered.api.event.connection.PluginMessageEvent;
import com.velocitypowered.api.event.player.ServerConnectedEvent;
import com.velocitypowered.api.event.proxy.ProxyInitializeEvent;
import com.velocitypowered.api.event.proxy.ProxyShutdownEvent;
import com.velocitypowered.api.plugin.annotation.DataDirectory;
import com.velocitypowered.api.proxy.Player;
import com.velocitypowered.api.proxy.ProxyServer;
import com.velocitypowered.api.proxy.ServerConnection;
import com.velocitypowered.api.proxy.messages.MinecraftChannelIdentifier;
import dev.ferrovoz.Protocol;
import dev.ferrovoz.proxy.ProxyCore;
import net.kyori.adventure.text.serializer.legacy.LegacyComponentSerializer;

import java.nio.file.Path;
import java.util.List;
import java.util.Locale;
import java.util.UUID;

/** Ferrovoz como plugin de Velocity (descriptor: {@code velocity-plugin.json}). */
public final class FerrovozVelocity {
    private static final MinecraftChannelIdentifier SETUP = MinecraftChannelIdentifier.create(Protocol.NAMESPACE, Protocol.PATH);

    private final ProxyServer proxy;
    private final Path dataDir;
    private final ProxyCore core = new ProxyCore("Velocity");

    @Inject
    public FerrovozVelocity(ProxyServer proxy, @DataDirectory Path dataDir) {
        this.proxy = proxy;
        this.dataDir = dataDir;
    }

    @Subscribe
    public void onInit(ProxyInitializeEvent e) {
        core.start(dataDir);
        // Registrado, Velocity nos avisa de este canal en vez de reenviarlo sin más.
        proxy.getChannelRegistrar().register(SETUP);
        core.setPlayers(new ProxyCore.Players() {
            @Override
            public ProxyCore.NetPlayer byName(String name) {
                return proxy.getPlayer(name).map(FerrovozVelocity::wrap).orElse(null);
            }

            @Override
            public ProxyCore.NetPlayer byId(UUID id) {
                return proxy.getPlayer(id).map(FerrovozVelocity::wrap).orElse(null);
            }
        });
        // Grupos de red: /vozred (subcomandos en español) y /voicenet (en inglés).
        proxy.getCommandManager().register(proxy.getCommandManager().metaBuilder("vozred").plugin(this).build(), new NetCommand(true));
        proxy.getCommandManager().register(proxy.getCommandManager().metaBuilder("voicenet").plugin(this).build(), new NetCommand(false));
    }

    private final class NetCommand implements SimpleCommand {
        private final boolean spanish;

        NetCommand(boolean spanish) {
            this.spanish = spanish;
        }

        @Override
        public void execute(Invocation invocation) {
            CommandSource src = invocation.source();
            core.command(src instanceof Player p ? wrap(p) : null, invocation.arguments());
        }

        @Override
        public List<String> suggest(Invocation invocation) {
            String[] args = invocation.arguments();
            if (args.length <= 1) {
                String prefix = args.length == 0 ? "" : args[0].toLowerCase(Locale.ROOT);
                return ProxyCore.subcommands(spanish).stream().filter(s -> s.startsWith(prefix)).toList();
            }
            return List.of();
        }
    }

    private static ProxyCore.NetPlayer wrap(Player p) {
        return new ProxyCore.NetPlayer() {
            @Override
            public UUID id() {
                return p.getUniqueId();
            }

            @Override
            public String name() {
                return p.getUsername();
            }

            @Override
            public String locale() {
                Locale l = p.getEffectiveLocale();
                return l != null ? l.toString() : "en";
            }

            @Override
            public void tell(String legacyText) {
                p.sendMessage(LegacyComponentSerializer.legacySection().deserialize(legacyText));
            }

            @Override
            public boolean isAdmin() {
                return p.hasPermission("ferrovoz.admin");
            }
        };
    }

    @Subscribe
    public void onShutdown(ProxyShutdownEvent e) {
        core.stop();
    }

    @Subscribe
    public void onPluginMessage(PluginMessageEvent e) {
        if (!SETUP.equals(e.getIdentifier())) return;
        if (!(e.getSource() instanceof ServerConnection server) || !(e.getTarget() instanceof Player player)) return;
        byte[] data = e.getData();
        byte[] out = core.onSetup(player.getUniqueId(), server.getServerInfo().getName(), server.getServerInfo().getAddress(), data);
        if (out != data) {
            e.setResult(PluginMessageEvent.ForwardResult.handled());
            player.sendPluginMessage(SETUP, out);
        }
    }

    @Subscribe
    public void onServerConnected(ServerConnectedEvent e) {
        core.onServerConnected(e.getPlayer().getUniqueId(), e.getServer().getServerInfo().getName());
    }

    @Subscribe
    public void onDisconnect(DisconnectEvent e) {
        core.onDisconnect(e.getPlayer().getUniqueId());
    }
}
