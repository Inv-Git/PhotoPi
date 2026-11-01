//! Verrou des photos : uniquement dans PhotoPi, les fichiers ne sont jamais modifiés.
//! Le fichier verrou.json ne contient ni noms de fichiers ni mot de passe :
//! seulement des empreintes SHA-256 (chemins) et un dérivé salé du mot de passe.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::util::{hex, norm, sha256};

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct LockStore {
    pub salt: String,
    pub hash: String,
    /// Empreintes des chemins verrouillés
    pub items: HashSet<String>,
}

/// Empreinte d'un chemin (insensible à la casse et au sens des barres).
pub fn path_id(p: &str) -> String {
    hex(&sha256(norm(p).as_bytes()))
}

/// Mot de passe -> dérivé lent (100 000 tours), pour décourager les essais en série.
fn derive(salt: &str, pw: &str) -> String {
    let mut h = sha256(format!("{salt}:{pw}").as_bytes());
    let mut buf = Vec::with_capacity(salt.len() + 32);
    for _ in 0..100_000 {
        buf.clear();
        buf.extend_from_slice(salt.as_bytes());
        buf.extend_from_slice(&h);
        h = sha256(&buf);
    }
    hex(&h)
}

fn new_salt() -> String {
    let t = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    hex(&sha256(format!("{t}:{}:{:p}", std::process::id(), &t).as_bytes()))
}

impl LockStore {
    pub fn has_password(&self) -> bool {
        !self.hash.is_empty()
    }

    pub fn check(&self, pw: &str) -> bool {
        self.has_password() && derive(&self.salt, pw) == self.hash
    }

    pub fn set_password(&mut self, pw: &str) {
        self.salt = new_salt();
        self.hash = derive(&self.salt, pw);
    }

    pub fn is_locked(&self, path: &str) -> bool {
        !self.items.is_empty() && self.items.contains(&path_id(path))
    }
}
