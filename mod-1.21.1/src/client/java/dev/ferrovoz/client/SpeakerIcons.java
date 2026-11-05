package dev.ferrovoz.client;

import com.mojang.blaze3d.vertex.PoseStack;
import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.resources.language.I18n;
import net.minecraft.network.chat.Component;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.phys.Vec3;

import java.util.UUID;

/**
 * Iconos sobre la cabeza, mirando a la cámara como los nombres: ♪ en quien habla y,
 * en los demás, su estado de voz si el servidor lo envía.
 *
 * <p>En 1.21.1 se dibujan como las etiquetas de nombre: pose relativa a la cámara,
 * girada con la orientación de la cámara, en el {@link MultiBufferSource} del mundo.
 *
 * <ul>
 *   <li>{@link #renderWorld}: todos los jugadores, con la pose en la cámara (Fabric
 *       {@code WorldRenderEvents.AFTER_ENTITIES}).</li>
 *   <li>{@link #renderForPlayer}: después de dibujar un jugador, con la pose en sus
 *       pies (NeoForge y Forge {@code RenderPlayerEvent.Post}).</li>
 * </ul>
 */
public final class SpeakerIcons {
    private static final int GREEN = 0xFF55FF55;
    private static final int AQUA = 0xFF55FFFF;
    private static final int BACKGROUND = 0x40000000;
    private static final int FULL_BRIGHT = 0xF000F0;
    private static final float SCALE = 0.025f;
    private static final double MAX_DISTANCE = 64.0;
    private static final double ABOVE_HEAD = 0.75;

    private SpeakerIcons() {}

    /** Distancia máxima de los indicadores de estado. */
    private static final double STATUS_DISTANCE = 24.0;
    private static final int RED = 0xFFFF5555;
    private static final int GREY = 0xFFAAAAAA;

    /** Texto y color a dibujar sobre un jugador, o null. */
    private record Icon(String text, int color, boolean small) {}

    private static boolean hidden(Minecraft mc) {
        return mc.level == null || mc.options.hideGui;
    }

    private static boolean nothingToShow() {
        long[] speaking = FerrovozClient.speaking();
        return (speaking == null || speaking.length == 0) && FerrovozClient.peerStates() == null;
    }

    /**
     * ♪ si habla (azul si es de grupo); si no, su estado: silenciado, ensordecido o
     * sin voz. Hablar tiene prioridad.
     */
    private static Icon iconFor(Player p, long[] speaking, long[] states) {
        UUID id = p.getUUID();
        long msb = id.getMostSignificantBits(), lsb = id.getLeastSignificantBits();
        for (int i = 0; speaking != null && i + 2 < speaking.length; i += 3) {
            if (speaking[i] == msb && speaking[i + 1] == lsb) {
                boolean group = speaking[i + 2] != 0;
                return new Icon(group ? "♪ " + I18n.get("ferrovoz.icon.group") : "♪", group ? AQUA : GREEN, false);
            }
        }
        if (states == null) return null; // servidor anterior a 0.10
        int flags = -1;
        for (int i = 0; i + 2 < states.length; i += 3) {
            if (states[i] == msb && states[i + 1] == lsb) {
                flags = (int) states[i + 2];
                break;
            }
        }
        if (flags < 0 || (flags & FerrovozNative.PEER_CONNECTED) == 0) {
            // Solo jugadores de la tab list, no NPCs.
            Minecraft mc = Minecraft.getInstance();
            boolean real = mc.getConnection() != null && mc.getConnection().getPlayerInfo(id) != null;
            return real ? new Icon(I18n.get("ferrovoz.icon.no_voice"), GREY, true) : null;
        }
        if ((flags & FerrovozNative.PEER_DEAFENED) != 0) return new Icon(I18n.get("ferrovoz.icon.deafened"), RED, true);
        if ((flags & FerrovozNative.PEER_MUTED) != 0) return new Icon(I18n.get("ferrovoz.icon.muted"), RED, true);
        return null;
    }

    private static boolean skip(Minecraft mc, Player p) {
        return p == mc.player || p == mc.getCameraEntity() || p.isInvisible();
    }

    /** Fabric: todos los jugadores; {@code camera} es la posición de la cámara. */
    public static void renderWorld(Vec3 camera, MultiBufferSource buffers, float partial) {
        Minecraft mc = Minecraft.getInstance();
        if (hidden(mc) || buffers == null || nothingToShow()) return;
        long[] speaking = FerrovozClient.speaking();
        long[] states = FerrovozClient.peerStates();
        PoseStack pose = new PoseStack();
        for (Player p : mc.level.players()) {
            if (skip(mc, p)) continue;
            Icon icon = iconFor(p, speaking, states);
            if (icon == null) continue;
            // Posición interpolada para que no tiemble.
            double x = Mth.lerp(partial, p.xo, p.getX());
            double y = Mth.lerp(partial, p.yo, p.getY()) + p.getBbHeight() + ABOVE_HEAD;
            double z = Mth.lerp(partial, p.zo, p.getZ());
            double dx = x - camera.x, dy = y - camera.y, dz = z - camera.z;
            pose.pushPose();
            pose.translate(dx, dy, dz);
            draw(mc, pose, buffers, dx, dy, dz, icon);
            pose.popPose();
        }
    }

    /**
     * NeoForge y Forge: se acaba de dibujar {@code p} y la pose está en sus pies
     * (relativa a la cámara).
     */
    public static void renderForPlayer(Player p, PoseStack pose, MultiBufferSource buffers, float partial) {
        Minecraft mc = Minecraft.getInstance();
        if (hidden(mc) || buffers == null || skip(mc, p) || nothingToShow()) return;
        Icon icon = iconFor(p, FerrovozClient.speaking(), FerrovozClient.peerStates());
        if (icon == null) return;
        Vec3 camera = mc.gameRenderer.getMainCamera().getPosition();
        double up = p.getBbHeight() + ABOVE_HEAD;
        double dx = Mth.lerp(partial, p.xo, p.getX()) - camera.x;
        double dy = Mth.lerp(partial, p.yo, p.getY()) + up - camera.y;
        double dz = Mth.lerp(partial, p.zo, p.getZ()) - camera.z;
        pose.pushPose();
        pose.translate(0, up, 0);
        draw(mc, pose, buffers, dx, dy, dz, icon);
        pose.popPose();
    }

    /**
     * Dibuja el icono en el origen de la pose, mirando a la cámara.
     * {@code (dx, dy, dz)}: de la cámara al icono.
     */
    private static void draw(Minecraft mc, PoseStack pose, MultiBufferSource buffers,
                             double dx, double dy, double dz, Icon icon) {
        double distance = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (distance > (icon.small() ? STATUS_DISTANCE : MAX_DISTANCE) || distance < 0.5) return;
        // Igual que las etiquetas de nombre de 1.21.1.
        pose.mulPose(mc.getEntityRenderDispatcher().cameraOrientation());
        // Los de estado, más pequeños que el ♪.
        float scale = icon.small() ? SCALE * 0.8f : SCALE;
        pose.scale(scale, -scale, scale);

        Font font = mc.font;
        float width = font.width(icon.text());
        font.drawInBatch(
                Component.literal(icon.text()),
                -width / 2f, 0f,
                icon.color(),
                false,
                pose.last().pose(),
                buffers,
                Font.DisplayMode.NORMAL,
                BACKGROUND,
                FULL_BRIGHT);
    }
}
