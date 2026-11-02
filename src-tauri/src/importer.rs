//! Import : range un dossier « en vrac » dans la photothèque, avec la même logique que le script
//! tri_photos.py (dates, photos Google, photos animées, événements, doublons exacts et recompressés).
//! L'analyse ne déplace rien ; le rangement déplace les fichiers et écrit un rapport rapport_tri_*.csv.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::library::{exif_date, kind_of, DUP_DIR};
use crate::util::{self, norm};

pub const A_TRIER: &str = "À trier";
pub const EXTRACT_DIR: &str = "_extraits";
/// Nombre minimum de fichiers pour garder un dossier comme événement
const EVENT_MIN: usize = 10;
const TRASH_DIRS: &[&str] = &["corbeille", "trash", "bin", "papierkorb", "papelera"];
const DATE_FIELDS: &[&str] = &["DateTimeOriginal", "CreateDate", "MediaCreateDate", "TrackCreateDate", "CreationTime", "DateCreated"];

pub type Progress<'a> = &'a dyn Fn(&str, usize, usize);

#[derive(Clone)]
pub struct Move {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub status: &'static str,
    pub month: String,
    pub dup_of: String,
}

#[derive(Serialize, Clone, Default)]
pub struct Summary {
    /// Photos et vidéos trouvées dans le dossier d'arrivée
    pub files: usize,
    pub sorted: usize,
    pub in_events: usize,
    pub unsorted: usize,
    pub dups: usize,
    pub quasi: usize,
    /// Autres fichiers, laissés en place
    pub ignored: usize,
    /// (mois, nom, nombre)
    pub events: Vec<(String, String, usize)>,
    pub exiftool: bool,
}

pub struct Plan {
    pub src: PathBuf,
    /// Import depuis un autre dossier : on copie seulement les nouveautés, la source n'est jamais modifiée
    pub copy: bool,
    pub moves: Vec<Move>,
    pub ignored: Vec<PathBuf>,
    pub summary: Summary,
}

#[derive(Serialize)]
pub struct Applied {
    pub moved: usize,
    pub skipped: usize,
    pub report: String,
}

fn fname(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn key(p: &Path) -> String {
    norm(&p.to_string_lossy()).trim_end_matches('\\').to_string()
}

fn parent_key(p: &Path) -> String {
    p.parent().map(key).unwrap_or_default()
}

fn ext_lower(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn is_media(p: &Path) -> bool {
    kind_of(p).is_some() || util::is_motion_name(&fname(p))
}

fn is_photo(p: &Path) -> bool {
    matches!(kind_of(p), Some("photo") | Some("raw"))
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(windows)]
fn quiet(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000); // pas de fenêtre de console
}
#[cfg(not(windows))]
fn quiet(_: &mut Command) {}

/// Tous les fichiers d'un dossier, sans les dossiers cachés ni ceux que `skip` écarte.
fn walk(dir: &Path, out: &mut Vec<PathBuf>, skip: &dyn Fn(&Path) -> bool) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if fname(&p).starts_with('.') {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_dir() => {
                if !skip(&p) {
                    walk(&p, out, skip);
                }
            }
            Ok(t) if t.is_file() => out.push(p),
            _ => {}
        }
    }
}

