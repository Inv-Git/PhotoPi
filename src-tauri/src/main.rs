#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dating;
mod dupes;
mod explore;
mod importer;
mod library;
mod lock;
mod media;
mod util;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(default)]
struct Settings {
    /// Dossier de la photothèque
    library: String,
    /// Dossier où se trouvent les rapports rapport_tri_*.csv
    reports: String,
    /// Taille des miniatures dans la grille (px)
    tile: u32,
    /// Dossier d'arrivée des photos à importer (vide : <photothèque>\Importation en cours)
    import_dir: String,
}

struct AppState {
    settings: Mutex<Settings>,
    data_dir: PathBuf,
    cache_dir: PathBuf,
    dates: Mutex<library::DateCache>,
    /// Numéro de la préparation des miniatures en cours (une nouvelle annule la précédente)
    pregen: Arc<AtomicU64>,
    /// Empreintes visuelles déjà calculées (miniature -> empreinte)
    hashes: Mutex<HashMap<String, u64>>,
    /// Paires que Clem a déclarées différentes (à ne plus proposer)
    ignored: Mutex<HashSet<String>>,
    /// Verrou : empreintes des photos verrouillées et mot de passe
    lock: Mutex<lock::LockStore>,
    /// Photos verrouillées de la dernière lecture (données seulement avec le mot de passe)
    locked_items: Mutex<Vec<library::Item>>,
    /// Photos secrètes affichées (mot de passe donné depuis le démarrage)
    revealed: AtomicBool,
    /// Dernière analyse d'import, en attente de validation
    import_plan: Mutex<Option<importer::Plan>>,
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn save_json<T: serde::Serialize>(p: &Path, v: &T) {
    if let Ok(s) = serde_json::to_string(v) {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(state: State<AppState>, settings: Settings) {
    save_json(&state.data_dir.join("settings.json"), &settings);
    *state.settings.lock().unwrap() = settings;
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
async fn scan_library(app: AppHandle) -> Result<library::Library, String> {
    let root = app.state::<AppState>().settings.lock().unwrap().library.clone();
    if root.is_empty() {
        return Err("Aucun dossier de photos choisi".into());
    }
    blocking(move || {
        let state = app.state::<AppState>();
        let mut cache = std::mem::take(&mut *state.dates.lock().unwrap());
        let lib = library::scan(Path::new(&root), &mut cache);
        save_json(&state.data_dir.join("index.json"), &cache);
        *state.dates.lock().unwrap() = cache;
        let mut lib = lib?;
        // miniatures déjà prêtes : l'interface les affiche sans rien demander
        let dir = state.cache_dir.join("miniatures");
        let albums = lib.years.iter_mut().flat_map(|y| y.albums.iter_mut()).chain(lib.others.iter_mut());
        for a in albums {
            for it in a.items.iter_mut().filter(|it| it.kind != "video") {
                let t = media::thumb_file(Path::new(&it.path), &dir);
                if t.exists() {
                    it.thumb = Some(t.to_string_lossy().into_owned());
                }
            }
        }
        // photos secrètes : retirées de la chronologie, sauf si elles sont affichées
        let store = state.lock.lock().unwrap().clone();
        let mut hidden: Vec<library::Item> = Vec::new();
        if !store.items.is_empty() && state.revealed.load(Ordering::SeqCst) {
            let albums = lib.years.iter_mut().flat_map(|y| y.albums.iter_mut()).chain(lib.others.iter_mut());
            let mut n = 0;
            for a in albums {
                for it in a.items.iter_mut().filter(|it| store.is_locked(&it.path)) {
                    it.locked = true;
                    it.thumb = None; // jamais de miniature sur le disque pour une photo secrète
                    n += 1;
                }
            }
            lib.locked = n;
            return Ok(lib);
        }
        if !store.items.is_empty() {
            for y in lib.years.iter_mut() {
                for a in y.albums.iter_mut() {
                    let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut a.items).into_iter().partition(|it| !store.is_locked(&it.path));
                    a.items = keep;
                    hidden.extend(gone);
                }
                y.albums.retain(|a| !a.items.is_empty());
            }
            lib.years.retain(|y| !y.albums.is_empty());
            for a in lib.others.iter_mut() {
                let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut a.items).into_iter().partition(|it| !store.is_locked(&it.path));
                a.items = keep;
                hidden.extend(gone);
            }
            lib.others.retain(|a| !a.items.is_empty());
        }
        for it in hidden.iter_mut() {
            it.thumb = None;
        }
        lib.locked = hidden.len();
        lib.count -= hidden.len();
        *state.locked_items.lock().unwrap() = hidden;
        Ok(lib)
    })
    .await?
}

/// Miniature d'une photo (chemin du fichier en cache), fabriquée tout de suite si besoin.
#[tauri::command]
async fn thumb(app: AppHandle, path: String) -> Result<String, String> {
    let dir = app.state::<AppState>().cache_dir.join("miniatures");
    blocking(move || media::resized(Path::new(&path), &dir, media::THUMB, 82).map(|p| p.to_string_lossy().into_owned())).await?
}

#[derive(Clone, serde::Serialize)]
struct ThumbProgress {
    done: usize,
    total: usize,
    /// (photo, miniature) prêtes depuis le dernier envoi
    items: Vec<(String, String)>,
}

/// Prépare en tâche de fond les miniatures manquantes, dans l'ordre donné.
/// Envoie l'événement « thumbs » au fur et à mesure.
#[tauri::command]
fn pregen_thumbs(app: AppHandle, paths: Vec<String>) {
    let state = app.state::<AppState>();
    let gen = state.pregen.fetch_add(1, Ordering::SeqCst) + 1;
    let flag = state.pregen.clone();
    let dir = state.cache_dir.join("miniatures");
    let total = paths.len();
    if total == 0 {
        let _ = app.emit("thumbs", ThumbProgress { done: 0, total: 0, items: Vec::new() });
        return;
    }
    let paths = Arc::new(paths);
    let next = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicUsize::new(0));
    let batch: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    // on laisse au moins un cœur libre pour que l'interface reste fluide
    let workers = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(1).clamp(1, 4);
    for _ in 0..workers {
        let (app, paths, next, done, batch, flag, dir) =
            (app.clone(), paths.clone(), next.clone(), done.clone(), batch.clone(), flag.clone(), dir.clone());
        std::thread::spawn(move || loop {
            if flag.load(Ordering::SeqCst) != gen {
                return;
            }
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= total {
                return;
            }
            let p = &paths[i];
            let res = media::resized(Path::new(p), &dir, media::THUMB, 82);
            let mut b = batch.lock().unwrap();
            if let Ok(t) = res {
                b.push((p.clone(), t.to_string_lossy().into_owned()));
            }
            let d = done.fetch_add(1, Ordering::SeqCst) + 1;
            if b.len() >= 24 || d == total || d % 50 == 0 {
                let items = std::mem::take(&mut *b);
                let _ = app.emit("thumbs", ThumbProgress { done: d, total, items });
            }
        });
    }
}

/// Image à afficher en grand : le fichier lui-même s'il est lisible, sinon un aperçu (RAW, TIFF).
#[tauri::command]
async fn display(app: AppHandle, path: String) -> Result<String, String> {
    if media::displayable(Path::new(&path)) {
        return Ok(path);
    }
    let dir = app.state::<AppState>().cache_dir.join("apercus");
    blocking(move || media::resized(Path::new(&path), &dir, 2560, 90).map(|p| p.to_string_lossy().into_owned())).await?
}

#[tauri::command]
async fn photo_info(path: String) -> Result<Vec<(String, String)>, String> {
    blocking(move || media::info(Path::new(&path))).await
}

#[tauri::command]
async fn duplicates(app: AppHandle) -> Result<Vec<dupes::Pair>, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    if s.library.is_empty() {
        return Ok(Vec::new());
    }
    blocking(move || dupes::pairs(Path::new(&s.library), Path::new(&s.reports))).await
}

