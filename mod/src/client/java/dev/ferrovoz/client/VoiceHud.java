package dev.ferrovoz.client;

import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.client.resources.language.I18n;

import java.util.UUID;

/**
 * HUD: estado del micro y quién está hablando. Solo lee estado del núcleo.
 */
public final class VoiceHud {
    private static final int WHITE = 0xFFFFFFFF;
    private static final int GREEN = 0xFF55FF55;
    private static final int RED = 0xFFFF5555;
    private static final int GREY = 0xFFAAAAAA;
    private static final int YELLOW = 0xFFFFDD55;
    private static final int AQUA = 0xFF55FFFF;
    private static final int BACKGROUND = 0x80000000;

    private VoiceHud() {}

    public static void extract(GuiGraphicsExtractor g, DeltaTracker delta) {
        long h = FerrovozClient.handle();
        Minecraft mc = Minecraft.getInstance();
        if (mc.gui.hud.isHidden()) return;

        int x = 6;
        int y = 6;

        // Sin voz por un problema conocido (versión distinta, sin librería nativa...).
        String problem = FerrovozClient.problem();
        if (problem != null && h == 0) {
            String line = I18n.get(problem);
            g.fill(x - 2, y - 2, x + mc.font.width(line) + 2, y + 10, BACKGROUND);
            g.text(mc.font, line, x, y, RED, false);
            return;
        }
        if (h == 0) {
        // Voz desactivada por el jugador.
            if (FerrovozClient.voiceOffHere()) {
                String line = I18n.get("ferrovoz.hud.voice_off");
                g.fill(x - 2, y - 2, x + mc.font.width(line) + 2, y + 10, BACKGROUND);
                g.text(mc.font, line, x, y, GREY, false);
            }
            return;
        }

        // Estado propio
        String status;
        int color;
        if (FerrovozNative.clientMicSilent(h)) {
            status = I18n.get("ferrovoz.hud.mic_blocked");
            color = RED;
        } else if (!FerrovozNative.clientIsConnected(h)) {
            status = I18n.get("ferrovoz.hud.connecting");
            color = GREY;
        } else if (FerrovozClient.isDeafened()) {
            status = I18n.get("ferrovoz.hud.deafened");
            color = RED;
        } else if (FerrovozClient.isMuted()) {
            status = I18n.get("ferrovoz.hud.muted");
            color = RED;
        } else if (FerrovozNative.clientIsTalking(h)) {
            int talk = FerrovozClient.talkMode();
            status = talk == FerrovozNative.TALK_WHISPER ? I18n.get("ferrovoz.hud.whispering")
                    : talk == FerrovozNative.TALK_SHOUT ? I18n.get("ferrovoz.hud.shouting") : I18n.get("ferrovoz.hud.talking");
            color = talk == FerrovozNative.TALK_SHOUT ? YELLOW : GREEN;
        } else {
            status = FerrovozNative.clientGetMode(h) == 1 ? I18n.get("ferrovoz.hud.voice_activation") : I18n.get("ferrovoz.hud.push_to_talk");
            color = GREY;
        }
        float[] net = FerrovozNative.clientNetStats(h);
        if (net != null && net.length >= 4 && net[3] > 0.5f) {
            status += "  " + I18n.get("ferrovoz.hud.tunnel");
        }
        g.fill(x - 2, y - 2, x + mc.font.width(status) + 2, y + 10, BACKGROUND);
        g.text(mc.font, status, x, y, color, false);

        // Grupo actual
        String groupName = FerrovozNative.clientGroupName(h);
        if (groupName != null) {
            int flags = FerrovozNative.clientGroupFlags(h);
            String line = "⚑ " + groupName + ((flags & 2) != 0 ? " ★" : "")
                    + ((flags & 4) != 0 ? "  " + I18n.get("ferrovoz.hud.network") : "")
                    + ((flags & 1) != 0 ? "  " + I18n.get("ferrovoz.hud.owner") : "");
            y += 12;
            g.fill(x - 2, y - 2, x + mc.font.width(line) + 2, y + 10, BACKGROUND);
            g.text(mc.font, line, x, y, AQUA, false);
        }

        // Vúmetro
        float level = Math.min(1f, FerrovozNative.clientMicLevel(h) * 5f);
        y += 13;
        g.fill(x, y, x + 50, y + 2, BACKGROUND);
        g.fill(x, y, x + (int) (50 * level), y + 2, level > 0.5f ? GREEN : GREY);

        // Quién habla
        long[] speaking = FerrovozClient.speaking();
        if (speaking == null || mc.getConnection() == null) return;
        y += 6;
        for (int i = 0; i + 2 < speaking.length; i += 3) {
            UUID id = new UUID(speaking[i], speaking[i + 1]);
            boolean group = speaking[i + 2] != 0;
            PlayerInfo info = mc.getConnection().getPlayerInfo(id);
            // Los del grupo de red pueden estar en otro servidor (no salen en la tab list).
            String name = info != null ? info.getProfile().name()
                    : java.util.Objects.requireNonNullElse(FerrovozNative.clientMemberName(h, id.getMostSignificantBits(),
                            id.getLeastSignificantBits()), id.toString().substring(0, 8));
            String line = "♪ " + name + (group ? "  " + I18n.get("ferrovoz.hud.group") : "");
            g.fill(x - 2, y - 2, x + mc.font.width(line) + 2, y + 10, BACKGROUND);
            g.text(mc.font, line, x, y, group ? AQUA : WHITE, false);
            y += 12;
        }
    }
}
