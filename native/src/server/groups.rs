//! Grupos de voz: crear, unirse, salir, expulsar, permanentes y lo que se avisa.

use super::*;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum GroupError {
    InvalidName,
    AlreadyExists,
    NotFound,
    WrongPassword,
    UnknownPlayer,
    NotOwner,
    NotInGroup,
}

impl GroupError {
    /// Código numérico para Java.
    pub fn code(self) -> i32 {
        match self {
            Self::InvalidName => 1,
            Self::AlreadyExists => 2,
            Self::NotFound => 3,
            Self::WrongPassword => 4,
            Self::UnknownPlayer => 5,
            Self::NotOwner => 6,
            Self::NotInGroup => 7,
        }
    }
}

impl VoiceServer {
    /// Crea un grupo con `creator` como dueño.
    pub fn create_group(&self, creator: &PlayerId, name: &str, password: Option<&str>) -> Result<(), GroupError> {
        let name = name.trim();
        // '|' es el separador en los mensajes hacia Java; '§' cambiaría colores y formato
        // en los mensajes del chat que llevan el nombre.
        if !valid_text(name, MAX_GROUP_NAME)
            || name.contains(['|', '§'])
            || password.is_some_and(|p| !p.is_empty() && !valid_text(p, MAX_PASSWORD))
        {
            return Err(GroupError::InvalidName);
        }
        // Cuesta unos ms: fuera del lock.
        let password = password.filter(|p| !p.is_empty()).map(PasswordHash::new);
        self.with_state(|st, out| {
            if !st.players.contains_key(creator) {
                return Err(GroupError::UnknownPlayer);
            }
            if st.find_group(name).is_some() {
                return Err(GroupError::AlreadyExists);
            }
            let id = st.next_group;
            st.next_group += 1;
            st.groups.insert(
                id,
                Group {
                    name: name.to_owned(),
                    password,
                    owner: Some(*creator),
                    persistent: false,
                    saved_members: HashSet::new(),
                },
            );
            st.set_group(creator, Some(id), out);
            Ok(())
        })
    }

    pub fn join_group(&self, player: &PlayerId, name: &str, password: Option<&str>) -> Result<(), GroupError> {
        // La contraseña se comprueba fuera del lock (cuesta unos ms) y después se mira
        // que el grupo siga teniendo la misma.
        let stored = {
            let st = self.state();
            if !st.players.contains_key(player) {
                return Err(GroupError::UnknownPlayer);
            }
            let id = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            st.groups[&id].password.clone()
        };
        if let Some(hash) = &stored {
            if !password.is_some_and(|p| hash.verify(p)) {
                return Err(GroupError::WrongPassword);
            }
        }
        self.with_state(|st, out| {
            if !st.players.contains_key(player) {
                return Err(GroupError::UnknownPlayer);
            }
            let id = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if st.groups[&id].password != stored {
                return Err(GroupError::WrongPassword); // lo han cambiado mientras
            }
            if st.players[player].group == Some(id) {
                return Ok(());
            }
            st.set_group(player, Some(id), out);
            st.push_event(EventKind::Joined, st.groups[&id].name.clone(), *player);
            Ok(())
        })
    }

    pub fn leave_group(&self, player: &PlayerId) -> bool {
        self.with_state(|st, out| {
            let Some(g) = st.players.get(player).and_then(|p| p.group) else { return false };
            let name = st.groups[&g].name.clone();
            st.set_group(player, None, out);
            if let Some(group) = st.groups.get_mut(&g) {
                if group.saved_members.remove(player) {
                    st.save();
                }
            }
            st.push_event(EventKind::Left, name, *player);
            st.after_member_left(g, player, out);
            true
        })
    }

