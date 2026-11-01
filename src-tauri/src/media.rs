//! Miniatures, aperçus (RAW, TIFF) et informations d'une photo.

use image::{DynamicImage, ImageFormat};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

/// Taille unique des miniatures (côté le plus long) : le curseur de taille ne recalcule jamais rien.
pub const THUMB: u32 = 400;

/// Compteur pour des fichiers temporaires uniques (plusieurs fils écrivent en même temps).
static TMP: AtomicUsize = AtomicUsize::new(0);

use crate::library::{ext_of, RAW_EXT};
use crate::util;

/// Nom du fichier en cache : dépend du chemin, de la taille, de la date de modification et de la taille voulue.
fn cache_name(src: &Path, max: u32) -> String {
    let (size, mtime) = fs::metadata(src)
        .map(|m| {
            let t = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            (m.len(), t)
        })
        .unwrap_or((0, 0));
    let mut h = DefaultHasher::new();
    src.to_string_lossy().hash(&mut h);
    size.hash(&mut h);
    mtime.hash(&mut h);
    max.hash(&mut h);
    format!("{:016x}.jpg", h.finish())
}

fn read_exif(src: &Path) -> Option<exif::Exif> {
    let f = fs::File::open(src).ok()?;
    let mut r = std::io::BufReader::new(f);
    exif::Reader::new().read_from_container(&mut r).ok()
}

/// Orientation EXIF (1 à 8), 1 par défaut.
fn orientation(ex: Option<&exif::Exif>) -> u32 {
    ex.and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY))
        .and_then(|f| f.value.get_uint(0))
        .unwrap_or(1)
}

fn apply_orientation(img: DynamicImage, o: u32) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// Aperçu JPEG intégré à un fichier RAW (le plus grand trouvé).
fn raw_preview(src: &Path) -> Result<DynamicImage, String> {
    let data = fs::read(src).map_err(|e| e.to_string())?;
    let off = util::largest_embedded_jpeg(&data).ok_or("Aucun aperçu dans ce fichier RAW")?;
    image::load_from_memory_with_format(&data[off..], ImageFormat::Jpeg).map_err(|e| e.to_string())
}

/// Ouvre une image, tournée dans le bon sens.
pub fn load(src: &Path) -> Result<DynamicImage, String> {
    let e = ext_of(src);
    let img = if RAW_EXT.contains(&e.as_str()) {
        raw_preview(src)?
    } else if e == "heic" || e == "heif" {
        return Err("Format HEIC non pris en charge".into());
    } else {
        image::open(src).map_err(|e| e.to_string())?
    };
    Ok(apply_orientation(img, orientation(read_exif(src).as_ref())))
}

fn save_jpeg(img: &DynamicImage, out: &Path, quality: u8) -> Result<(), String> {
    let tmp = out.with_extension(format!("{}.tmp", TMP.fetch_add(1, Ordering::Relaxed)));
    {
        let rgb = img.to_rgb8();
        let file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut w = std::io::BufWriter::new(file);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, quality)
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, out).map_err(|e| e.to_string())
}

/// Chemin de la miniature d'une photo dans le cache (qu'elle existe ou pas).
pub fn thumb_file(src: &Path, dir: &Path) -> PathBuf {
    dir.join(cache_name(src, THUMB))
}

/// Image réduite (côté le plus long = max) gardée en cache. Renvoie son chemin.
pub fn resized(src: &Path, dir: &Path, max: u32, quality: u8) -> Result<PathBuf, String> {
    let out = dir.join(cache_name(src, max));
    if out.exists() {
        return Ok(out);
    }
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let img = load(src)?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    save_jpeg(&img, &out, quality)?;
    Ok(out)
}

/// Chemin de l'aperçu grand format d'une photo dans le cache (qu'il existe ou pas).
pub fn preview_file(src: &Path, dir: &Path) -> PathBuf {
    dir.join(cache_name(src, 2560))
}