/// Envoie des fichiers à la corbeille de Windows (récupérables).
#[tauri::command]
async fn trash_files(paths: Vec<String>) -> Result<(), String> {
    blocking(move || {
        for p in &paths {
            if Path::new(p).exists() {
                trash::delete(p).map_err(|e| format!("{p} : {e}"))?;
            }
        }
        Ok(())
    })
    .await?
}

/// Ouvre l'explorateur Windows sur le fichier.
#[tauri::command]
fn show_in_folder(path: String) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{path}\"")).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

#[derive(serde::Serialize)]
struct SimPair {
    a: String,
    b: String,
    /// Nombre de bits différents entre les empreintes (0 = quasi identiques)
    dist: u32,
}

/// Paires de photos qui se ressemblent. `items` : (photo, miniature).
#[tauri::command]
async fn similar_pairs(app: AppHandle, items: Vec<(String, String)>, max_dist: u32) -> Result<Vec<SimPair>, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let mut known = state.hashes.lock().unwrap().clone();
        // empreintes manquantes, calculées sur 4 fils
        let missing: Vec<&String> = items.iter().map(|(_, t)| t).filter(|t| !known.contains_key(*t)).collect();
        let fresh: Mutex<Vec<(String, u64)>> = Mutex::new(Vec::new());
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= missing.len() {
                        break;
                    }
                    if let Some(h) = media::dhash_file(Path::new(missing[i])) {
                        fresh.lock().unwrap().push((missing[i].clone(), h));
                    }
                });
            }
        });
        let fresh = fresh.into_inner().unwrap();
        if !fresh.is_empty() {
            known.extend(fresh);
            save_json(&state.data_dir.join("hashes.json"), &known);
            *state.hashes.lock().unwrap() = known.clone();
        }
        let ignored = state.ignored.lock().unwrap().clone();
        let hs: Vec<(&String, u64)> = items
            .iter()
            .filter_map(|(p, t)| known.get(t).map(|h| (p, *h)))
            .filter(|(_, h)| util::informative(*h))
            .collect();
        let mut out = Vec::new();
        for i in 0..hs.len() {
            for j in i + 1..hs.len() {
                let d = util::hamming(hs[i].1, hs[j].1);
                if d <= max_dist && !ignored.contains(&util::pair_id(hs[i].0, hs[j].0)) {
                    out.push(SimPair { a: hs[i].0.clone(), b: hs[j].0.clone(), dist: d });
                }
            }
        }
        out.sort_by_key(|p| p.dist);
        out.truncate(5000);
        out
    })
    .await
}