    /// Expulsa a `target` de su grupo. Solo el dueño o un admin (`admin = true`).
    pub fn kick_from_group(&self, requester: &PlayerId, target: &PlayerId, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.players.get(target).and_then(|p| p.group).ok_or(GroupError::NotInGroup)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            let name = st.groups[&g].name.clone();
            st.set_group(target, None, out);
            st.groups.get_mut(&g).unwrap().saved_members.remove(target);
            st.save();
            st.push_event(EventKind::Kicked, name.clone(), *target);
            st.after_member_left(g, target, out);
            Ok(name)
        })
    }

    /// Hace permanente (o temporal) el grupo de `requester`. Solo el dueño o un admin.
    pub fn set_group_persistent(&self, requester: &PlayerId, persistent: bool, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.players.get(requester).and_then(|p| p.group).ok_or(GroupError::NotInGroup)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            Ok(st.apply_persistent(g, persistent, out))
        })
    }

    /// Borra un grupo, también si es permanente. Solo el dueño o un admin.
    pub fn delete_group(&self, requester: &PlayerId, name: &str, admin: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if !admin && st.groups[&g].owner != Some(*requester) {
                return Err(GroupError::NotOwner);
            }
            let real_name = st.groups[&g].name.clone();
            for m in st.members_of(g) {
                st.set_group(&m, None, out);
                if m != *requester {
                    st.push_event(EventKind::Dissolved, real_name.clone(), m);
                }
            }
            st.groups.remove(&g);
            st.save();
            st.push_event(EventKind::Deleted, real_name.clone(), *requester);
            Ok(real_name)
        })
    }

    /// Mete a un jugador en un grupo sin pedir contraseña (plugins).
    pub fn force_join_group(&self, player: &PlayerId, name: &str) -> Result<(), GroupError> {
        self.with_state(|st, out| {
            if !st.players.contains_key(player) {
                return Err(GroupError::UnknownPlayer);
            }
            let id = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            if st.players[player].group == Some(id) {
                return Ok(());
            }
            st.set_group(player, Some(id), out);
            st.push_event(EventKind::Joined, st.groups[&id].name.clone(), *player);
            Ok(())
        })
    }

    /// Hace permanente (o temporal) un grupo por su nombre, sin mirar quién lo pide
    /// (plugins). Un grupo temporal sin nadie dentro se borra.
    pub fn set_group_persistent_by_name(&self, name: &str, persistent: bool) -> Result<String, GroupError> {
        self.with_state(|st, out| {
            let g = st.find_group(name.trim()).ok_or(GroupError::NotFound)?;
            let real = st.apply_persistent(g, persistent, out);
            st.cleanup_group(g);
            Ok(real)
        })
    }

    /// (dueño, contraseña sí/no, permanente) de un grupo.
    pub fn group_info(&self, name: &str) -> Option<(Option<PlayerId>, bool, bool)> {
        let st = self.state();
        let g = st.find_group(name.trim())?;
        let group = &st.groups[&g];
        Some((group.owner, group.password.is_some(), group.persistent))
    }

    pub fn group_of(&self, player: &PlayerId) -> Option<String> {
        let st = self.state();
        let g = st.players.get(player)?.group?;
        st.groups.get(&g).map(|g| g.name.clone())
    }

    pub fn group_owner(&self, name: &str) -> Option<PlayerId> {
        let st = self.state();
        st.find_group(name).and_then(|g| st.groups[&g].owner)
    }

    /// (nombre, miembros conectados, tiene contraseña, permanente), ordenado por nombre.
    pub fn list_groups(&self) -> Vec<(String, usize, bool, bool)> {
        let st = self.state();
        let mut out: Vec<_> = st
            .groups
            .iter()
            .map(|(id, g)| {
                let members = st.players.values().filter(|p| p.group == Some(*id)).count();
                (g.name.clone(), members, g.password.is_some(), g.persistent)
            })
            .collect();
        out.sort_by_key(|g| g.0.to_lowercase());
        out
    }

    pub fn group_members(&self, name: &str) -> Vec<PlayerId> {
        let st = self.state();
        st.find_group(name).map(|g| st.members_of(g)).unwrap_or_default()
    }
}

impl State {
    pub(super) fn find_group(&self, name: &str) -> Option<u32> {
        self.groups.iter().find(|(_, g)| g.name.eq_ignore_ascii_case(name)).map(|(id, _)| *id)
    }

    pub(super) fn members_of(&self, g: u32) -> Vec<PlayerId> {
        self.players.iter().filter(|(_, p)| p.group == Some(g)).map(|(id, _)| *id).collect()
    }

    /// Hace permanente o temporal un grupo, lo guarda y avisa a sus miembros.
    pub(super) fn apply_persistent(&mut self, g: u32, persistent: bool, out: &mut Outgoing) -> String {
        let members: Vec<PlayerId> = self.members_of(g);
        let group = self.groups.get_mut(&g).unwrap();
        group.persistent = persistent;
        group.saved_members = if persistent { members.iter().copied().collect() } else { HashSet::new() };
        let name = group.name.clone();
        self.save();
        for m in members {
            self.send_group_state(&m, out);
        }
        name
    }

    /// Cambia el grupo de un jugador y se lo notifica.
    pub(super) fn set_group(&mut self, player: &PlayerId, group: Option<u32>, out: &mut Outgoing) {
        let old = match self.players.get_mut(player) {
            Some(p) => std::mem::replace(&mut p.group, group),
            None => return,
        };
        if let Some(g) = group {
            let grp = self.groups.get_mut(&g).unwrap();
            if grp.persistent && grp.saved_members.insert(*player) {
                self.save();
            }
        }
        self.send_group_state(player, out);
        if let Some(old) = old {
            if Some(old) != group {
                self.cleanup_group(old);
            }
        }
    }

    pub(super) fn send_group_state(&mut self, player: &PlayerId, out: &mut Outgoing) {
        let state = match self.players.get(player).and_then(|p| p.group).and_then(|g| self.groups.get(&g)) {
            Some(g) => GroupState { name: g.name.clone(), owner: g.owner == Some(*player), persistent: g.persistent },
            None => GroupState::default(),
        };
        let body = state.encode();
        if let Some(p) = self.players.get_mut(player) {
            let pkt = p.seal(PacketType::GroupState, &body);
            // Contador local; las estadísticas aquí dan igual.
            let stats = ServerStats::default();
            deliver(&stats, p, pkt, out);
        }
    }

    /// Al salir alguien: si era dueño de un grupo temporal, pasa la propiedad; si el
    /// grupo se queda vacío, se borra.
    pub(super) fn after_member_left(&mut self, g: u32, who: &PlayerId, out: &mut Outgoing) {
        let members = self.members_of(g);
        let Some(group) = self.groups.get_mut(&g) else { return };
        if !group.persistent && group.owner == Some(*who) {
            group.owner = members.first().copied();
            if let Some(new_owner) = group.owner {
                let name = group.name.clone();
                self.push_event(EventKind::NewOwner, name, new_owner);
                self.send_group_state(&new_owner, out);
            }
        }
        self.cleanup_group(g);
    }

    /// Borra los grupos temporales vacíos.
    pub(super) fn cleanup_group(&mut self, id: u32) {
        let persistent = self.groups.get(&id).is_some_and(|g| g.persistent);
        if !persistent && !self.players.values().any(|p| p.group == Some(id)) {
            self.groups.remove(&id);
        }
    }
}
