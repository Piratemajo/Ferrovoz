//! Quién oye a quién según el estado de cada jugador: vivo, espectador o muerto.
//!
//! El servidor elige una regla para espectadores y otra para muertos:
//!
//! | Regla    | Se les oye          | Oyen                  |
//! |----------|---------------------|-----------------------|
//! | `normal` | todos               | a todos               |
//! | `listen` | nadie               | a los vivos           |
//! | `ghost`  | solo los fantasmas  | vivos y fantasmas     |
//! | `own`    | solo los fantasmas  | solo a los fantasmas  |
//! | `off`    | nadie               | nadie                 |
//!
//! Fantasmas = espectadores y muertos con regla `ghost` u `own`, que tienen su
//! propio canal. Distancia y grupos se siguen aplicando.

/// Estado del jugador (lo envía Java cada tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Role {
    #[default]
    Alive = 0,
    Spectator = 1,
    Dead = 2,
}

impl Role {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Spectator,
            2 => Self::Dead,
            _ => Self::Alive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum RoleRule {
    #[default]
    Normal = 0,
    Listen = 1,
    Ghost = 2,
    Own = 3,
    Off = 4,
}

impl RoleRule {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Listen,
            2 => Self::Ghost,
            3 => Self::Own,
            4 => Self::Off,
            _ => Self::Normal,
        }
    }

    /// Canales en los que habla: bit 0 = vivos, bit 1 = fantasmas.
    fn speaks(self) -> u8 {
        match self {
            Self::Normal => LIVING,
            Self::Listen | Self::Off => 0,
            Self::Ghost | Self::Own => GHOSTS,
        }
    }

    /// Canales que oye.
    fn hears(self) -> u8 {
        match self {
            Self::Normal | Self::Listen => LIVING,
            Self::Ghost => LIVING | GHOSTS,
            Self::Own => GHOSTS,
            Self::Off => 0,
        }
    }
}

const LIVING: u8 = 1;
const GHOSTS: u8 = 2;

/// Por defecto los espectadores son fantasmas (no se les oye desde la partida) y los
/// muertos, normales (solo están unos segundos).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleRules {
    pub spectator: RoleRule,
    pub dead: RoleRule,
}

impl Default for RoleRules {
    fn default() -> Self {
        Self { spectator: RoleRule::Ghost, dead: RoleRule::Normal }
    }
}

impl RoleRules {
    fn rule(&self, role: Role) -> RoleRule {
        match role {
            Role::Alive => RoleRule::Normal,
            Role::Spectator => self.spectator,
            Role::Dead => self.dead,
        }
    }

    /// ¿Se le puede oír en este estado?
    pub fn can_be_heard(&self, speaker: Role) -> bool {
        self.rule(speaker).speaks() != 0
    }

    /// ¿Oye `listener` a `speaker`? Sin contar distancia ni grupos.
    pub fn can_hear(&self, speaker: Role, listener: Role) -> bool {
        self.rule(speaker).speaks() & self.rule(listener).hears() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Role::*;
    use RoleRule::*;

    fn rules(spectator: RoleRule, dead: RoleRule) -> RoleRules {
        RoleRules { spectator, dead }
    }

    #[test]
    fn by_default_spectators_are_not_heard_by_players() {
        let r = RoleRules::default();
        assert!(!r.can_hear(Spectator, Alive), "los que juegan no oyen a los espectadores");
        assert!(r.can_hear(Alive, Spectator), "los espectadores sí oyen la partida");
        assert!(r.can_hear(Spectator, Spectator), "y hablan entre ellos");
        assert!(r.can_hear(Dead, Alive) && r.can_hear(Alive, Dead), "los muertos, normal");
    }

    #[test]
    fn with_normal_rules_everyone_hears_everyone() {
        let r = rules(Normal, Normal);
        for s in [Alive, Spectator, Dead] {
            for l in [Alive, Spectator, Dead] {
                assert!(r.can_hear(s, l), "{s:?} -> {l:?}");
            }
        }
    }

    #[test]
    fn listen_only_spectators_hear_but_are_not_heard() {
        let r = rules(Listen, Normal);
        assert!(r.can_hear(Alive, Spectator));
        assert!(!r.can_hear(Spectator, Alive));
        assert!(!r.can_hear(Spectator, Spectator));
        assert!(!r.can_be_heard(Spectator));
    }

    #[test]
    fn ghosts_talk_among_themselves_and_hear_the_living() {
        let r = rules(Ghost, Ghost);
        assert!(r.can_hear(Spectator, Dead));
        assert!(r.can_hear(Dead, Spectator));
        assert!(r.can_hear(Alive, Dead));
        assert!(!r.can_hear(Dead, Alive));
        assert!(!r.can_hear(Spectator, Alive));
    }

    #[test]
    fn own_channel_is_fully_separate() {
        let r = rules(Own, Own);
        assert!(r.can_hear(Spectator, Spectator));
        assert!(!r.can_hear(Alive, Spectator));
        assert!(!r.can_hear(Spectator, Alive));
    }

    #[test]
    fn off_means_nothing_either_way() {
        let r = rules(Normal, Off);
        assert!(!r.can_hear(Alive, Dead));
        assert!(!r.can_hear(Dead, Alive));
        assert!(!r.can_be_heard(Dead));
        assert!(r.can_hear(Spectator, Alive));
    }
}