/// Retient qu'une paire n'est pas un doublon.
#[tauri::command]
fn similar_ignore(state: State<AppState>, a: String, b: String) {
    let mut ig = state.ignored.lock().unwrap();
    ig.insert(util::pair_id(&a, &b));
    let list: Vec<&String> = ig.iter().collect();
    save_json(&state.data_dir.join("similaires_ignores.json"), &list);
}

// ---------------------------------------------------------------------------
// Verrou des photos (dans PhotoPi seulement)
// ---------------------------------------------------------------------------

fn save_lock(state: &AppState) {
    save_json(&state.data_dir.join("verrou.json"), &*state.lock.lock().unwrap());
}

#[derive(serde::Serialize)]
struct LockStatus {
    has_password: bool,
    count: usize,
    revealed: bool,
}

#[tauri::command]
fn lock_status(state: State<AppState>) -> LockStatus {
    let l = state.lock.lock().unwrap();
    LockStatus { has_password: l.has_password(), count: l.items.len(), revealed: state.revealed.load(Ordering::SeqCst) }
}

/// Affiche les photos secrètes dans la chronologie (jusqu'à la fermeture ou « Masquer »).
#[tauri::command]
async fn lock_reveal(app: AppHandle, password: String) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let ok = state.lock.lock().unwrap().clone().check(&password);
        if !ok {
            return Err("Mot de passe incorrect.".to_string());
        }
        state.revealed.store(true, Ordering::SeqCst);
        Ok(())
    })
    .await?
}

#[tauri::command]
fn lock_hide(state: State<AppState>) {
    state.revealed.store(false, Ordering::SeqCst);
}

/// Crée ou change le mot de passe (l'ancien est demandé s'il existe).
#[tauri::command]
async fn lock_set_password(app: AppHandle, old: String, new: String) -> Result<(), String> {
    if new.chars().count() < 4 {
        return Err("Le mot de passe doit faire au moins 4 caractères.".into());
    }
    blocking(move || {
        let state = app.state::<AppState>();
        let mut l = state.lock.lock().unwrap().clone();
        if l.has_password() && !l.check(&old) {
            return Err("Ancien mot de passe incorrect.".to_string());
        }
        l.set_password(&new);
        *state.lock.lock().unwrap() = l;
        save_lock(&state);
        Ok(())
    })
    .await?
}

