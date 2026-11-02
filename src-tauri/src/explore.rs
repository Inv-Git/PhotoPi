//! Explorer un dossier ou un disque en lecture seule : toutes les photos et vidéos,
//! regroupées par dossier. Rien n'est lu au-delà du nom, de la taille et de la date du fichier,
//! et rien n'est modifié.

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::library::{kind_of, Item};
use crate::util::{self, norm};

/// Dossiers système ou techniques ignorés
const SKIP: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "appdata",
    "system volume information",
    "recovery",
    "perflogs",
    "msocache",
    "node_modules",
    "__pycache__",
    "_extraits",
];

#[derive(Serialize)]
pub struct Folder {
    pub path: String,
    pub items: Vec<Item>,
}

fn key(p: &Path) -> String {
    norm(&p.to_string_lossy()).trim_end_matches('\\').to_string()
}

struct Walk<'a> {
    exclude: Vec<String>,
    out: Vec<Folder>,
    seen: usize,
    found: usize,
    progress: &'a dyn Fn(usize, usize),
}

impl Walk<'_> {
    fn dir(&mut self, dir: &Path) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut items = Vec::new();
        let mut subs = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                // les liens et jonctions ne sont pas suivis (file_type ne les suit pas)
                let low = name.to_lowercase();
                if name.starts_with('$') || SKIP.contains(&low.as_str()) || self.exclude.contains(&key(&p)) {
                    continue;
                }
                subs.push(p);
            } else if ft.is_file() {
                self.seen += 1;
                if self.seen % 2000 == 0 {
                    (self.progress)(self.seen, self.found);
                }
                let Some(kind) = kind_of(&p) else { continue };
                let Ok(m) = e.metadata() else { continue };
                let mtime = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                items.push(Item {
                    path: p.to_string_lossy().into_owned(),
                    name,
                    kind: kind.to_string(),
                    raw: None,
                    motion: None,
                    date: if mtime > 0 { Some(util::date_from_unix(mtime)) } else { None },
                    size: m.len(),
                    thumb: None,
                    locked: false,
                });
                self.found += 1;
            }
        }
        if !items.is_empty() {
            items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            self.out.push(Folder { path: dir.to_string_lossy().into_owned(), items });
        }
        subs.sort_by_key(|p| p.to_string_lossy().to_lowercase());
        for s in subs {
            self.dir(&s);
        }
    }
}

/// Copie des fichiers dans le dossier d'arrivée (les originaux ne sont pas touchés),
/// chacun dans un sous-dossier au nom de son dossier d'origine (utile pour les événements).
/// Un fichier déjà copié (même nom, même taille) n'est pas recopié. Renvoie le nombre de copies.
pub fn copy_to(paths: &[String], inbox: &Path) -> Result<usize, String> {
    let mut n = 0;
    for p in paths {
        let src = Path::new(p);
        let Ok(meta) = fs::metadata(src) else { continue };
        if !meta.is_file() {
            continue;
        }
        let folder = src.parent().and_then(|d| d.file_name()).map(|f| f.to_string_lossy().into_owned()).unwrap_or_else(|| "Explorateur".into());
        let dir = inbox.join(folder);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let name = src.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        let (stem, ext) = util::split_ext(&name);
        let mut dst = dir.join(&name);
        let mut i = 0;
        let mut already = false;
        while let Ok(m) = fs::metadata(&dst) {
            if m.len() == meta.len() {
                already = true;
                break;
            }
            i += 1;
            dst = dir.join(format!("{stem}_{i}{ext}"));
        }
        if already {
            continue;
        }
        fs::copy(src, &dst).map_err(|e| format!("{} : {e}", src.display()))?;
        n += 1;
    }
    Ok(n)
}

/// Toutes les photos et vidéos sous `root`, par dossier, sans entrer dans les dossiers `exclude`.
pub fn scan(root: &Path, exclude: &[PathBuf], progress: &dyn Fn(usize, usize)) -> Result<Vec<Folder>, String> {
    if !root.is_dir() {
        return Err(format!("Dossier introuvable : {}", root.display()));
    }
    let mut w = Walk { exclude: exclude.iter().map(|p| key(p)).collect(), out: Vec::new(), seen: 0, found: 0, progress };
    w.dir(root);
    progress(w.seen, w.found);
    Ok(w.out)
}
