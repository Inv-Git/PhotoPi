//! Lecture de la photothèque : années, mois, événements, photos.
//!
//! Structure attendue (celle produite par tri_photos.py) :
//!   <racine>/AAAA/AAAA-MM/...            un mois
//!   <racine>/AAAA/AAAA-MM Événement/...  un événement
//!   <racine>/À trier/...                 et tout autre dossier : albums « Autres »
//!   <racine>/_doublons_a_verifier/...    revue des doublons (pas dans la chronologie)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::util;

pub const PHOTO_EXT: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif"];
pub const RAW_EXT: &[&str] = &[
    "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "dng", "srw", "x3f", "3fr",
    "iiq", "rwl",
];
pub const VIDEO_EXT: &[&str] = &["mp4", "mov", "m4v", "avi", "mkv", "mts", "m2ts", "3gp", "wmv", "mpg", "mpeg", "webm"];
pub const DUP_DIR: &str = "_doublons_a_verifier";
/// Dossiers ignorés dans la chronologie (import en cours, dossiers techniques).
const SKIP_DIRS: &[&str] = &["importation en cours", "_extraits"];

#[derive(Serialize, Clone)]
pub struct Item {
    pub path: String,
    pub name: String,
    /// "photo", "raw" ou "video"
    pub kind: String,
    /// Fichier RAW jumeau d'une photo (même nom)
    pub raw: Option<String>,
    /// Petite vidéo d'une photo animée
    pub motion: Option<String>,
    /// « AAAA-MM-JJ HH:MM:SS »
    pub date: Option<String>,
    pub size: u64,
    /// Miniature déjà prête dans le cache (renseignée après la lecture)
    pub thumb: Option<String>,
    /// Photo secrète (affichée seulement après le mot de passe)
    pub locked: bool,
}

#[derive(Serialize, Clone)]
pub struct Album {
    pub id: String,
    pub title: String,
    /// « AAAA-MM » pour un mois ou un événement
    pub month: Option<String>,
    pub event: bool,
    pub items: Vec<Item>,
}

#[derive(Serialize, Clone)]
pub struct Year {
    pub year: String,
    pub albums: Vec<Album>,
}

#[derive(Serialize, Clone)]
pub struct Library {
    pub root: String,
    pub years: Vec<Year>,
    pub others: Vec<Album>,
    pub count: usize,
    pub duplicates: usize,
    /// Photos verrouillées (retirées de la chronologie)
    pub locked: usize,
}

/// Cache des dates (évite de relire l'EXIF de chaque fichier à chaque lancement).
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime: u64,
    pub date: Option<String>,
}
pub type DateCache = HashMap<String, CacheEntry>;

pub fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn kind_of(p: &Path) -> Option<&'static str> {
    let e = ext_of(p);
    let e = e.as_str();
    if PHOTO_EXT.contains(&e) {
        Some("photo")
    } else if RAW_EXT.contains(&e) {
        Some("raw")
    } else if VIDEO_EXT.contains(&e) {
        Some("video")
    } else {
        None
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_hidden(p: &Path) -> bool {
    file_name(p).starts_with('.')
}

/// Fichiers et sous-dossiers directs d'un dossier, triés par nom.
fn list(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if is_hidden(&p) {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => dirs.push(p),
                Ok(t) if t.is_file() => files.push(p),
                _ => {}
            }
        }
    }
    files.sort();
    dirs.sort();
    (files, dirs)
}

/// Tous les fichiers d'un dossier et de ses sous-dossiers.
pub fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let (files, dirs) = list(dir);
    out.extend(files);
    for d in dirs {
        walk_files(&d, out);
    }
}

/// Date de prise de vue lue dans l'EXIF (JPEG, HEIC, TIFF et RAW dérivés du TIFF).
pub fn exif_date(p: &Path) -> Option<String> {
    let f = fs::File::open(p).ok()?;
    let mut r = std::io::BufReader::new(f);
    let ex = exif::Reader::new().read_from_container(&mut r).ok()?;
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime] {
        if let Some(field) = ex.get_field(tag, exif::In::PRIMARY) {
            if let exif::Value::Ascii(ref v) = field.value {
                if let Some(s) = v.first() {
                    if let Some(d) = util::exif_date_to_iso(&String::from_utf8_lossy(s)) {
                        return Some(d);
                    }
                }
            }
        }
    }
    None
}

fn date_of(p: &Path, kind: &str, cache: &mut DateCache) -> (Option<String>, u64) {
    let meta = match fs::metadata(p) {
        Ok(m) => m,
        Err(_) => return (None, 0),
    };
    let size = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let key = p.to_string_lossy().into_owned();
    if let Some(e) = cache.get(&key) {
        if e.size == size && e.mtime == mtime {
            return (e.date.clone(), size);
        }
    }
    let exif = if kind == "video" { None } else { exif_date(p) };
    // sans EXIF : date du fichier (conservée par le rangement)
    let date = exif.or_else(|| if mtime > 0 { Some(util::date_from_unix(mtime)) } else { None });
    cache.insert(key, CacheEntry { size, mtime, date: date.clone() });
    (date, size)
}