/// Image réduite fabriquée en mémoire, renvoyée en « data URL » : rien n'est écrit sur le disque
/// (utilisé pour les photos verrouillées).
pub fn data_url(src: &Path, max: u32) -> Result<String, String> {
    let img = load(src)?;
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgb = img.to_rgb8();
    let mut buf: Vec<u8> = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", util::base64(&buf)))
}

/// Empreinte visuelle d'une image (calculée sur sa miniature, rapide à lire).
pub fn dhash_file(p: &Path) -> Option<u64> {
    let img = image::open(p).ok()?;
    let small = img.resize_exact(9, 8, image::imageops::FilterType::Triangle).to_luma8();
    Some(util::dhash(small.as_raw()))
}

/// Le navigateur sait-il afficher ce fichier tel quel ?
pub fn displayable(src: &Path) -> bool {
    matches!(ext_of(src).as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp")
}

fn field(ex: &exif::Exif, tag: exif::Tag) -> Option<String> {
    let f = ex.get_field(tag, exif::In::PRIMARY)?;
    let s = f.display_value().with_unit(ex).to_string();
    let s = s.trim().trim_matches('"').trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn gps_coord(ex: &exif::Exif, tag: exif::Tag, ref_tag: exif::Tag) -> Option<f64> {
    let f = ex.get_field(tag, exif::In::PRIMARY)?;
    let v = match f.value {
        exif::Value::Rational(ref v) if v.len() >= 3 => v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0,
        _ => return None,
    };
    let r = field(ex, ref_tag).unwrap_or_default();
    Some(if r.starts_with('S') || r.starts_with('W') { -v } else { v })
}

/// Informations d'une photo (libellé, valeur), dans l'ordre d'affichage.
pub fn info(src: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let ex = read_exif(src);
    if let Some(ex) = ex.as_ref() {
        if let Some(d) = field(ex, exif::Tag::DateTimeOriginal) {
            out.push(("Prise de vue".into(), d));
        }
        let make = field(ex, exif::Tag::Make).unwrap_or_default();
        let model = field(ex, exif::Tag::Model).unwrap_or_default();
        let camera = if !make.is_empty() && model.to_lowercase().starts_with(&make.to_lowercase()) {
            model
        } else {
            format!("{make} {model}").trim().to_string()
        };
        if !camera.is_empty() {
            out.push(("Appareil".into(), camera));
        }
        if let Some(l) = field(ex, exif::Tag::LensModel) {
            out.push(("Objectif".into(), l));
        }
        let mut reglages = Vec::new();
        if let Some(v) = field(ex, exif::Tag::FNumber) {
            reglages.push(v);
        }
        if let Some(v) = field(ex, exif::Tag::ExposureTime) {
            reglages.push(v);
        }
        if let Some(v) = field(ex, exif::Tag::PhotographicSensitivity) {
            reglages.push(format!("ISO {v}"));
        }
        if let Some(v) = field(ex, exif::Tag::FocalLength) {
            reglages.push(v);
        }
        if !reglages.is_empty() {
            out.push(("Réglages".into(), reglages.join(" · ")));
        }
        if let (Some(lat), Some(lon)) = (
            gps_coord(ex, exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef),
            gps_coord(ex, exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef),
        ) {
            out.push(("GPS".into(), format!("{lat:.5}, {lon:.5}")));
        }
    }
    if displayable(src) || ext_of(src) == "tif" || ext_of(src) == "tiff" {
        if let Ok((w, h)) = image::image_dimensions(src) {
            let o = orientation(ex.as_ref());
            let (w, h) = if (5..=8).contains(&o) { (h, w) } else { (w, h) };
            out.push(("Dimensions".into(), format!("{w} × {h} px ({:.1} Mpx)", (w as f64 * h as f64) / 1e6)));
        }
    }
    if let Ok(m) = fs::metadata(src) {
        let size = m.len() as f64;
        let txt = if size >= 1e9 {
            format!("{:.2} Go", size / 1e9)
        } else if size >= 1e6 {
            format!("{:.1} Mo", size / 1e6)
        } else {
            format!("{:.0} ko", size / 1e3)
        };
        out.push(("Taille".into(), txt.replace('.', ",")));
    }
    out
}
