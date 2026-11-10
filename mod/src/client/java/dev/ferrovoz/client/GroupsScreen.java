package dev.ferrovoz.client;

import dev.ferrovoz.FerrovozNative;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.client.multiplayer.PlayerInfo;
import net.minecraft.network.chat.Component;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Grupos de voz sin escribir comandos: ver el propio y quién está dentro, salir, crear
 * uno, unirse a los del servidor y, siendo el dueño, expulsar, fijar o borrar.
 *
 * <p>Cada acción se manda como su comando ({@code /voice group …}, o {@code /voicenet …}
 * en un grupo de red), así que el servidor comprueba lo mismo y avisa por chat igual que
 * si se escribiera a mano, y un plugin que cancele el evento sigue cancelándolo.
 *
 * <p>La lista se le pide al relé mientras la pantalla está abierta (ver
 * {@link FerrovozNative#clientRequestGroups}). Con un servidor anterior a 0.13 no llega
 * ninguna, y quedan crear y unirse escribiendo el nombre.
 */
final class GroupsScreen extends Screen {
    private static final int ROWS = 5;
    private static final int ROW = 24;
    private static final int WIDTH = 300;
    private static final int ACTION = 90;
    /** Primera fila, y de ahí para abajo todo sale de las constantes. */
    private static final int TOP = 46;
    private static final int PROBLEM_Y = TOP + ROWS * ROW + 4;
    private static final int LABEL_Y = PROBLEM_Y + 12;
    private static final int FIELDS_Y = LABEL_Y + 11;
    private static final long ASK_EVERY = 1000;
    private static final int WHITE = 0xFFFFFFFF;
    private static final int GREY = 0xFFAAAAAA;
    private static final int AQUA = 0xFF55FFFF;
    private static final int GOLD = 0xFFFFAA00;
    private static final int RED = 0xFFFF5555;

    /** Un grupo de la lista del servidor. */
    private record Entry(String name, int members, boolean locked, boolean persistent, boolean mine) {}

    /**
     * Un miembro del grupo propio. {@code name} null = no está en la lista de jugadores
     * (otro servidor de la red, o guardado en un grupo permanente y desconectado).
     * {@code isOwner} solo se sabe de uno mismo: el servidor no dice de quién es.
     */
    private record Member(UUID id, String name, boolean isOwner) {}

    private final Screen parent;
    private EditBox name;
    private EditBox password;
    private boolean focused;

    private final List<Entry> groups = new ArrayList<>();
    private final List<Member> members = new ArrayList<>();
    /** Grupo propio, null si ninguno, y sus banderas. */
    private String group;
    private boolean owner, persistent, network;
    private int page;
    private long asked;
    /** Lo que se mostraba al hacer los botones, para no rehacerlos en cada tick. */
    private String seen = "";
    /** Aviso en rojo: falta el nombre, falta la contraseña, hay espacios… */
    private Component problem;
    /** Filas que se están pintando; lo calcula {@link #init()}. */
    private int shown;

    GroupsScreen(Screen parent) {
        super(Component.translatable("ferrovoz.groups.title"));
        this.parent = parent;
    }

    // --- Datos ---

    private long h() {
        return FerrovozClient.handle();
    }

    private boolean canAct() {
        return this.minecraft.getConnection() != null && h() != 0;
    }

    private void read() {
        long h = h();
        groups.clear();
        group = h == 0 ? null : FerrovozNative.clientGroupName(h);
        int flags = h == 0 ? 0 : FerrovozNative.clientGroupFlags(h);
        owner = (flags & 1) != 0;
        persistent = (flags & 2) != 0;
        network = (flags & 4) != 0;

        String[] list = h == 0 ? null : FerrovozNative.clientGroupList(h);
        if (list != null) {
            for (String g : list) {
                // "nombre|miembros|candado|permanente|propio"; el nombre no puede llevar '|'.
                String[] f = g.split("\\|");
                if (f.length < 5) continue;
                groups.add(new Entry(f[0], number(f[1]), f[2].equals("1"), f[3].equals("1"), f[4].equals("1")));
            }
        }

        members.clear();
        if (group == null || h == 0) return;
        long[] ids = FerrovozNative.clientGroupMembers(h);
        UUID me = this.minecraft.player == null ? null : this.minecraft.player.getUUID();
        for (int i = 0; ids != null && i + 1 < ids.length; i += 2) {
            UUID id = new UUID(ids[i], ids[i + 1]);
            // En un grupo de red los miembros pueden estar en otro servidor: el nombre lo
            // manda el proxy, no sale de la lista de jugadores.
            String who = network ? FerrovozNative.clientMemberName(h, ids[i], ids[i + 1]) : nameOf(id);
            members.add(new Member(id, who, owner && id.equals(me)));
        }
        members.sort((a, b) -> {
            if (a.isOwner != b.isOwner) return a.isOwner ? -1 : 1;
            return text(a).compareToIgnoreCase(text(b));
        });
    }

    private String nameOf(UUID id) {
        if (this.minecraft.getConnection() == null) return null;
        PlayerInfo info = this.minecraft.getConnection().getPlayerInfo(id);
        return info == null ? null : info.getProfile().name();
    }

    private static String text(Member m) {
        return m.name == null ? "" : m.name;
    }

    private static int number(String s) {
        try {
            return Integer.parseInt(s);
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    /** Resumen de lo que se muestra: si cambia, hay que rehacer los botones. */
    private String signature() {
        StringBuilder sb = new StringBuilder();
        sb.append(group).append(owner).append(persistent).append(network).append('|').append(page);
        for (Entry e : groups) sb.append('\u0000').append(e);
        for (Member m : members) sb.append('\u0000').append(m.id).append(m.name).append(m.isOwner);
        return sb.toString();
    }

    private int pages() {
        int items = group != null ? members.size() : groups.size();
        return Math.max(1, (items + ROWS - 1) / ROWS);
    }

    // --- Pantalla ---

    @Override
    protected void init() {
        read();
        int x = (this.width - WIDTH) / 2;
        int pages = pages();
        page = Math.min(page, pages - 1);
        seen = signature();

        // Páginas arriba a la derecha, al lado de la cabecera, para no gastar una fila.
        if (pages > 1) {
            addRenderableWidget(arrow("◀", x + WIDTH - 52, 26, page > 0, b -> turn(-1)));
            addRenderableWidget(arrow("▶", x + WIDTH - 24, 26, page < pages - 1, b -> turn(1)));
        }

        // Una fila por miembro (con [Expulsar]) o por grupo (con [Unirse]).
        int y = TOP;
        int from = page * ROWS;
        shown = 0;
        if (group != null) {
            UUID me = this.minecraft.player == null ? null : this.minecraft.player.getUUID();
            for (int i = from; i < Math.min(members.size(), from + ROWS); i++) {
                Member m = members.get(i);
                // Sin nombre no hay comando con el que expulsarlo, y el comando se parte por
                // espacios (un nombre de Minecraft no los lleva, el de un proxy podría).
                boolean can = canAct() && owner && m.name != null && !m.id.equals(me) && !m.name.contains(" ");
                addRenderableWidget(button("ferrovoz.groups.kick", x + WIDTH - ACTION, y, ACTION, can,
                        b -> send("kick", m.name)));
                y += ROW;
                shown++;
            }
        } else {
            for (int i = from; i < Math.min(groups.size(), from + ROWS); i++) {
                Entry e = groups.get(i);
                // Un nombre con espacios solo lo crea un plugin y no cabe en el comando.
                boolean can = canAct() && !e.name.contains(" ");
                addRenderableWidget(button("ferrovoz.groups.join", x + WIDTH - ACTION, y, ACTION, can, b -> join(e)));
                y += ROW;
                shown++;
            }
        }

        y = LABEL_Y;
        if (group != null) {
            // [Salir] [Hacer fijo / Quitar fijo] [Borrar]
            int third = (WIDTH - 8) / 3;
            addRenderableWidget(button("ferrovoz.groups.leave", x, y, third, canAct(), b -> send("leave")));
            addRenderableWidget(button(persistent ? "ferrovoz.groups.unpin" : "ferrovoz.groups.pin",
                    x + third + 4, y, third, canAct() && owner && !network, b -> send(persistent ? "unpin" : "pin")));
            addRenderableWidget(button("ferrovoz.groups.delete", x + 2 * (third + 4), y, WIDTH - 2 * (third + 4),
                    canAct() && owner, b -> send("delete", group)));
            y += ROW;
        } else {
            // Los campos se crean una vez: así no se pierde lo escrito al rehacer los botones.
            int half = (WIDTH - 4) / 2;
            if (name == null) {
                name = new EditBox(this.font, x, FIELDS_Y, half, 20, Component.translatable("ferrovoz.groups.name"));
                name.setMaxLength(24);
                password = new EditBox(this.font, x + half + 4, FIELDS_Y, half, 20,
                        Component.translatable("ferrovoz.groups.password"));
                // La contraseña se ve mientras se escribe: el EditBox de Minecraft no tiene
                // forma de taparla que sirva en las dos versiones. Se borra al usarla.
                password.setMaxLength(32);
            }
            name.setPosition(x, FIELDS_Y);
            password.setPosition(x + half + 4, FIELDS_Y);
            addRenderableWidget(name);
            addRenderableWidget(password);
            if (!focused) {
                focused = true;
                setInitialFocus(name);
            }
            y = FIELDS_Y + ROW;
            addRenderableWidget(button("ferrovoz.groups.create", x, y, half, canAct(), b -> typed("create")));
            addRenderableWidget(button("ferrovoz.groups.join_typed", x + half + 4, y, half, canAct(), b -> typed("join")));
            y += ROW;
        }

        addRenderableWidget(Button.builder(Component.translatable("gui.done"), b -> onClose())
                .bounds(x, y, WIDTH, 20).build());
    }

    private void turn(int by) {
        page += by;
        rebuildWidgets();
    }

    /** Hay un campo con el cursor dentro. */
    private boolean typing() {
        return (name != null && name.isFocused()) || (password != null && password.isFocused());
    }

    @Override
    public void tick() {
        long h = h();
        if (h == 0) return;
        long now = System.currentTimeMillis();
        if (now - asked >= ASK_EVERY) {
            asked = now;
            FerrovozNative.clientRequestGroups(h);
        }
        // Solo se rehace si cambió algo: así no se pierde lo escrito ni el foco. Y nunca
        // mientras se escribe en un campo, que rehacerlos quitaría el cursor de ahí.
        read();
        if (!signature().equals(seen) && !typing()) {
            rebuildWidgets();
        }
    }

    @Override
    public void extractRenderState(GuiGraphicsExtractor g, int mouseX, int mouseY, float delta) {
        super.extractRenderState(g, mouseX, mouseY, delta);
        g.text(this.font, this.title, (this.width - this.font.width(this.title)) / 2, 14, WHITE, true);
        int x = (this.width - WIDTH) / 2;

        // Cabecera: el grupo propio, o que no hay ninguno.
        Component head;
        int color;
        if (h() == 0) {
            head = Component.translatable("ferrovoz.groups.no_voice");
            color = GREY;
        } else if (group == null) {
            head = Component.translatable(groups.isEmpty() ? "ferrovoz.groups.none" : "ferrovoz.groups.pick");
            color = GREY;
        } else {
            head = Component.translatable(network ? "ferrovoz.groups.mine_network" : "ferrovoz.groups.mine", group)
                    .append(persistent ? " ★" : "");
            color = AQUA;
        }
        g.text(this.font, head, x, 32, color, false);

        // `read()` corre en cada tick, así que la lista puede haber encogido desde que se
        // hicieron los botones: se pinta lo que siga habiendo y el tick los rehace.
        int from = page * ROWS;
        if (group != null) {
            for (int i = 0; i < Math.min(shown, members.size() - from); i++) {
                Member m = members.get(from + i);
                String line = (m.isOwner ? "★ " : "")
                        + (m.name != null ? m.name : Component.translatable("ferrovoz.groups.offline").getString());
                g.text(this.font, line, x, TOP + i * ROW + 6, m.name == null ? GREY : WHITE, false);
            }
            if (members.isEmpty()) {
                g.text(this.font, Component.translatable("ferrovoz.groups.only_you"), x, TOP + 6, GREY, false);
            }
        } else {
            for (int i = 0; i < Math.min(shown, groups.size() - from); i++) {
                Entry e = groups.get(from + i);
                String line = (e.persistent ? "★ " : "") + e.name + " · "
                        + Component.translatable("ferrovoz.groups.members", e.members).getString()
                        + (e.locked ? " · " + Component.translatable("ferrovoz.groups.locked").getString() : "");
                g.text(this.font, line, x, TOP + i * ROW + 6, e.persistent ? GOLD : WHITE, false);
            }
            // Nombre y contraseña van sin etiqueta propia: se escriben encima.
            int half = (WIDTH - 4) / 2;
            g.text(this.font, Component.translatable("ferrovoz.groups.name"), x + 2, LABEL_Y, GREY, false);
            g.text(this.font, Component.translatable("ferrovoz.groups.password"), x + half + 6, LABEL_Y, GREY, false);
        }
        if (problem != null) {
            g.text(this.font, problem, x, PROBLEM_Y, RED, false);
        }
    }

    // --- Acciones ---

    /** Crear o unirse con lo escrito en los campos. */
    private void typed(String sub) {
        if (name.getValue().trim().isEmpty()) {
            problem = Component.translatable("ferrovoz.groups.need_name");
            return;
        }
        if (!send(sub, name.getValue().trim(), password.getValue().trim())) return;
        name.setValue("");
        password.setValue("");
    }

    /** Unirse a uno de la lista. Con candado, la contraseña va en su campo. */
    private void join(Entry e) {
        String pass = password == null ? "" : password.getValue().trim();
        if (e.locked && pass.isEmpty()) {
            problem = Component.translatable("ferrovoz.groups.need_password");
            return;
        }
        if (!send("join", e.name, pass)) return;
        if (password != null) password.setValue("");
    }

    /**
     * Manda el comando que corresponda. Devuelve false, con el aviso puesto, si algo de lo
     * escrito no cabe en un comando (que se parte por espacios).
     */
    private boolean send(String sub, String... args) {
        problem = null;
        if (this.minecraft.player == null || this.minecraft.getConnection() == null) return false;
        StringBuilder cmd = new StringBuilder(network ? "voicenet " : "voice group ");
        cmd.append(sub);
        for (String a : args) {
            if (a == null || a.isEmpty()) continue;
            if (a.contains(" ")) {
                problem = Component.translatable("ferrovoz.groups.no_spaces");
                return false;
            }
            cmd.append(' ').append(a);
        }
        this.minecraft.player.connection.sendCommand(cmd.toString());
        asked = 0; // pedir la lista ya, sin esperar al siguiente segundo
        return true;
    }

    private Button button(String key, int x, int y, int w, boolean active, Button.OnPress onPress) {
        Button b = Button.builder(Component.translatable(key), onPress).bounds(x, y, w, 20).build();
        b.active = active;
        return b;
    }

    private Button arrow(String glyph, int x, int y, boolean active, Button.OnPress onPress) {
        Button b = Button.builder(Component.literal(glyph), onPress).bounds(x, y, 24, 20).build();
        b.active = active;
        return b;
    }

    @Override
    public void onClose() {
        this.minecraft.gui.setScreen(parent);
    }
}
