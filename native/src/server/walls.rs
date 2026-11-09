//! Paredes calculadas en el servidor: qué parejas las necesitan y sus valores.

use super::*;

impl VoiceServer {
    /// Pares (hablante, oyente) de proximidad activos ahora, para que Java calcule
    /// la oclusión con el mundo del servidor.
    /// Parejas (hablante, oyente) que se oyen por separado, de más cerca a más lejos
    /// y como mucho 2000.
    pub fn active_pairs(&self) -> Vec<(PlayerId, PlayerId)> {
        let st = self.state();
        let mut pairs = st.candidate_pairs(Instant::now());
        if pairs.len() > ACTIVE_PAIRS_MAX {
            pairs.select_nth_unstable_by(ACTIVE_PAIRS_MAX, |a, b| a.0.total_cmp(&b.0));
            pairs.truncate(ACTIVE_PAIRS_MAX);
        }
        pairs.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        pairs.into_iter().map(|(_, s, l)| (s, l)).collect()
    }

    pub fn set_occlusion(&self, speaker: &PlayerId, listener: &PlayerId, amount: f32) {
        self.set_occlusions(&[(*speaker, *listener, amount)]);
    }

    /// Varias de golpe (un solo lock).
    pub fn set_occlusions(&self, values: &[(PlayerId, PlayerId, f32)]) {
        let now = Instant::now();
        let mut st = self.state();
        for (speaker, listener, amount) in values {
            let q = (amount.clamp(0.0, 1.0) * 254.0).round() as u8;
            st.occlusion.insert((*speaker, *listener), (q, now));
        }
    }
}

impl State {
    /// Parejas (distancia², hablante, oyente) que necesitan paredes: el oyente le oye
    /// por proximidad y no en el murmullo.
    ///
    /// Se recorre por oyente. En una zona con multitud, el oyente solo oye por separado
    /// a las voces de su plan (unas 24) y a quien tenga prioridad, así que se miran solo
    /// esas en vez de todas las que hablan: con 500 juntos y 300 hablando, de 150 000
    /// parejas a unas 12 000.
    pub(super) fn candidate_pairs(&self, now: Instant) -> Vec<(f64, PlayerId, PlayerId)> {
        let speakers: HashMap<PlayerId, &Player> = self
            .players
            .iter()
            .filter(|(_, s)| {
                s.group.is_none()
                    && s.last_audio.is_some_and(|t| now.duration_since(t) < ACTIVE_SPEAKER)
                    && self.role_rules.can_be_heard(s.role)
            })
            .map(|(id, s)| (*id, s))
            .collect();
        if speakers.is_empty() {
            return Vec::new();
        }
        let priority: Vec<PlayerId> = speakers.iter().filter(|(_, s)| s.priority).map(|(id, _)| *id).collect();
        let mut pairs = Vec::new();
        let consider = |sid: &PlayerId, lid: &PlayerId, l: &Player, pairs: &mut Vec<(f64, PlayerId, PlayerId)>| {
            let Some(s) = speakers.get(sid) else { return };
            if sid == lid || l.dimension != s.dimension || !self.role_rules.can_hear(s.role, l.role) {
                return;
            }
            let d = [s.pos[0] - l.pos[0], s.pos[1] - l.pos[1], s.pos[2] - l.pos[2]];
            let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            let r = s.talk_range as f64;
            if d2 <= r * r {
                pairs.push((d2, *sid, *lid));
            }
        };
        for (lid, l) in &self.players {
            if matches!(l.transport, Transport::Pending) {
                continue;
            }
            match self.crowd_plans.get(&Cell::of(l.pos, l.dimension)) {
                // Con multitud: su plan y los que tienen prioridad (que no van al murmullo).
                Some(near) => {
                    for sid in near.iter().chain(priority.iter().filter(|p| !near.contains(*p))) {
                        consider(sid, lid, l, &mut pairs);
                    }
                }
                None => {
                    for sid in speakers.keys() {
                        consider(sid, lid, l, &mut pairs);
                    }
                }
            }
        }
        pairs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `n` jugadores juntos en rejilla (2 bloques), con UDP, y 3 de cada 5 hablando.
    /// El primero que habla tiene prioridad.
    fn crowd(n: u32) -> VoiceServer {
        let s = VoiceServer::start("127.0.0.1:0".parse().unwrap(), 48.0).unwrap();
        s.set_max_voices(24); // como en ferrovoz.properties
        for i in 0..n {
            let mut id = [0u8; 16];
            id[..4].copy_from_slice(&i.to_le_bytes());
            s.add_player(id);
            s.update_player(&id, [(i % 10) as f64 * 2.0, 64.0, (i / 10) as f64 * 2.0], 1);
            let mut st = s.state();
            let p = st.players.get_mut(&id).unwrap();
            p.transport = Transport::Udp("127.0.0.1:9".parse().unwrap());
            if i % 5 < 3 {
                p.last_audio = Some(Instant::now() + Duration::from_secs(3600));
                p.talk_range = 48.0;
                p.priority = i == 0;
            }
        }
        s.shared.plan_crowds();
        s
    }

    /// Lo que salía recorriendo todas las parejas, como antes.
    fn full_scan(st: &State) -> Vec<(u64, PlayerId, PlayerId)> {
        let mut out = Vec::new();
        for (sid, sp) in st.players.iter().filter(|(_, p)| p.last_audio.is_some()) {
            for (lid, l) in &st.players {
                if sid == lid
                    || (!sp.priority && st.crowd_plans.get(&Cell::of(l.pos, l.dimension)).is_some_and(|n| !n.contains(sid)))
                {
                    continue;
                }
                let d = [sp.pos[0] - l.pos[0], sp.pos[1] - l.pos[1], sp.pos[2] - l.pos[2]];
                let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                if d2 <= 48.0 * 48.0 {
                    out.push((d2.to_bits(), *sid, *lid));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn candidate_pairs_match_the_full_scan() {
        let s = crowd(200);
        let st = s.state();
        assert!(!st.crowd_plans.is_empty(), "hay multitud");
        let mut fast: Vec<(u64, PlayerId, PlayerId)> =
            st.candidate_pairs(Instant::now()).into_iter().map(|(d, a, b)| (d.to_bits(), a, b)).collect();
        fast.sort();
        let slow = full_scan(&st);
        assert!(!slow.is_empty());
        assert_eq!(fast, slow);
        let mut host = [0u8; 16];
        host[..4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(fast.iter().filter(|p| p.1 == host).count(), 199, "al anfitrión le buscan paredes todos");
    }

    #[test]
    #[ignore = "medida: cargo test --release --lib pairs_tests -- --ignored --nocapture"]
    fn active_pairs_cost_with_500_together() {
        let s = crowd(500);
        let t = Instant::now();
        let slow = full_scan(&s.state()).len();
        let slow_ms = t.elapsed().as_secs_f64() * 1e3;
        for _ in 0..3 {
            let t = Instant::now();
            let n = s.active_pairs().len();
            println!("active_pairs: {n} de {slow} parejas en {:.2} ms (recorriéndolas todas: {slow_ms:.2} ms)", t.elapsed().as_secs_f64() * 1e3);
        }
    }
}