fn pair_key(p: &Path, stem: &str) -> String {
    format!("{}|{}", p.parent().map(|d| d.to_string_lossy().to_lowercase()).unwrap_or_default(), stem)
}

fn stem_lower(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Construit les éléments d'un album : RAW et petites vidéos rattachés à leur photo.
fn build_items(files: &[PathBuf], cache: &mut DateCache) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut by_stem: HashMap<String, usize> = HashMap::new();
    let new_item = |p: &Path, kind: &str| Item {
        path: p.to_string_lossy().into_owned(),
        name: file_name(p),
        kind: kind.to_string(),
        raw: None,
        motion: None,
        date: None,
        size: 0,
        thumb: None,
        locked: false,
    };
    for p in files.iter().filter(|p| kind_of(p) == Some("photo")) {
        by_stem.insert(pair_key(p, &stem_lower(p)), items.len());
        items.push(new_item(p, "photo"));
    }
    for p in files.iter().filter(|p| kind_of(p) == Some("raw")) {
        match by_stem.get(&pair_key(p, &stem_lower(p))) {
            Some(&i) if items[i].raw.is_none() => items[i].raw = Some(p.to_string_lossy().into_owned()),
            _ => items.push(new_item(p, "raw")),
        }
    }
    for p in files.iter().filter(|p| kind_of(p) == Some("video")) {
        let photo = util::motion_key(&file_name(p)).and_then(|k| by_stem.get(&pair_key(p, &k)).copied());
        match photo {
            Some(i) if items[i].motion.is_none() => items[i].motion = Some(p.to_string_lossy().into_owned()),
            _ => items.push(new_item(p, "video")),
        }
    }
    for it in items.iter_mut() {
        let (date, size) = date_of(Path::new(&it.path), &it.kind, cache);
        it.date = date;
        it.size = size;
    }
    items.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.name.cmp(&b.name)));
    items
}

fn album(dir: &Path, title: String, month: Option<String>, event: bool, files: &[PathBuf], cache: &mut DateCache) -> Album {
    Album { id: dir.to_string_lossy().into_owned(), title, month, event, items: build_items(files, cache) }
}

pub fn scan(root: &Path, cache: &mut DateCache) -> Result<Library, String> {
    if !root.is_dir() {
        return Err(format!("Dossier introuvable : {}", root.display()));
    }
    let (root_files, root_dirs) = list(root);
    let mut years = Vec::new();
    let mut others = Vec::new();
    let mut duplicates = 0;

    for dir in &root_dirs {
        let name = file_name(dir);
        let low = name.to_lowercase();
        if low == DUP_DIR {
            let mut all = Vec::new();
            walk_files(dir, &mut all);
            duplicates = all.iter().filter(|p| kind_of(p).is_some()).count();
            continue;
        }
        if SKIP_DIRS.contains(&low.as_str()) {
            continue;
        }
        if util::is_year(&name) {
            let (files, subs) = list(dir);
            let mut albums = Vec::new();
            for sub in &subs {
                let sub_name = file_name(sub);
                if SKIP_DIRS.contains(&sub_name.to_lowercase().as_str()) {
                    continue;
                }
                let mut all = Vec::new();
                walk_files(sub, &mut all);
                let (month, title) = util::parse_album_name(&sub_name);
                let event = month.is_some() && title.is_some();
                let title = title.unwrap_or_else(|| sub_name.clone());
                let a = album(sub, title, month, event, &all, cache);
                if !a.items.is_empty() {
                    albums.push(a);
                }
            }
            if files.iter().any(|p| kind_of(p).is_some()) {
                let a = album(dir, format!("{name} (sans mois)"), None, false, &files, cache);
                albums.push(a);
            }
            // du plus récent au plus ancien ; à mois égal, le mois avant ses événements
            albums.sort_by(|a, b| b.month.cmp(&a.month).then(a.event.cmp(&b.event)).then(a.title.cmp(&b.title)));
            if !albums.is_empty() {
                years.push(Year { year: name, albums });
            }
        } else {
            // Autres dossiers (« À trier »…) : un album pour le dossier, un par sous-dossier
            let (files, subs) = list(dir);
            let a = album(dir, name.clone(), None, false, &files, cache);
            if !a.items.is_empty() {
                others.push(a);
            }
            for sub in &subs {
                let mut all = Vec::new();
                walk_files(sub, &mut all);
                let a = album(sub, format!("{} › {}", name, file_name(sub)), None, false, &all, cache);
                if !a.items.is_empty() {
                    others.push(a);
                }
            }
        }
    }
    if root_files.iter().any(|p| kind_of(p).is_some()) {
        others.insert(0, album(root, "Racine".into(), None, false, &root_files, cache));
    }
    years.sort_by(|a, b| b.year.cmp(&a.year));
    let count = years.iter().flat_map(|y| &y.albums).chain(others.iter()).map(|a| a.items.len()).sum();
    Ok(Library { root: root.to_string_lossy().into_owned(), years, others, count, duplicates, locked: 0 })
}
