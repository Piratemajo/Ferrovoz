//! Multitudes: qué voces van por separado en cada zona y la mezcla del murmullo.

use super::*;

impl Shared {
    /// Hilo de multitudes: cada 20 ms mezcla el murmullo de las zonas con multitud y
    /// cada 100 ms decide qué voces van por separado en cada zona.
    pub(super) fn crowd_loop(self: &Arc<Self>) {
        let mut mixer = crowd::Mixer::new();
        let mut tick: u32 = 0;
        let mut next = Instant::now();
        while self.running.load(Ordering::Relaxed) {
            next += FRAME;
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else if now - next > FRAME * 5 {
                next = now; // muy atrasado (máquina parada): no intentar recuperar
            }
            tick = tick.wrapping_add(1);
            let step = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if tick.is_multiple_of(CROWD_PLAN_EVERY) {
                    self.plan_crowds();
                    self.sweep_speaking();
                    mixer.cleanup(Instant::now());
                }
                self.mix_crowds(&mut mixer, tick);
            }));
            if step.is_err() {
                // Se empieza con el mezclador limpio; la voz por separado sigue igual.
                log::error!("voice relay: crowd mixing failed; restarting it");
                mixer = crowd::Mixer::new();
            }
        }
    }

    /// Para cada zona con oyentes, si le llegan más de `max_voices` voces, cuáles
    /// van por separado.
    pub(super) fn plan_crowds(&self) {
        let mut st = lock(&self.state);
        if st.max_voices == 0 {
            st.crowd_plans.clear();
            return;
        }
        let now = Instant::now();
        let rules = st.role_rules;
        // Los de prioridad no cuentan: van siempre por separado.
        let speakers: Vec<crowd::Speaker> = st
            .players
            .iter()
            .filter(|(id, p)| {
                p.group.is_none()
                    && !p.priority
                    && p.can_speak
                    && !st.admin_muted.contains(*id)
                    && rules.can_be_heard(p.role)
                    && p.last_audio.is_some_and(|t| now.duration_since(t) < ACTIVE_SPEAKER)
            })
            .map(|(id, p)| crowd::Speaker { id: *id, pos: p.pos, dim: p.dimension, range: p.talk_range })
            .collect();
        if speakers.len() <= st.max_voices {
            st.crowd_plans.clear();
            return;
        }
        let cells: HashSet<Cell> = st
            .players
            .values()
            .filter(|p| !matches!(p.transport, Transport::Pending))
            .map(|p| Cell::of(p.pos, p.dimension))
            .collect();
        st.crowd_plans = crowd::plan(cells, &speakers, st.max_voices);
    }

    /// Mezcla y envía el murmullo de cada zona con multitud.
    pub(super) fn mix_crowds(&self, mixer: &mut crowd::Mixer, tick: u32) {
        // 1. Con el lock, solo copiar lo necesario.
        let now = Instant::now();
        let (frames, cells) = {
            let mut st = lock(&self.state);
            if st.crowd_plans.is_empty() && st.crowd_stash.is_empty() {
                return;
            }
            let frames = st.crowd_stash.take(now);
            let cells: Vec<(Cell, HashSet<PlayerId>)> =
                st.crowd_plans.iter().map(|(c, near)| (*c, near.clone())).collect();
            (frames, cells)
        };
        if frames.is_empty() || cells.is_empty() {
            return;
        }

        // 2. Sin el lock: decodificar cada voz una vez y mezclar cada zona.
        let voices = mixer.decode(frames, now);
        let mixes: HashMap<Cell, crowd::CellMix> = cells
            .into_iter()
            .filter_map(|(cell, near)| mixer.mix(cell, &near, &voices, now).map(|m| (cell, m)))
            .collect();
        if mixes.is_empty() {
            return;
        }

        // 3. Con el lock: cifrar para cada oyente de esas zonas y enviar.
        let mut out = Vec::new();
        {
            let mut st = lock(&self.state);
            let rules = st.role_rules;
            let range = st.ranges.normal;
            for p in st.players.values_mut() {
                if matches!(p.transport, Transport::Pending) || !rules.can_hear(Role::Alive, p.role) {
                    continue;
                }
                let Some(mix) = mixes.get(&Cell::of(p.pos, p.dimension)) else { continue };
                // Dirección hacia la multitud, a una distancia que el cliente oye a
                // volumen completo (la mezcla ya lleva la atenuación de cada voz).
                let mut rel = [mix.centroid[0] - p.pos[0], mix.centroid[1] - p.pos[1], mix.centroid[2] - p.pos[2]];
                let len = (rel[0] * rel[0] + rel[1] * rel[1] + rel[2] * rel[2]).sqrt();
                if len > CROWD_REL_DISTANCE {
                    for r in rel.iter_mut() {
                        *r *= CROWD_REL_DISTANCE / len;
                    }
                }
                let down = AudioDown {
                    sender: CROWD_ID,
                    // El mismo número en todas las zonas: al cambiar de zona el
                    // cliente sigue el murmullo sin saltos.
                    seq: tick,
                    rel: [rel[0] as f32, rel[1] as f32, rel[2] as f32],
                    range,
                    group: false,
                    underwater: false,
                    env: Environment::default(),
                    occlusion: OCCLUSION_UNKNOWN,
                    opus: mix.opus.clone(),
                };
                let pkt = p.seal(PacketType::AudioDown, &down.encode());
                deliver(&self.stats, p, pkt, &mut out);
            }
        }
        self.send_udp(out);
    }
}