/// Verrouille des fichiers (photo, RAW, vidéo associée) et efface leurs miniatures du cache.
#[tauri::command]
async fn lock_items(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        if !state.lock.lock().unwrap().has_password() {
            return Err("Choisis d'abord un mot de passe.".to_string());
        }
        let thumbs = state.cache_dir.join("miniatures");
        let previews = state.cache_dir.join("apercus");
        {
            let mut l = state.lock.lock().unwrap();
            for p in &paths {
                l.items.insert(lock::path_id(p));
                let _ = std::fs::remove_file(media::thumb_file(Path::new(p), &thumbs));
                let _ = std::fs::remove_file(media::preview_file(Path::new(p), &previews));
            }
        }
        save_lock(&state);
        Ok(())
    })
    .await?
}

/// Ouvre l'espace verrouillé : renvoie les photos verrouillées si le mot de passe est bon.
#[tauri::command]
async fn lock_open(app: AppHandle, password: String) -> Result<Vec<library::Item>, String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let ok = state.lock.lock().unwrap().clone().check(&password);
        if !ok {
            return Err("Mot de passe incorrect.".to_string());
        }
        let items = state.locked_items.lock().unwrap().clone();
        Ok(items)
    })
    .await?
}

/// Déverrouille des fichiers (ils reviennent dans la chronologie à la prochaine lecture).
#[tauri::command]
async fn unlock_items(app: AppHandle, password: String, paths: Vec<String>) -> Result<(), String> {
    blocking(move || {
        let state = app.state::<AppState>();
        let ok = state.lock.lock().unwrap().clone().check(&password);
        if !ok {
            return Err("Mot de passe incorrect.".to_string());
        }
        {
            let mut l = state.lock.lock().unwrap();
            for p in &paths {
                l.items.remove(&lock::path_id(p));
            }
        }
        save_lock(&state);
        Ok(())
    })
    .await?
}

/// Miniature ou grande image d'une photo verrouillée, fabriquée en mémoire (rien dans le cache).
#[tauri::command]
async fn private_image(app: AppHandle, path: String, max: u32) -> Result<String, String> {
    {
        let state = app.state::<AppState>();
        if !state.lock.lock().unwrap().is_locked(&path) {
            return Err("Photo non verrouillée.".into());
        }
    }
    let max = max.clamp(200, 4096);
    blocking(move || media::data_url(Path::new(&path), max)).await?
}

// ---------------------------------------------------------------------------
// Albums thématiques : des listes de photos, les fichiers ne bougent pas
// ---------------------------------------------------------------------------

#[tauri::command]
fn albums_get(state: State<AppState>) -> serde_json::Value {
    read_json(&state.data_dir.join("albums.json")).unwrap_or_else(|| serde_json::json!([]))
}

#[tauri::command]
fn albums_set(state: State<AppState>, albums: serde_json::Value) {
    save_json(&state.data_dir.join("albums.json"), &albums);
}

// ---------------------------------------------------------------------------
// Import : ranger un dossier d'arrivée en vrac dans la photothèque
// ---------------------------------------------------------------------------

fn norm_path(p: &Path) -> String {
    let mut k = util::norm(&p.to_string_lossy());
    if !k.ends_with('\\') {
        k.push('\\');
    }
    k
}

fn import_dir_of(s: &Settings) -> PathBuf {
    if !s.import_dir.is_empty() {
        PathBuf::from(&s.import_dir)
    } else if !s.library.is_empty() {
        Path::new(&s.library).join("Importation en cours")
    } else {
        PathBuf::new()
    }
}

#[derive(serde::Serialize)]
struct ImportInfo {
    dir: String,
    pending: usize,
    exiftool: bool,
}

#[derive(Clone, serde::Serialize)]
struct ImportProgress {
    step: String,
    done: usize,
    total: usize,
}