pub fn find_exiftool() -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&paths) {
            for n in ["exiftool.exe", "ExifTool.exe", "exiftool"] {
                let p = d.join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let mut bases = Vec::new();
    for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(v) = std::env::var_os(var) {
            bases.push(PathBuf::from(v));
        }
    }
    for b in bases {
        for sub in ["Programs\\ExifTool", "ExifTool"] {
            for n in ["ExifTool.exe", "exiftool.exe"] {
                let p = b.join(sub).join(n);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn zips(src: &Path) -> Vec<PathBuf> {
    let mut z: Vec<PathBuf> = fs::read_dir(src)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && ext_lower(p) == "zip").collect())
        .unwrap_or_default();
    z.sort();
    z
}

fn marker(src: &Path, zip: &Path) -> PathBuf {
    src.join(EXTRACT_DIR).join(format!(".{}.ok", fname(zip)))
}

/// Photos et vidéos en attente dans le dossier d'arrivée (archives pas encore extraites comprises).
pub fn pending(src: &Path) -> usize {
    if !src.is_dir() {
        return 0;
    }
    let mut files = Vec::new();
    walk(src, &mut files, &|p| TRASH_DIRS.contains(&fname(p).to_lowercase().as_str()));
    let media = files.iter().filter(|p| is_media(p)).count();
    let waiting = zips(src).iter().filter(|z| !marker(src, z).exists()).count();
    media + waiting
}

/// Extrait les archives .zip (Google Takeout) dans <arrivée>/_extraits, une seule fois chacune.
fn extract_zips(src: &Path, progress: Progress) -> Result<(), String> {
    let list = zips(src);
    if list.is_empty() {
        return Ok(());
    }
    let out = src.join(EXTRACT_DIR);
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    // tar de Windows (bsdtar) sait lire les .zip
    let tar = std::env::var_os("SystemRoot")
        .map(|r| PathBuf::from(r).join("System32").join("tar.exe"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"));
    for (i, z) in list.iter().enumerate() {
        progress("extract", i, list.len());
        let m = marker(src, z);
        if m.exists() {
            continue;
        }
        let mut cmd = Command::new(&tar);
        cmd.arg("-xf").arg(z).arg("-C").arg(&out);
        quiet(&mut cmd);
        let st = cmd.status().map_err(|e| format!("Extraction impossible : {e}"))?;
        if !st.success() {
            return Err(format!("Échec de l'extraction de {}", fname(z)));
        }
        let _ = fs::write(&m, "ok");
    }
    progress("extract", list.len(), list.len());
    Ok(())
}

/// Dates lues par ExifTool, par paquets (photos, RAW, vidéos).
fn exiftool_dates(
    tool: &Path,
    files: &[PathBuf],
    max_year: i32,
    step: &str,
    progress: Progress,
    months: &mut HashMap<String, String>,
    instants: &mut HashMap<String, String>,
) {
    const CHUNK: usize = 400;
    for (ci, chunk) in files.chunks(CHUNK).enumerate() {
        progress(step, ci * CHUNK, files.len());
        let argfile = std::env::temp_dir().join(format!("photopi_exif_{}_{}.txt", std::process::id(), ci));
        let list: String = chunk.iter().map(|p| format!("{}\n", p.to_string_lossy())).collect();
        if fs::write(&argfile, list).is_err() {
            continue;
        }
        let mut cmd = Command::new(tool);
        cmd.args(["-charset", "filename=utf8", "-json", "-q", "-q", "-fast", "-api", "QuickTimeUTC", "-d", "%Y-%m-%d %H:%M:%S"]);
        for f in DATE_FIELDS {
            cmd.arg(format!("-{f}"));
        }
        cmd.arg("-@").arg(&argfile);
        quiet(&mut cmd);
        let res = cmd.output();
        let _ = fs::remove_file(&argfile);
        let Ok(res) = res else { continue };
        let Ok(items) = serde_json::from_slice::<Vec<serde_json::Value>>(&res.stdout) else { continue };
        for it in items {
            let Some(sf) = it.get("SourceFile").and_then(|v| v.as_str()) else { continue };
            for f in DATE_FIELDS {
                let Some(v) = it.get(*f) else { continue };
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if let Some((m, inst)) = util::parse_date(&s, max_year) {
                    let k = norm(sf);
                    months.insert(k.clone(), m);
                    if let Some(i) = inst {
                        instants.insert(k, i);
                    }
                    break;
                }
            }
        }
    }
    progress(step, files.len(), files.len());
}

/// Dates des fichiers .json de Google Takeout : dossier -> (nom de photo -> « AAAA-MM »).
fn google_dates(dirs: &HashSet<PathBuf>) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    for d in dirs {
        let Ok(rd) = fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if ext_lower(&p) != "json" {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else { continue };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            let Some(title) = v.get("title").and_then(|t| t.as_str()) else { continue };
            let ts = v
                .get("photoTakenTime")
                .and_then(|t| t.get("timestamp"))
                .and_then(|t| t.as_str().map(|s| s.to_string()).or_else(|| t.as_i64().map(|n| n.to_string())))
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            if ts <= 0 {
                continue;
            }
            let month = util::date_from_unix(ts as u64)[..7].to_string();
            let (main, base) = util::google_keys(title, &fname(&p));
            let idx = out.entry(key(d)).or_default();
            idx.insert(main, month.clone());
            idx.entry(base).or_insert(month);
        }
    }
    out
}

fn google_date(p: &Path, google: &HashMap<String, HashMap<String, String>>) -> Option<String> {
    let idx = google.get(&parent_key(p))?;
    for v in util::google_variants(&fname(p)) {
        if let Some(m) = idx.get(&v) {
            return Some(m.clone());
        }
    }
    // nom tronqué par Google (47 caractères environ)
    let nom = fname(p).to_lowercase();
    idx.iter().find(|(t, _)| t.chars().count() >= 30 && nom.starts_with(t.as_str())).map(|(_, m)| m.clone())
}

fn file_hash(p: &Path) -> Option<[u8; 32]> {
    let mut f = fs::File::open(p).ok()?;
    let mut h = util::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finish())
}

/// Analyse le dossier d'arrivée et prépare le rangement, sans rien déplacer.
pub fn analyze(src: &Path, dest: &Path, copy: bool, progress: Progress) -> Result<Plan, String> {
    if !src.is_dir() {
        return Err(format!("Dossier d'arrivée introuvable : {}", src.display()));
    }
    if !copy {
        // les archives ne sont extraites que dans le dossier d'arrivée (jamais dans une source en copie)
        extract_zips(src, progress)?;
    }

    // 1. Inventaire (la corbeille Google n'est pas importée)
    progress("scan", 0, 0);
    let src_k = key(src);
    let mut all = Vec::new();
    walk(src, &mut all, &|p| TRASH_DIRS.contains(&fname(p).to_lowercase().as_str()));
    let mut files = Vec::new();
    let mut ignored = Vec::new();
    for p in all {
        if is_media(&p) {
            files.push(p);
        } else if !["json", "zip", "ok", "html"].contains(&ext_lower(&p).as_str()) {
            ignored.push(p);
        }
    }
    let mut summary = Summary { files: files.len(), ignored: ignored.len(), ..Default::default() };
    let tool = find_exiftool();
    summary.exiftool = tool.is_some();
    if files.is_empty() {
        return Ok(Plan { src: src.to_path_buf(), copy, moves: Vec::new(), ignored, summary });
    }

    // Photothèque déjà rangée (sans le dossier d'arrivée ni les doublons mis de côté)
    let dup_k = key(&dest.join(DUP_DIR));
    let mut existing = Vec::new();
    walk(dest, &mut existing, &|p| {
        let k = key(p);
        k == src_k || k == dup_k
    });
    existing.retain(|p| is_media(p));
    let existing_set: HashSet<String> = existing.iter().map(|p| key(p)).collect();

    // 2. Dates : ExifTool, puis EXIF des photos, puis fichiers .json de Google, puis photo d'une photo animée
    let max_year = util::date_from_unix(now_secs())[..4].parse::<i32>().unwrap_or(2100) + 1;
    let mut months: HashMap<String, String> = HashMap::new();
    let mut instants: HashMap<String, String> = HashMap::new();
    if let Some(t) = &tool {
        exiftool_dates(t, &files, max_year, "dates", progress, &mut months, &mut instants);
    }
    for p in &files {
        let k = key(p);
        if months.contains_key(&k) || !is_photo(p) {
            continue;
        }
        if let Some((m, i)) = exif_date(p).and_then(|d| util::parse_date(&d, max_year)) {
            months.insert(k.clone(), m);
            if let Some(i) = i {
                instants.insert(k, i);
            }
        }
    }
    let dirs: HashSet<PathBuf> = files.iter().filter_map(|p| p.parent().map(|d| d.to_path_buf())).collect();
    let google = google_dates(&dirs);
    if !google.is_empty() {
        for p in &files {
            let k = key(p);
            if !months.contains_key(&k) {
                if let Some(m) = google_date(p, &google) {
                    months.insert(k, m);
                }
            }
        }
    }
    for p in &files {
        let k = key(p);
        let n = fname(p);
        if !util::is_motion_name(&n) || months.contains_key(&k) {
            continue;
        }
        let base = util::final_name(&n).0;
        for ext in [".jpg", ".jpeg", ".heic"] {
            let ph = key(&p.with_file_name(format!("{base}{ext}")));
            if let Some(m) = months.get(&ph).cloned() {
                if let Some(i) = instants.get(&ph).cloned() {
                    instants.insert(k.clone(), i);
                }
                months.insert(k.clone(), m);
                break;
            }
        }
    }

    // un RAW sans date prend celle de son JPG jumeau (même nom, même dossier)
    for p in &files {
        let k = key(p);
        if kind_of(p) != Some("raw") || months.contains_key(&k) {
            continue;
        }
        let stem = util::split_ext(&fname(p)).0.to_string();
        for ext in [".jpg", ".jpeg", ".heic", ".tif", ".png"] {
            let ph = key(&p.with_file_name(format!("{stem}{ext}")));
            if let Some(m) = months.get(&ph).cloned() {
                if let Some(i) = instants.get(&ph).cloned() {
                    instants.insert(k.clone(), i);
                }
                months.insert(k.clone(), m);
                break;
            }
        }
    }

    // 3. Événements : dossiers d'origine bien nommés et assez fournis
    let mut by_dir: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    for p in &files {
        by_dir.entry(parent_key(p)).or_default().push(p);
    }
    let mut events: HashMap<String, (String, Option<String>)> = HashMap::new();
    for (d, list) in &by_dir {
        if *d == src_k || list.len() < EVENT_MIN {
            continue;
        }
        let dir_name = list[0].parent().map(fname).unwrap_or_default();
        if let Some(name) = util::event_name(&dir_name) {
            let mut ms: Vec<&String> = list.iter().filter_map(|p| months.get(&key(p))).collect();
            ms.sort();
            events.insert(d.clone(), (name, ms.first().map(|s| (*s).clone())));
        }
    }

    // 4. Doublons exacts (empreinte calculée seulement pour les tailles en commun)
    let new_set: HashSet<String> = files.iter().map(|p| key(p)).collect();
    let mut by_size: HashMap<u64, Vec<&PathBuf>> = HashMap::new();
    for p in files.iter().chain(existing.iter()) {
        if let Ok(m) = fs::metadata(p) {
            by_size.entry(m.len()).or_default().push(p);
        }
    }
    let groups: Vec<&Vec<&PathBuf>> = by_size.values().filter(|g| g.len() > 1 && g.iter().any(|p| new_set.contains(&key(p)))).collect();
    let total: usize = groups.iter().map(|g| g.len()).sum();
    let mut by_hash: HashMap<[u8; 32], Vec<&PathBuf>> = HashMap::new();
    let mut done = 0;
    for g in groups {
        for p in g.iter() {
            done += 1;
            if done % 20 == 0 {
                progress("dups", done, total);
            }
            if let Some(h) = file_hash(p) {
                by_hash.entry(h).or_default().push(*p);
            }
        }
    }
    progress("dups", total, total);
    let mut dup_of: HashMap<String, PathBuf> = HashMap::new();
    for (_, mut g) in by_hash {
        if g.len() < 2 {
            continue;
        }
        // un fichier déjà rangé reste l'exemplaire de référence ; sinon celui d'un événement, puis le chemin le plus court
        g.sort_by_key(|p| {
            let s = p.to_string_lossy().into_owned();
            (!existing_set.contains(&key(p)), !events.contains_key(&parent_key(p)), s.len(), s)
        });
        let keep = g[0].clone();
        for p in &g[1..] {
            let k = key(p);
            if !existing_set.contains(&k) {
                dup_of.insert(k, keep.clone());
            }
        }
    }

    // 5. Photos déjà rangées mais recompressées (Google) : même nom, même date à la seconde
    let mut quasi: HashSet<String> = HashSet::new();
    if !existing.is_empty() && !instants.is_empty() {
        let stems: HashSet<String> = files.iter().map(|p| util::signature(&fname(p)).0).collect();
        let cands: Vec<PathBuf> = existing.iter().filter(|p| stems.contains(&util::signature(&fname(p)).0)).cloned().collect();
        if !cands.is_empty() {
            let mut cm = HashMap::new();
            let mut ci = HashMap::new();
            if let Some(t) = &tool {
                exiftool_dates(t, &cands, max_year, "quasi", progress, &mut cm, &mut ci);
            }
            for p in &cands {
                let k = key(p);
                if ci.contains_key(&k) || !is_photo(p) {
                    continue;
                }
                if let Some((_, Some(i))) = exif_date(p).and_then(|d| util::parse_date(&d, max_year)) {
                    ci.insert(k, i);
                }
            }
            let mut index: HashMap<(String, String, String), PathBuf> = HashMap::new();
            for p in &cands {
                if let Some(i) = ci.get(&key(p)) {
                    let (s, e) = util::signature(&fname(p));
                    index.entry((i.clone(), s, e)).or_insert_with(|| p.clone());
                }
            }
            for p in &files {
                let k = key(p);
                if dup_of.contains_key(&k) {
                    continue;
                }
                let Some(i) = instants.get(&k) else { continue };
                let (s, e) = util::signature(&fname(p));
                if let Some(r) = index.get(&(i.clone(), s, e)) {
                    dup_of.insert(k.clone(), r.clone());
                    quasi.insert(k);
                }
            }
        }
    }

    // 6. Plan de rangement (noms en conflit gérés, RAW + JPG gardés sous le même nom)
    progress("plan", 0, 0);
    let mut sorted: Vec<&PathBuf> = files.iter().collect();
    sorted.sort_by_key(|p| p.to_string_lossy().into_owned());
    let mut used: HashMap<String, HashMap<String, (String, String)>> = HashMap::new();
    let mut fams: HashMap<(String, (String, String)), String> = HashMap::new();
    let mut planned: HashSet<String> = HashSet::new();
    let mut moves = Vec::new();
    let mut event_counts: HashMap<String, usize> = HashMap::new();
    for p in sorted {
        let k = key(p);
        let month = months.get(&k).cloned();
        let (dir, status): (PathBuf, &'static str) = if dup_of.contains_key(&k) {
            let rel = p.parent().and_then(|d| d.strip_prefix(src).ok()).map(|r| r.to_path_buf()).unwrap_or_default();
            (dest.join(DUP_DIR).join(rel), if quasi.contains(&k) { "quasi-doublon" } else { "doublon" })
        } else if let Some((name, first)) = events.get(&parent_key(p)) {
            *event_counts.entry(parent_key(p)).or_default() += 1;
            match first {
                Some(m) => (dest.join(&m[..4]).join(format!("{m} {name}")), "événement"),
                None => (dest.join(A_TRIER).join(name), "événement"),
            }
        } else if let Some(m) = &month {
            (dest.join(&m[..4]).join(m), "rangé")
        } else {
            (dest.join(A_TRIER), "sans date")
        };
        let (stem, ext) = util::final_name(&fname(p));
        let family = (parent_key(p), stem.to_lowercase());
        let kd = key(&dir);
        let radical = if let Some(r) = fams.get(&(kd.clone(), family.clone())) {
            r.clone()
        } else {
            let mut n = 0;
            let mut radical = stem.clone();
            loop {
                let owner = used.get(&kd).and_then(|m| m.get(&radical.to_lowercase()));
                let free = owner.map_or(true, |o| *o == family);
                if free && !dir.join(format!("{radical}{ext}")).exists() {
                    break;
                }
                n += 1;
                radical = format!("{stem}_{n}");
            }
            used.entry(kd.clone()).or_default().insert(radical.to_lowercase(), family.clone());
            fams.insert((kd.clone(), family), radical.clone());
            radical
        };
        let mut dst = dir.join(format!("{radical}{ext}"));
        let mut n = 0;
        while planned.contains(&key(&dst)) || dst.exists() {
            n += 1;
            dst = dir.join(format!("{radical}_{n}{ext}"));
        }
        planned.insert(key(&dst));
        match status {
            "doublon" => summary.dups += 1,
            "quasi-doublon" => summary.quasi += 1,
            "événement" => summary.in_events += 1,
            "rangé" => summary.sorted += 1,
            _ => summary.unsorted += 1,
        }
        moves.push(Move {
            src: p.clone(),
            dst,
            status,
            month: month.unwrap_or_default(),
            dup_of: dup_of.get(&k).map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        });
    }
    let mut ev: Vec<(String, String, usize)> = events
        .iter()
        .filter_map(|(d, (name, first))| event_counts.get(d).map(|n| (first.clone().unwrap_or_default(), name.clone(), *n)))
        .collect();
    ev.sort();
    summary.events = ev;
    Ok(Plan { src: src.to_path_buf(), copy, moves, ignored, summary })
}

fn csv_field(s: &str) -> String {
    if s.contains(';') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Supprime les dossiers devenus vides (pas le dossier d'arrivée lui-même).
fn remove_empty(dir: &Path, root: bool) {
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                remove_empty(&e.path(), false);
            }
        }
    }
    if !root && fs::read_dir(dir).map(|mut r| r.next().is_none()).unwrap_or(false) {
        let _ = fs::remove_dir(dir);
    }
}

/// Range pour de bon : déplace les fichiers (jamais d'écrasement) et écrit le rapport.
pub fn apply(plan: &Plan, reports: &Path, progress: Progress) -> Result<Applied, String> {
    let total = plan.moves.len();
    let mut moved = 0;
    let mut skipped = 0;
    let mut rows: Vec<String> = Vec::new();
    let step = if plan.copy { "copy" } else { "move" };
    for (i, m) in plan.moves.iter().enumerate() {
        if i % 20 == 0 {
            progress(step, i, total);
        }
        if plan.copy && (m.status == "doublon" || m.status == "quasi-doublon") {
            // déjà dans la photothèque : rien à copier
            rows.push(format!("{};;déjà présent ({});{};{}", csv_field(&m.src.to_string_lossy()), m.status, m.month, csv_field(&m.dup_of)));
            continue;
        }
        if !m.src.is_file() || m.dst.exists() {
            skipped += 1;
            continue;
        }
        if let Some(d) = m.dst.parent() {
            if fs::create_dir_all(d).is_err() {
                skipped += 1;
                continue;
            }
        }
        let ok = if plan.copy {
            fs::copy(&m.src, &m.dst).is_ok()
        } else {
            fs::rename(&m.src, &m.dst).is_ok() || (fs::copy(&m.src, &m.dst).is_ok() && fs::remove_file(&m.src).is_ok())
        };
        if !ok {
            skipped += 1;
            continue;
        }
        moved += 1;
        rows.push(
            [m.src.to_string_lossy().as_ref(), m.dst.to_string_lossy().as_ref(), m.status, m.month.as_str(), m.dup_of.as_str()]
                .iter()
                .map(|s| csv_field(s))
                .collect::<Vec<_>>()
                .join(";"),
        );
    }
    progress(step, total, total);
    if !plan.copy {
        remove_empty(&plan.src, true);
    }

    for p in &plan.ignored {
        rows.push(format!("{};;ignoré (pas une photo/vidéo);;", csv_field(&p.to_string_lossy())));
    }
    let stamp: String = util::date_from_unix(now_secs()).chars().filter(|c| c.is_ascii_digit()).collect();
    let name = format!("rapport_tri_{}_{}.csv", &stamp[..8], &stamp[8..]);
    let _ = fs::create_dir_all(reports);
    let path = reports.join(name);
    let mut text = String::from("\u{feff}source;destination;statut;mois;doublon de\r\n");
    for r in rows {
        text.push_str(&r);
        text.push_str("\r\n");
    }
    fs::write(&path, text).map_err(|e| format!("Rapport non écrit : {e}"))?;
    Ok(Applied { moved, skipped, report: path.to_string_lossy().into_owned() })
}
