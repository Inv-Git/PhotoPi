//! Revue des doublons : relie chaque fichier de « _doublons_a_verifier »
//! à l'exemplaire conservé, grâce aux rapports CSV du script tri_photos.py.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::library::{kind_of, walk_files, DUP_DIR};
use crate::util::{norm, parse_csv};

#[derive(Serialize)]
pub struct Pair {
    /// Fichier mis de côté
    pub dup: String,
    /// Exemplaire conservé (None s'il est introuvable)
    pub keep: Option<String>,
    /// "doublon" (identique), "quasi-doublon" (recompressé) ou "inconnu" (absent des rapports)
    pub kind: String,
    pub dup_size: u64,
    pub keep_size: u64,
}

fn size(p: &str) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn exists(p: &str) -> bool {
    !p.is_empty() && Path::new(p).is_file()
}

pub fn pairs(root: &Path, reports: &Path) -> Vec<Pair> {
    let mut files = Vec::new();
    walk_files(&root.join(DUP_DIR), &mut files);
    files.retain(|p| kind_of(p).is_some());
    if files.is_empty() {
        return Vec::new();
    }

    // Rapports du plus ancien au plus récent (le nom contient la date)
    let mut csvs: Vec<PathBuf> = fs::read_dir(reports)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                    n.starts_with("rapport_tri_") && n.ends_with(".csv")
                })
                .collect()
        })
        .unwrap_or_default();
    csvs.sort();

    // source -> destination, et destination d'un doublon -> (exemplaire conservé, statut)
    let mut dest_of: HashMap<String, String> = HashMap::new();
    let mut dup_of: HashMap<String, (String, String)> = HashMap::new();
    for csv in &csvs {
        let text = match fs::read(csv) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(_) => continue,
        };
        for row in parse_csv(&text).into_iter().skip(1) {
            if row.len() < 3 || row[1].is_empty() {
                continue;
            }
            dest_of.insert(norm(&row[0]), row[1].clone());
            let statut = row[2].as_str();
            if statut == "doublon" || statut == "quasi-doublon" {
                let of = row.get(4).cloned().unwrap_or_default();
                dup_of.insert(norm(&row[1]), (of, statut.to_string()));
            }
        }
    }

    files
        .iter()
        .map(|f| {
            let dup = f.to_string_lossy().into_owned();
            let (keep, kind) = match dup_of.get(&norm(&dup)) {
                Some((of, statut)) => {
                    // l'exemplaire conservé a pu être déplacé par le rangement : on suit sa destination
                    let keep = dest_of
                        .get(&norm(of))
                        .filter(|p| exists(p))
                        .cloned()
                        .or_else(|| if exists(of) { Some(of.clone()) } else { None });
                    (keep, statut.clone())
                }
                None => (None, "inconnu".to_string()),
            };
            let keep_size = keep.as_deref().map(size).unwrap_or(0);
            Pair { dup_size: size(&dup), dup, keep, kind, keep_size }
        })
        .collect()
}
