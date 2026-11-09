//! Sonido de los plugins (canales) y escucha del micro para la API.

use super::*;

impl VoiceServer {
    // Escucha del micro (API)

    /// Copia (o deja de copiar) las tramas del micro para [`Self::poll_mic`]. Solo las
    /// que se reenvían: un silenciado no sale.
    pub fn set_mic_tap(&self, on: bool) {
        let mut st = self.state();
        st.mic_tap = on;
        if !on {
            st.mic_frames.clear();
        }
    }

    pub fn poll_mic(&self) -> Vec<MicFrame> {
        self.state().mic_frames.drain(..).collect()
    }

    // Canales de audio (API)

    /// Canal nuevo. `None` si ya hay [`channels::MAX_CHANNELS`] o Opus falla.
    pub fn create_channel(&self, target: Target, range: f32, bitrate: i32, music: bool) -> Option<PlayerId> {
        let channel = Channel::new(target, range, bitrate, music).ok()?;
        let mut st = self.state();
        if st.channels.len() >= channels::MAX_CHANNELS {
            return None;
        }
        let id = channels::new_id();
        st.channels.insert(id, channel);
        Some(id)
    }

    pub fn set_channel_target(&self, id: &PlayerId, target: Target) -> bool {
        self.state().channels.get_mut(id).map(|c| c.target = target).is_some()
    }

    pub fn set_channel_range(&self, id: &PlayerId, range: f32) -> bool {
        self.state().channels.get_mut(id).map(|c| c.range = channels::clamp_range(range)).is_some()
    }

    pub fn remove_channel(&self, id: &PlayerId) -> bool {
        self.state().channels.remove(id).is_some()
    }

    pub fn channel_count(&self) -> usize {
        self.state().channels.len()
    }

    /// Codifica 20 ms de audio (960 muestras mono, -1..1) y lo envía por el canal.
    /// Codifica en el hilo que llama, sin bloquear al relé.
    pub fn channel_send_pcm(&self, id: &PlayerId, pcm: &[f32]) -> bool {
        if pcm.len() != protocol::FRAME_SAMPLES {
            return false;
        }
        let Some(encoder) = self.state().channels.get(id).map(|c| c.encoder.clone()) else { return false };
        let opus = {
            let mut enc = encoder.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match enc.encode(pcm) {
                Ok(o) => o.to_vec(),
                Err(_) => return false,
            }
        };
        self.channel_send_opus(id, &opus)
    }

    /// Envía una trama Opus ya codificada (20 ms, 48 kHz, mono).
    pub fn channel_send_opus(&self, id: &PlayerId, opus: &[u8]) -> bool {
        if opus.is_empty() || opus.len() > crate::codec::MAX_OPUS_FRAME {
            return false;
        }
        let mut out = Vec::new();
        let mut jobs = Vec::new();
        let sent = self.shared.route_channel(id, opus, &mut out, &mut jobs);
        self.shared.send_udp(out);
        let n = crate::fanout::seal_and_send(&self.shared.socket, jobs);
        self.shared.stats.packets_out.fetch_add(n as u64, Ordering::Relaxed);
        sent
    }
}

impl Shared {
    /// Envía una trama de un canal a quien le toque. false si el canal no existe.
    pub(super) fn route_channel(&self, id: &PlayerId, opus: &[u8], out: &mut Outgoing, jobs: &mut Vec<SealJob>) -> bool {
        let mut guard = lock(&self.state);
        let st = &mut *guard;
        let Some(channel) = st.channels.get_mut(id) else { return false };
        channel.seq = channel.seq.wrapping_add(1);
        let (seq, range, target) = (channel.seq, channel.range, channel.target.clone());
        let origin = match &target {
            Target::Point { pos, dim } => Some((*pos, *dim)),
            Target::Follow(p) => match st.players.get(p) {
                Some(p) => Some((p.pos, p.dimension)),
                None => return true, // no está: se pierde la trama, el canal sigue
            },
            Target::Listeners(_) | Target::Group(_) => None,
        };
        let group_id = match &target {
            Target::Group(name) => match st.find_group(name) {
                Some(g) => Some(g),
                None => return true,
            },
            _ => None,
        };
        let rules = st.role_rules;
        let recipients: Vec<PlayerId> = st
            .players
            .iter()
            .filter(|(pid, l)| {
                if matches!(l.transport, Transport::Pending) {
                    return false;
                }
                match (&target, origin) {
                    (_, Some((pos, dim))) => {
                        if l.dimension != dim || !rules.can_hear(Role::Alive, l.role) {
                            return false;
                        }
                        let d = [pos[0] - l.pos[0], pos[1] - l.pos[1], pos[2] - l.pos[2]];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= (range as f64) * (range as f64)
                    }
                    (Target::Listeners(set), None) => set.contains(*pid),
                    (Target::Group(_), None) => l.group == group_id,
                    _ => false,
                }
            })
            .map(|(pid, _)| *pid)
            .collect();
        let template = Arc::new(AudioDown {
            sender: *id,
            seq,
            rel: [0.0; 3],
            range: if origin.is_some() { range } else { 0.0 },
            group: origin.is_none(),
            underwater: false,
            env: Environment::default(),
            occlusion: OCCLUSION_UNKNOWN,
            opus: opus.to_vec(),
        }
        .encode());
        for pid in recipients {
            let listener = st.players.get_mut(&pid).unwrap();
            let rel = match origin {
                Some((pos, _)) => [
                    (pos[0] - listener.pos[0]) as f32,
                    (pos[1] - listener.pos[1]) as f32,
                    (pos[2] - listener.pos[2]) as f32,
                ],
                None => [0.0; 3],
            };
            match listener.transport {
                Transport::Udp(addr) => {
                    listener.tx_counter += 1;
                    let body = Body::Audio { template: template.clone(), rel, occlusion: OCCLUSION_UNKNOWN };
                    jobs.push(SealJob { addr, key: listener.key, counter: listener.tx_counter, body });
                }
                _ => {
                    let body = AudioDown::for_listener(&template, rel, OCCLUSION_UNKNOWN);
                    let pkt = listener.seal(PacketType::AudioDown, &body);
                    deliver(&self.stats, listener, pkt, out);
                }
            }
        }
        true
    }
}
