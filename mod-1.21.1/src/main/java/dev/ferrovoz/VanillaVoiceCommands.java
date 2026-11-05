package dev.ferrovoz;

import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.suggestion.SuggestionsBuilder;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;

import java.util.Collection;
import java.util.List;
import java.util.UUID;
import java.util.function.LongSupplier;
import java.util.function.Supplier;

/**
 * {@code /voz} y {@code /voice} con Brigadier, que es de Minecraft: igual en
 * Fabric, NeoForge y Forge. {@link VoiceCommands} hace todo lo demás; los
 * permisos los resuelve cada cargador ({@link Ferrovoz.Platform#hasPermission}).
 */
final class VanillaVoiceCommands {
    private VanillaVoiceCommands() {}

    static boolean check(CommandSourceStack source, String node) {
        return Ferrovoz.platform().hasPermission(source, node);
    }

    /**
     * Idioma de Minecraft del jugador ({@code es_es}, {@code en_us}...). En 1.21.1 se
     * llama directamente: en Fabric los nombres están ofuscados y por reflexión no se
     * encontraría el método.
     */
    static String localeOf(ServerPlayer player) {
        if (player == null) return null;
        try {
            return player.clientInformation().language();
        } catch (RuntimeException e) {
            return null;
        }
    }

    static void register(CommandDispatcher<CommandSourceStack> dispatcher, LongSupplier server, Supplier<FerrovozConfig> config) {
        // /voz y /voice (los subcomandos valen en los dos idiomas).
        for (String name : new String[] {"voz", "voice"}) {
            dispatcher.register(Commands.literal(name)
                    .executes(ctx -> run(ctx, "", server, config))
                    .then(Commands.argument("args", StringArgumentType.greedyString())
                            .suggests((ctx, builder) -> suggest(ctx, builder, server, config))
                            .executes(ctx -> run(ctx, StringArgumentType.getString(ctx, "args"), server, config))));
        }
    }

    private static int run(CommandContext<CommandSourceStack> ctx, String text, LongSupplier server, Supplier<FerrovozConfig> config) {
        String trimmed = text.trim();
        String[] args = trimmed.isEmpty() ? new String[0] : trimmed.split("\\s+");
        CommandSourceStack source = ctx.getSource();
        VoiceCommands.execute(sender(source), host(source.getServer(), server, config), args);
        return 1;
    }

    private static java.util.concurrent.CompletableFuture<com.mojang.brigadier.suggestion.Suggestions> suggest(
            CommandContext<CommandSourceStack> ctx, SuggestionsBuilder builder, LongSupplier server, Supplier<FerrovozConfig> config) {
        String typed = builder.getRemaining();
        String[] parts = typed.split(" ", -1);
        SuggestionsBuilder last = builder.createOffset(builder.getStart() + typed.lastIndexOf(' ') + 1);
        CommandSourceStack source = ctx.getSource();
        for (String option : VoiceCommands.complete(sender(source), host(source.getServer(), server, config), parts)) {
            last.suggest(option);
        }
        return last.buildFuture();
    }

    private static VoiceCommands.Sender sender(CommandSourceStack source) {
        ServerPlayer player = source.getPlayer();
        return new VoiceCommands.Sender() {
            @Override
            public UUID player() {
                return player == null ? null : player.getUUID();
            }

            @Override
            public boolean hasPermission(String node) {
                return check(source, node);
            }

            @Override
            public void reply(String message) {
                source.sendSuccess(() -> Component.literal(message), false);
            }

            @Override
            public String locale() {
                return player == null ? VoiceCommands.consoleLocale() : localeOf(player);
            }
        };
    }

    static VoiceCommands.Host host(MinecraftServer mc, LongSupplier server, Supplier<FerrovozConfig> config) {
        return new VoiceCommands.Host() {
            @Override
            public long voiceServer() {
                return server.getAsLong();
            }

            @Override
            public UUID findOnline(String name) {
                for (ServerPlayer p : mc.getPlayerList().getPlayers()) {
                    if (p.getName().getString().equalsIgnoreCase(name)) return p.getUUID();
                }
                return null;
            }

            @Override
            public String nameOf(UUID id) {
                ServerPlayer p = mc.getPlayerList().getPlayer(id);
                return p == null ? id.toString().substring(0, 8) : p.getName().getString();
            }

            @Override
            public Collection<String> onlineNames() {
                List<String> names = new java.util.ArrayList<>();
                for (ServerPlayer p : mc.getPlayerList().getPlayers()) names.add(p.getName().getString());
                return names;
            }

            @Override
            public FerrovozConfig config() {
                return config.get();
            }

            @Override
            public void message(UUID id, String message) {
                ServerPlayer p = mc.getPlayerList().getPlayer(id);
                if (p != null) p.sendSystemMessage(Component.literal(message));
            }

            @Override
            public String localeOf(UUID id) {
                return VanillaVoiceCommands.localeOf(mc.getPlayerList().getPlayer(id));
            }
        };
    }
}