#[tauri::command]
async fn import_info(app: AppHandle) -> Result<ImportInfo, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let dir = import_dir_of(&s);
    blocking(move || ImportInfo {
        pending: if dir.as_os_str().is_empty() { 0 } else { importer::pending(&dir) },
        exiftool: importer::find_exiftool().is_some(),
        dir: dir.to_string_lossy().into_owned(),
    })
    .await
}

/// Analyse le dossier d'arrivée (sans rien déplacer) et garde le plan en attente de validation.
#[tauri::command]
async fn import_analyze(app: AppHandle, source: Option<String>) -> Result<importer::Summary, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    if s.library.is_empty() {
        return Err("Choisis d'abord le dossier de ta photothèque.".into());
    }
    // sans source : le dossier d'arrivée (déplacement) ; avec : un autre dossier (copie des nouveautés)
    let copy = source.as_deref().map_or(false, |p| !p.is_empty());
    let src = if copy { PathBuf::from(source.unwrap_or_default()) } else { import_dir_of(&s) };
    if !copy {
        let _ = std::fs::create_dir_all(&src);
    }
    if copy && norm_path(&src).starts_with(&norm_path(Path::new(&s.library))) {
        return Err("Ce dossier fait déjà partie de ta photothèque.".into());
    }
    blocking(move || {
        let emit = |step: &str, done: usize, total: usize| {
            let _ = app.emit("import", ImportProgress { step: step.to_string(), done, total });
        };
        let plan = importer::analyze(&src, Path::new(&s.library), copy, &emit)?;
        let summary = plan.summary.clone();
        *app.state::<AppState>().import_plan.lock().unwrap() = Some(plan);
        Ok(summary)
    })
    .await?
}

/// Range pour de bon les fichiers de la dernière analyse.
#[tauri::command]
async fn import_apply(app: AppHandle) -> Result<importer::Applied, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let reports = if s.reports.is_empty() { app.state::<AppState>().data_dir.clone() } else { PathBuf::from(&s.reports) };
    blocking(move || {
        let plan = app.state::<AppState>().import_plan.lock().unwrap().take();
        let plan = plan.ok_or("Relance l'analyse avant de ranger.")?;
        let emit = |step: &str, done: usize, total: usize| {
            let _ = app.emit("import", ImportProgress { step: step.to_string(), done, total });
        };
        importer::apply(&plan, &reports, &emit)
    })
    .await?
}

/// Ouvre un dossier dans l'explorateur Windows (en le créant s'il n'existe pas).
#[tauri::command]
fn open_folder(path: String) {
    let _ = std::fs::create_dir_all(&path);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer").arg(&path).spawn();
    }
}

// ---------------------------------------------------------------------------
// Explorer un dossier ou un disque, en lecture seule
// ---------------------------------------------------------------------------

#[derive(Clone, serde::Serialize)]
struct ExploreProgress {
    seen: usize,
    found: usize,
}

#[tauri::command]
async fn explore_scan(app: AppHandle, path: String) -> Result<Vec<explore::Folder>, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    let cache = app.state::<AppState>().cache_dir.clone();
    blocking(move || {
        // la photothèque triée et le cache de PhotoPi ne sont pas explorés
        let mut exclude = vec![cache.clone()];
        if !s.library.is_empty() {
            exclude.push(PathBuf::from(&s.library));
        }
        let emit = |seen: usize, found: usize| {
            let _ = app.emit("explore", ExploreProgress { seen, found });
        };
        let mut folders = explore::scan(Path::new(&path), &exclude, &emit)?;
        let dir = cache.join("miniatures");
        for f in folders.iter_mut() {
            for it in f.items.iter_mut().filter(|i| i.kind != "video") {
                let t = media::thumb_file(Path::new(&it.path), &dir);
                if t.exists() {
                    it.thumb = Some(t.to_string_lossy().into_owned());
                }
            }
        }
        Ok(folders)
    })
    .await?
}

/// Copie des fichiers explorés dans le dossier d'arrivée, pour les ranger avec l'import.
#[tauri::command]
async fn explore_copy(app: AppHandle, paths: Vec<String>) -> Result<usize, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    if s.library.is_empty() {
        return Err("Choisis d'abord le dossier de ta photothèque.".into());
    }
    let inbox = import_dir_of(&s);
    blocking(move || explore::copy_to(&paths, &inbox)).await?
}

