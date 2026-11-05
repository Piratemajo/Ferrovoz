package dev.ferrovoz.client;

import dev.ferrovoz.FerrovozNative;
import dev.ferrovoz.SoundGeometry;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

/**
 * Calcula en el cliente, con los bloques cargados, lo que afecta al sonido y se lo
 * pasa al núcleo. No cuesta nada al servidor. Los cálculos están en
 * {@link SoundGeometry}; aquí solo se traducen los bloques de Minecraft.
 *
 * <ul>
 *   <li>Oclusión en línea recta entre oyente y hablante.</li>
 *   <li>Si está tapado, camino alternativo (puerta, ventana, esquina).</li>
 *   <li>Acústica de la sala con rayos: reverb, RT60 y reflexiones tempranas.</li>
 * </ul>
 */
final class WorldSensor {
    private static final int OCCLUSION_EVERY = 2;
    private static final int ACOUSTICS_EVERY = 10;
    /** Bloques que explora como mucho la búsqueda de camino (~1-2 ms). */
    private static final int PATH_NODES = 2000;
    /** Longitud máxima de un camino. */
    private static final double PATH_MAX_LENGTH = 64;

    private int ticks;
    /** Hablantes tapados; se les busca camino por turnos, uno por tick. */
    private final List<UUID> occluded = new ArrayList<>();
    private int nextPath;

    /**
     * @param eye  posición de los oídos: el jugador, o la cámara en una repetición
     * @param self entidad que escucha (se excluye)
     */
    void tick(Minecraft mc, long handle, long[] speaking, Vec3 eye, Entity self) {
        ClientLevel level = mc.level;
        if (level == null || handle == 0) return;
        ticks++;
        LevelBlocks blocks = new LevelBlocks(level);

        if (ticks % ACOUSTICS_EVERY == 0) {
            SoundGeometry.Acoustics a = SoundGeometry.measure(blocks, eye.x, eye.y, eye.z);
            FerrovozNative.clientSetAcoustics(handle, a.wet(), a.room(), a.damp(), a.reflections());
        }

        if (speaking == null) return;
        if (ticks % OCCLUSION_EVERY == 0) {
            occluded.clear();
            for (int i = 0; i + 2 < speaking.length; i += 3) {
                UUID id = new UUID(speaking[i], speaking[i + 1]);
                boolean group = speaking[i + 2] != 0;
                Player other = find(level, id);
                if (other == null || other == self || group) {
                    // Fuera de la render distance: la oclusión la calcula el servidor.
                    continue;
                }
                Vec3 to = other.getEyePosition();
                float occ = SoundGeometry.occlusion(blocks, eye.x, eye.y, eye.z, to.x, to.y, to.z);
                FerrovozNative.clientSetOcclusion(handle, speaking[i], speaking[i + 1], occ);
                if (occ > 0.05f) {
                    occluded.add(id);
                } else {
                    FerrovozNative.clientSetSoundPath(handle, speaking[i], speaking[i + 1], false, 0, 0, 0, 0, 0);
                }
            }
        }

        // Como mucho un camino por tick.
        if (!occluded.isEmpty()) {
            UUID id = occluded.get(nextPath++ % occluded.size());
            Player other = find(level, id);
            if (other != null) {
                Vec3 to = other.getEyePosition();
                SoundGeometry.Path p = SoundGeometry.findPath(blocks, eye.x, eye.y, eye.z, to.x, to.y, to.z, PATH_NODES, PATH_MAX_LENGTH);
                long msb = id.getMostSignificantBits(), lsb = id.getLeastSignificantBits();
                if (p != null) {
                    FerrovozNative.clientSetSoundPath(handle, msb, lsb, true,
                            (float) p.dirX(), (float) p.dirY(), (float) p.dirZ(), (float) p.length(), p.corners());
                } else {
                    FerrovozNative.clientSetSoundPath(handle, msb, lsb, false, 0, 0, 0, 0, 0);
                }
            }
        }
    }

    private static Player find(ClientLevel level, UUID id) {
        for (Player p : level.players()) {
            if (p.getUUID().equals(id)) return p;
        }
        return null;
    }

    /** Bloques del mundo para el sonido, con caché durante el tick. */
    private static final class LevelBlocks implements SoundGeometry.Blocks {
        private final Level level;
        private final BlockPos.MutableBlockPos pos = new BlockPos.MutableBlockPos();
        private final Map<Long, float[]> cache = new HashMap<>();

        LevelBlocks(Level level) {
            this.level = level;
        }

        private float[] info(int x, int y, int z) {
            long key = BlockPos.asLong(x, y, z);
            float[] v = cache.get(key);
            if (v == null) {
                BlockState st = level.getBlockState(pos.set(x, y, z));
                v = new float[] {wall(st), absorption(st)};
                cache.put(key, v);
            }
            return v;
        }

        private float wall(BlockState st) {
            if (st.isAir()) return 0f;
            // Puertas, trampillas y puertas de valla: abiertas no tapan; cerradas, algo.
            if (st.hasProperty(BlockStateProperties.OPEN)) {
                return st.getValue(BlockStateProperties.OPEN) ? 0f : 0.5f;
            }
            if (st.isSolidRender(level, pos)) return 1f;
            // Plantas, antorchas, agua, hojas...: no tapan.
            if (st.is(BlockTags.LEAVES) || st.getCollisionShape(level, pos).isEmpty()) return 0f;
            // Cristal, paneles, vallas, losas: algo.
            return 0.5f;
        }

        private static float absorption(BlockState st) {
            SoundType t = st.getSoundType();
            if (t == SoundType.WOOL) return 0.7f;
            if (t == SoundType.SNOW) return 0.55f;
            if (t == SoundType.MOSS || t == SoundType.MOSS_CARPET) return 0.5f;
            if (t == SoundType.GRASS) return 0.35f;
            if (t == SoundType.SAND || t == SoundType.GRAVEL) return 0.25f;
            if (t == SoundType.WOOD) return 0.15f;
            if (t == SoundType.GLASS || t == SoundType.METAL) return 0.03f;
            return 0.05f; // piedra, pizarra, ladrillo...
        }

        @Override
        public float wall(int x, int y, int z) {
            return info(x, y, z)[0];
        }

        @Override
        public float absorption(int x, int y, int z) {
            return info(x, y, z)[1];
        }
    }
}
