package dev.ferrovoz.client;

import com.mojang.blaze3d.vertex.PoseStack;
import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.resources.language.I18n;
import net.minecraft.network.chat.Component;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.phys.Vec3;
import org.joml.Matrix4f;

import java.util.UUID;

/**
 * Iconos sobre la cabeza, mirando a la cámara como los nombres: ♪ en quien habla y,
 * en los demás, su estado de voz si el servidor lo envía.
 *
 * <ul>
 *   <li>{@link #submitWorld}: con la pose en la cámara (Fabric
 *       {@code LevelRenderEvents.COLLECT_SUBMITS}, NeoForge {@code SubmitCustomGeometryEvent}).</li>
 *   <li>{@link #submitForAvatar}: después de dibujar cada jugador, con la pose en sus
 *       pies (Forge {@code RenderAvatarEvent.Post}).</li>
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
        return mc.level == null || mc.gui.hud.isHidden();
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

    /** Fabric y NeoForge: pose en la cámara. */
    public static void submitWorld(Vec3 camera, PoseStack pose, SubmitNodeCollector collector) {
        Minecraft mc = Minecraft.getInstance();
        if (hidden(mc)) return;
        long[] speaking = FerrovozClient.speaking();
        long[] states = FerrovozClient.peerStates();
        if ((speaking == null || speaking.length == 0) && states == null) return;
        float partial = mc.getDeltaTracker().getGameTimeDeltaPartialTick(false);

        for (Player p : mc.level.players()) {
            if (p == mc.player || p == mc.getCameraEntity() || p.isInvisible()) continue;
            Icon icon = iconFor(p, speaking, states);
            if (icon == null) continue;
            // Posición interpolada para que no tiemble.
            double x = Mth.lerp(partial, p.xo, p.getX());
            double y = Mth.lerp(partial, p.yo, p.getY()) + p.getBbHeight() + ABOVE_HEAD;
            double z = Mth.lerp(partial, p.zo, p.getZ());
            double dx = x - camera.x, dy = y - camera.y, dz = z - camera.z;
            pose.pushPose();
            pose.translate(dx, dy, dz);
            draw(pose, collector, mc.font, dx, dy, dz, icon);
            pose.popPose();
        }
    }

    /**
     * Forge: se acaba de dibujar un jugador en {@code (x, y, z)} (pies, interpolado) y
     * la pose está ahí.
     */
    public static void submitForAvatar(double x, double y, double z, float height, Vec3 camera,
                                       PoseStack pose, SubmitNodeCollector collector) {
        Minecraft mc = Minecraft.getInstance();
        if (hidden(mc)) return;
        long[] speaking = FerrovozClient.speaking();
        long[] states = FerrovozClient.peerStates();
        if ((speaking == null || speaking.length == 0) && states == null) return;
        float partial = mc.getDeltaTracker().getGameTimeDeltaPartialTick(false);

        for (Player p : mc.level.players()) {
            if (p == mc.player || p == mc.getCameraEntity() || p.isInvisible()) continue;
            // El evento no dice qué jugador es: se busca por posición.
            double px = Mth.lerp(partial, p.xo, p.getX());
            double py = Mth.lerp(partial, p.yo, p.getY());
            double pz = Mth.lerp(partial, p.zo, p.getZ());
            if (Math.abs(px - x) > 0.1 || Math.abs(py - y) > 0.1 || Math.abs(pz - z) > 0.1) continue;
            Icon icon = iconFor(p, speaking, states);
            if (icon == null) return;
            double up = height + ABOVE_HEAD;
            double dx = x - camera.x, dy = y + up - camera.y, dz = z - camera.z;
            pose.pushPose();
            pose.translate(0, up, 0);
            draw(pose, collector, mc.font, dx, dy, dz, icon);
            pose.popPose();
            return;
        }
    }

    /**
     * Dibuja el icono en el origen de la pose, girado hacia la cámara.
     * {@code (dx, dy, dz)}: de la cámara al icono.
     */
    private static void draw(PoseStack pose, SubmitNodeCollector collector, Font font,
                             double dx, double dy, double dz, Icon icon) {
        double distance = Math.sqrt(dx * dx + dy * dy + dz * dz);
        if (distance > (icon.small() ? STATUS_DISTANCE : MAX_DISTANCE) || distance < 0.5) return;
        // Girar para que el texto (+Z local) mire a la cámara.
        double horizontal = Math.sqrt(dx * dx + dz * dz);
        float yaw = (float) Math.atan2(-dx, -dz);
        float pitch = (float) Math.atan2(dy, horizontal);
        // En 26.3 mulPose solo acepta matrices: Y y luego X.
        pose.mulPose(new Matrix4f().rotationY(yaw).rotateX(pitch));
        // Los de estado, más pequeños que el ♪.
        float scale = icon.small() ? SCALE * 0.8f : SCALE;
        pose.scale(scale, -scale, scale);

        float width = font.width(icon.text());
        collector.submitText(
                pose,
                -width / 2f, 0f,
                Component.literal(icon.text()).getVisualOrderText(),
                false,
                Font.DisplayMode.NORMAL,
                FULL_BRIGHT,
                icon.color(),
                BACKGROUND,
                0);
    }
}