// ---------------------------------------------------------------------------
// Dater à la main : déplacer des photos dans le dossier d'un mois
// ---------------------------------------------------------------------------

/// `groups` : pour chaque élément, ses fichiers (photo, RAW, petite vidéo). Renvoie (ancien, nouveau chemin).
#[tauri::command]
async fn date_items(app: AppHandle, groups: Vec<Vec<String>>, month: String) -> Result<Vec<(String, String)>, String> {
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    if s.library.is_empty() {
        return Err("Choisis d'abord le dossier de ta photothèque.".into());
    }
    let dest = dating::month_dir(Path::new(&s.library), &month).ok_or("Mois invalide.")?;
    blocking(move || {
        let mut moved = Vec::new();
        let mut err = None;
        for g in &groups {
            match dating::move_group(g, &dest) {
                Ok(m) => moved.extend(m),
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }
        // une photo secrète le reste à son nouvel emplacement
        let state = app.state::<AppState>();
        let mut changed = false;
        {
            let mut l = state.lock.lock().unwrap();
            for (from, to) in &moved {
                if l.items.remove(&lock::path_id(from)) {
                    l.items.insert(lock::path_id(to));
                    changed = true;
                }
            }
        }
        if changed {
            save_lock(&state);
        }
        match err {
            Some(e) if moved.is_empty() => Err(e),
            _ => Ok(moved),
        }
    })
    .await?
}

fn dir_size(dir: &Path) -> u64 {
    let mut files = Vec::new();
    library::walk_files(dir, &mut files);
    files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum()
}

#[tauri::command]
async fn cache_size(app: AppHandle) -> Result<u64, String> {
    let dir = app.state::<AppState>().cache_dir.clone();
    blocking(move || dir_size(&dir.join("miniatures")) + dir_size(&dir.join("apercus"))).await
}

#[tauri::command]
async fn clear_cache(app: AppHandle) -> Result<(), String> {
    let dir = app.state::<AppState>().cache_dir.clone();
    blocking(move || {
        let _ = std::fs::remove_dir_all(dir.join("miniatures"));
        let _ = std::fs::remove_dir_all(dir.join("apercus"));
    })
    .await
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&cache_dir);
            let mut settings: Settings = read_json(&data_dir.join("settings.json")).unwrap_or_default();
            if settings.reports.is_empty() {
                if let Ok(d) = app.path().download_dir() {
                    settings.reports = d.to_string_lossy().into_owned();
                }
            }
            if settings.tile == 0 {
                settings.tile = 180;
            }
            let dates: library::DateCache = read_json(&data_dir.join("index.json")).unwrap_or_default();
            let lock_store: lock::LockStore = read_json(&data_dir.join("verrou.json")).unwrap_or_default();
            let hashes: HashMap<String, u64> = read_json(&data_dir.join("hashes.json")).unwrap_or_default();
            let ignored: HashSet<String> = read_json::<Vec<String>>(&data_dir.join("similaires_ignores.json"))
                .unwrap_or_default()
                .into_iter()
                .collect();
            app.manage(AppState {
                settings: Mutex::new(settings),
                data_dir,
                cache_dir,
                dates: Mutex::new(dates),
                pregen: Arc::new(AtomicU64::new(0)),
                hashes: Mutex::new(hashes),
                ignored: Mutex::new(ignored),
                lock: Mutex::new(lock_store),
                locked_items: Mutex::new(Vec::new()),
                revealed: AtomicBool::new(false),
                import_plan: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            pick_folder,
            scan_library,
            thumb,
            pregen_thumbs,
            display,
            photo_info,
            duplicates,
            trash_files,
            show_in_folder,
            similar_pairs,
            similar_ignore,
            lock_status,
            lock_set_password,
            lock_items,
            lock_open,
            lock_reveal,
            lock_hide,
            unlock_items,
            private_image,
            albums_get,
            import_info,
            import_analyze,
            import_apply,
            open_folder,
            explore_scan,
            explore_copy,
            date_items,
            albums_set,
            cache_size,
            clear_cache,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de PhotoPi");
}
