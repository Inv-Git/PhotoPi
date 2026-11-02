//! Dater à la main : déplace des photos dans le dossier d'un mois (AAAA/AAAA-MM).
//! Les fichiers ne sont pas modifiés, seulement déplacés ; rien n'est jamais écrasé.

use std::fs;
use std::path::{Path, PathBuf};

use crate::util;

/// Déplace un groupe de fichiers (photo + RAW + petite vidéo) dans `dest`, en gardant leurs noms
/// ensemble : si l'un des noms est pris, le même suffixe « _n » est ajouté à tous.
/// Renvoie les paires (ancien chemin, nouveau chemin).
pub fn move_group(files: &[String], dest: &Path) -> Result<Vec<(String, String)>, String> {
    let files: Vec<&Path> = files.iter().map(Path::new).filter(|p| p.is_file()).collect();
    if files.is_empty() {
        return Ok(Vec::new());
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let target = |f: &Path, suffix: &str| -> PathBuf {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let (stem, ext) = util::split_ext(&name);
        dest.join(format!("{stem}{suffix}{ext}"))
    };
    let mut n = 0;
    let suffix = loop {
        let s = if n == 0 { String::new() } else { format!("_{n}") };
        // déjà au bon endroit, ou noms libres pour tout le groupe
        if files.iter().all(|f| {
            let t = target(f, &s);
            t == *f || !t.exists()
        }) {
            break s;
        }
        n += 1;
    };
    let mut moved = Vec::new();
    for f in files {
        let t = target(f, &suffix);
        if t == f {
            continue;
        }
        let ok = fs::rename(f, &t).is_ok() || (fs::copy(f, &t).is_ok() && fs::remove_file(f).is_ok());
        if !ok {
            return Err(format!("Impossible de déplacer {}", f.display()));
        }
        moved.push((f.to_string_lossy().into_owned(), t.to_string_lossy().into_owned()));
    }
    Ok(moved)
}

/// Dossier du mois « AAAA-MM » dans la photothèque, ou None si le mois n'est pas valide.
pub fn month_dir(library: &Path, month: &str) -> Option<PathBuf> {
    let (m, _) = util::parse_date(month, 2100)?;
    if m != month {
        return None;
    }
    Some(library.join(&month[..4]).join(month))
}
