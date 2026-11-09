//! `ferrovoz-data.txt`: grupos permanentes y silencios, escritos en su propio hilo.

use super::*;

/// Escribe el archivo de datos en su propio hilo, para que la voz no espere al disco
/// (en un hosting con disco lento, varios ms por escritura). Si llegan varios cambios
/// seguidos, solo se escribe el último. Al parar el relé escribe lo pendiente.
#[derive(Default)]
pub(super) struct Saver {
    queue: Mutex<SaveQueue>,
    wake: Condvar,
}

#[derive(Default)]
pub(super) struct SaveQueue {
    next: Option<(PathBuf, String)>,
    closed: bool,
}

impl Saver {
    pub(super) fn submit(&self, path: PathBuf, text: String) {
        lock(&self.queue).next = Some((path, text));
        self.wake.notify_one();
    }

    pub(super) fn close(&self) {
        lock(&self.queue).closed = true;
        self.wake.notify_one();
    }

    pub(super) fn run(&self) {
        loop {
            let job = {
                let mut q = lock(&self.queue);
                loop {
                    if let Some(job) = q.next.take() {
                        break Some(job);
                    }
                    if q.closed {
                        break None;
                    }
                    q = self.wake.wait(q).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };
            let Some((path, text)) = job else { return };
            // Escritura atómica (temporal + rename): un corte a mitad no deja el archivo roto.
            let tmp = path.with_extension("tmp");
            if let Err(e) = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, &path)) {
                log::error!("could not save {}: {e}", path.display());
            }
        }
    }
}

impl VoiceServer {
    /// Fichero de silencios y grupos permanentes. Lo carga si existe.
    pub fn set_data_file(&self, path: &Path) -> io::Result<()> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut st = self.state();
        st.data_file = Some(path.to_owned());
        if st.load(&text) {
            log::info!("voice groups: protected the passwords stored in plain text by older versions");
            st.save();
        }
        Ok(())
    }
}

impl State {
    pub(super) fn serialize(&self) -> String {
        let mut s = String::from("# Ferrovoz: silencios y grupos permanentes. Edítalo solo con el servidor apagado.\n");
        let mut mutes: Vec<String> = self.admin_muted.iter().map(protocol::uuid_to_string).collect();
        mutes.sort();
        for m in mutes {
            s.push_str(&format!("mute\t{m}\n"));
        }
        let mut groups: Vec<&Group> = self.groups.values().filter(|g| g.persistent).collect();
        groups.sort_by_key(|g| g.name.to_lowercase());
        for g in groups {
            let owner = g.owner.as_ref().map(protocol::uuid_to_string).unwrap_or_else(|| "-".into());
            let password = g.password.as_ref().map(PasswordHash::encode).unwrap_or_default();
            s.push_str(&format!("group\t{}\t{}\t{}\n", g.name, owner, password));
            let mut members: Vec<String> = g.saved_members.iter().map(protocol::uuid_to_string).collect();
            members.sort();
            for m in members {
                s.push_str(&format!("member\t{}\t{m}\n", g.name));
            }
        }
        s
    }

    /// Devuelve true si había contraseñas antiguas en claro: se han protegido y hay que
    /// guardar.
    pub(super) fn load(&mut self, text: &str) -> bool {
        let mut upgraded = false;
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["mute", id] => {
                    if let Some(id) = protocol::uuid_from_str(id) {
                        self.admin_muted.insert(id);
                    }
                }
                ["group", name, owner, password] if valid_text(name, MAX_GROUP_NAME) && self.find_group(name).is_none() => {
                    let id = self.next_group;
                    self.next_group += 1;
                    self.groups.insert(
                        id,
                        Group {
                            name: (*name).to_owned(),
                            password: match *password {
                                "" => None,
                                p if PasswordHash::is_encoded(p) => match PasswordHash::decode(p) {
                                    Some(h) => Some(h),
                                    None => {
                                        log::warn!("voice group '{name}': unreadable password in the data file, group left open");
                                        None
                                    }
                                },
                                // Antes de 0.13 se guardaban en claro.
                                p => {
                                    upgraded = true;
                                    Some(PasswordHash::new(p))
                                }
                            },
                            owner: protocol::uuid_from_str(owner),
                            persistent: true,
                            saved_members: HashSet::new(),
                        },
                    );
                }
                ["member", group, id] => {
                    if let (Some(g), Some(id)) = (self.find_group(group), protocol::uuid_from_str(id)) {
                        self.groups.get_mut(&g).unwrap().saved_members.insert(id);
                    }
                }
                _ => {}
            }
        }
        upgraded
    }

    /// Prepara el archivo aquí (en memoria, rápido) y lo escribe el hilo de [`Saver`].
    pub(super) fn save(&self) {
        if let Some(path) = &self.data_file {
            self.saver.submit(path.clone(), self.serialize());
        }
    }
}
