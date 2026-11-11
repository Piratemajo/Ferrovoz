//! `ferrovoz-data.txt`: grupos permanentes y silencios, escritos en su propio hilo.

use super::groups::GROUPS_MAX;
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
    next: Option<(PathBuf, SaveData)>,
    closed: bool,
}

/// Lo que hace falta para escribir el archivo, copiado del estado. Ordenarlo y
/// formatearlo lo hace el hilo que guarda: con muchos grupos fijos, hacerlo donde se
/// llama costaba milisegundos con el candado del relé cogido, o sea con la voz parada.
#[derive(Default)]
pub(super) struct SaveData {
    mutes: Vec<PlayerId>,
    groups: Vec<SavedGroup>,
}

struct SavedGroup {
    name: String,
    owner: Option<PlayerId>,
    password: Option<PasswordHash>,
    members: Vec<PlayerId>,
}

impl SaveData {
    fn text(mut self) -> String {
        let mut s = String::from("# Ferrovoz: silencios y grupos permanentes. Edítalo solo con el servidor apagado.\n");
        let mut mutes: Vec<String> = self.mutes.iter().map(protocol::uuid_to_string).collect();
        mutes.sort();
        for m in mutes {
            s.push_str(&format!("mute\t{m}\n"));
        }
        self.groups.sort_by_key(|g| g.name.to_lowercase());
        for g in &self.groups {
            let owner = g.owner.as_ref().map(protocol::uuid_to_string).unwrap_or_else(|| "-".into());
            let password = g.password.as_ref().map(PasswordHash::encode).unwrap_or_default();
            s.push_str(&format!("group\t{}\t{}\t{}\n", g.name, owner, password));
            let mut members: Vec<String> = g.members.iter().map(protocol::uuid_to_string).collect();
            members.sort();
            for m in members {
                s.push_str(&format!("member\t{}\t{m}\n", g.name));
            }
        }
        s
    }
}

impl Saver {
    pub(super) fn submit(&self, path: PathBuf, data: SaveData) {
        lock(&self.queue).next = Some((path, data));
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
            let Some((path, data)) = job else { return };
            let text = data.text();
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
    /// Copia lo que va al archivo. Barato: ni ordena ni formatea, eso es del hilo que
    /// guarda (ver [`SaveData`]).
    fn snapshot(&self) -> SaveData {
        SaveData {
            mutes: self.admin_muted.iter().copied().collect(),
            groups: self
                .groups
                .values()
                .filter(|g| g.persistent)
                .map(|g| SavedGroup {
                    name: g.name.clone(),
                    owner: g.owner,
                    password: g.password.clone(),
                    members: g.saved_members.iter().copied().collect(),
                })
                .collect(),
        }
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
        if self.groups.len() > GROUPS_MAX {
            log::warn!(
                "voice relay: {} saved groups, more than the {GROUPS_MAX} allowed: no new ones can be created until some are deleted with /voice group delete",
                self.groups.len()
            );
        }
        upgraded
    }

    /// Copia lo que hay que guardar; escribirlo es cosa del hilo de [`Saver`].
    pub(super) fn save(&self) {
        if let Some(path) = &self.data_file {
            self.saver.submit(path.clone(), self.snapshot());
        }
    }
}
