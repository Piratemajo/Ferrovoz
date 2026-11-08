//! Contraseñas de los grupos. Nunca se guardan tal cual, ni en memoria ni en disco:
//! solo PBKDF2-HMAC-SHA256 con una sal al azar por grupo.
//!
//! Son contraseñas de poco valor (entrar en un grupo de voz), pero la gente repite
//! contraseñas, y `ferrovoz-data.txt` acaba en copias de seguridad y en manos de
//! quien administra el servidor. Las iteraciones son pocas para PBKDF2 porque se
//! comprueban en el hilo del juego al usar un comando: unos milisegundos.

use sha2::{Digest, Sha256};

/// Iteraciones para contraseñas nuevas. Van guardadas con cada una, así que se pueden
/// subir sin romper las existentes.
const ITERATIONS: u32 = 10_000;
const SALT_LEN: usize = 16;
/// Prefijo en `ferrovoz-data.txt`. Lo que no lo lleva es una contraseña antigua en claro.
const PREFIX: &str = "pbkdf2-sha256$";

#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash {
    iterations: u32,
    salt: [u8; SALT_LEN],
    digest: [u8; 32],
}

impl std::fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PasswordHash(..)") // ni en los logs de depuración
    }
}

impl PasswordHash {
    pub fn new(password: &str) -> Self {
        let mut salt = [0u8; SALT_LEN];
        getrandom::getrandom(&mut salt).expect("no OS randomness source");
        Self { iterations: ITERATIONS, salt, digest: pbkdf2(password.as_bytes(), &salt, ITERATIONS) }
    }

    /// Comparación en tiempo constante.
    pub fn verify(&self, password: &str) -> bool {
        let got = pbkdf2(password.as_bytes(), &self.salt, self.iterations);
        got.iter().zip(&self.digest).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    /// `pbkdf2-sha256$<iteraciones>$<sal hex>$<hash hex>`.
    pub fn encode(&self) -> String {
        format!("{PREFIX}{}${}${}", self.iterations, hex(&self.salt), hex(&self.digest))
    }

    /// Lo que escribe [`Self::encode`]. None si no tiene ese formato.
    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.strip_prefix(PREFIX)?.split('$');
        let iterations: u32 = parts.next()?.parse().ok().filter(|&n| n > 0 && n <= 10_000_000)?;
        let salt = unhex(parts.next()?)?.try_into().ok()?;
        let digest = unhex(parts.next()?)?.try_into().ok()?;
        parts.next().is_none().then_some(Self { iterations, salt, digest })
    }

    /// ¿Es una contraseña ya protegida (o una antigua en claro, que hay que convertir)?
    pub fn is_encoded(s: &str) -> bool {
        s.starts_with(PREFIX)
    }
}

fn hmac(key: &[u8], message: &[&[u8]]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    for part in message {
        inner.update(part);
    }
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// PBKDF2-HMAC-SHA256 (RFC 8018), un solo bloque de 32 bytes.
fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut u = hmac(password, &[salt, &1u32.to_be_bytes()]);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac(password, &[&u]);
        for (o, b) in out.iter_mut().zip(u) {
            *o ^= b;
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_the_rfc_7914_test_vector() {
        // RFC 7914, sección 11: P = "passwd", S = "salt", c = 1 (primeros 32 bytes).
        let expected = unhex("55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc").unwrap();
        assert_eq!(pbkdf2(b"passwd", b"salt", 1).to_vec(), expected);
    }

    #[test]
    fn verifies_and_rejects() {
        let h = PasswordHash::new("secreta");
        assert!(h.verify("secreta"));
        assert!(!h.verify("Secreta") && !h.verify("") && !h.verify("secreta "));
        assert_ne!(PasswordHash::new("secreta"), h, "cada una lleva su propia sal");
    }

    #[test]
    fn round_trips_without_leaking_the_password() {
        let h = PasswordHash::new("pa|ss\tword");
        let text = h.encode();
        assert!(!text.contains("pa|ss") && PasswordHash::is_encoded(&text));
        assert!(!text.contains('\t') && !text.contains('\n'), "cabe en una línea del archivo de datos");
        assert_eq!(PasswordHash::decode(&text), Some(h.clone()));
        assert!(PasswordHash::decode(&text).unwrap().verify("pa|ss\tword"));
        assert_eq!(format!("{h:?}"), "PasswordHash(..)");
        for bad in ["", "1234", "pbkdf2-sha256$0$00$00", "pbkdf2-sha256$10$zz$00", &format!("{text}$extra")] {
            assert!(PasswordHash::decode(bad).is_none(), "{bad}");
        }
    }
}
