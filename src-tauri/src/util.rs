//! Fonctions pures, sans dépendance externe (testables seules).

/// Normalise un chemin pour comparer : séparateurs Windows et minuscules.
pub fn norm(s: &str) -> String {
    s.replace('/', "\\").to_lowercase()
}

/// Lit un CSV séparé par des « ; » (format du script tri_photos.py, guillemets possibles).
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            if c == '"' {
                if it.peek() == Some(&'"') {
                    field.push('"');
                    it.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else {
            match c {
                '"' => quoted = true,
                ';' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Dimensions d'un JPEG (largeur, hauteur) lues dans son en-tête, sans le décoder.
/// Renvoie None pour un JPEG sans perte (données brutes d'un RAW) ou illisible.
pub fn jpeg_dims(d: &[u8]) -> Option<(u32, u32)> {
    if d.len() < 4 || d[0] != 0xFF || d[1] != 0xD8 {
        return None;
    }
    let mut p = 2;
    while p + 4 <= d.len() {
        if d[p] != 0xFF {
            return None;
        }
        let m = d[p + 1];
        if m == 0xFF {
            p += 1;
            continue;
        }
        if m == 0x01 || (0xD0..=0xD7).contains(&m) {
            p += 2;
            continue;
        }
        let len = u16::from_be_bytes([d[p + 2], d[p + 3]]) as usize;
        if len < 2 {
            return None;
        }
        match m {
            0xC0 | 0xC1 | 0xC2 => {
                if p + 9 > d.len() {
                    return None;
                }
                let h = u16::from_be_bytes([d[p + 5], d[p + 6]]) as u32;
                let w = u16::from_be_bytes([d[p + 7], d[p + 8]]) as u32;
                return if w > 0 && h > 0 { Some((w, h)) } else { None };
            }
            // sans perte, arithmétique, ou données avant toute dimension : pas exploitable
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDA | 0xD9 => return None,
            _ => p += 2 + len,
        }
    }
    None
}

/// Cherche dans un fichier RAW l'aperçu JPEG intégré le plus grand.
/// Renvoie son décalage dans le fichier, ou None.
pub fn largest_embedded_jpeg(data: &[u8]) -> Option<usize> {
    let mut best: Option<(usize, u64)> = None;
    let mut candidates = 0;
    let mut i = 0;
    while i + 3 < data.len() && candidates < 400 {
        if data[i] == 0xFF && data[i + 1] == 0xD8 && data[i + 2] == 0xFF {
            candidates += 1;
            if let Some((w, h)) = jpeg_dims(&data[i..]) {
                let area = w as u64 * h as u64;
                if best.map_or(true, |(_, a)| area > a) {
                    best = Some((i, area));
                }
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    best.map(|(o, _)| o)
}

/// Date « AAAA-MM-JJ HH:MM:SS » (UTC) d'un horodatage Unix en secondes.
pub fn date_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Algorithme de Howard Hinnant (jours depuis 1970 -> date civile)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, d, rem / 3600, (rem % 3600) / 60, rem % 60
    )
}

/// Date EXIF « AAAA:MM:JJ HH:MM:SS » -> « AAAA-MM-JJ HH:MM:SS », None si vide ou nulle.
pub fn exif_date_to_iso(s: &str) -> Option<String> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 || !b[..4].iter().all(|c| c.is_ascii_digit()) || &s[..4] == "0000" {
        return None;
    }
    let mut out = s.get(..19)?.to_string();
    out.replace_range(4..5, "-");
    out.replace_range(7..8, "-");
    Some(out)
}

/// Lit un nom de dossier de mois : « 2023-07 » -> (Some("2023-07"), None),
/// « 2023-07 Vacances » -> (Some("2023-07"), Some("Vacances")), « Divers » -> (None, Some("Divers")).
pub fn parse_album_name(name: &str) -> (Option<String>, Option<String>) {
    let b = name.as_bytes();
    let is_month = b.len() >= 7
        && b[..4].iter().all(|c| c.is_ascii_digit())
        && b[4] == b'-'
        && b[5].is_ascii_digit()
        && b[6].is_ascii_digit()
        && (b.len() == 7 || b[7] == b' ');
    if is_month {
        let rest = name[7..].trim();
        (Some(name[..7].to_string()), if rest.is_empty() { None } else { Some(rest.to_string()) })
    } else {
        (None, Some(name.to_string()))
    }
}

/// Est-ce un nom d'année (« 2023 ») ?
pub fn is_year(name: &str) -> bool {
    name.len() == 4 && name.bytes().all(|c| c.is_ascii_digit())
}

/// Clé d'appariement d'une petite vidéo de photo animée : « PXL_x.MP.mp4 » -> Some("pxl_x.mp").
pub fn motion_key(file_name: &str) -> Option<String> {
    let low = file_name.to_lowercase();
    let stem = low.strip_suffix(".mp4")?;
    if stem.ends_with(".mp") {
        Some(stem.to_string())
    } else {
        None
    }
}

/// Empreinte visuelle (dHash) d'une image réduite à 9 × 8 en niveaux de gris, ligne par ligne :
/// chaque bit dit si un pixel est plus clair que son voisin de droite.
pub fn dhash(gray: &[u8]) -> u64 {
    let mut h = 0u64;
    if gray.len() < 72 {
        return h;
    }
    for y in 0..8 {
        for x in 0..8 {
            h <<= 1;
            if gray[y * 9 + x] > gray[y * 9 + x + 1] {
                h |= 1;
            }
        }
    }
    h
}

/// Nombre de bits différents entre deux empreintes (0 = identiques, 64 = opposées).
pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Une empreinte presque vide (image unie, très sombre) ressemble à tout : on l'écarte.
pub fn informative(h: u64) -> bool {
    let n = h.count_ones();
    (6..=58).contains(&n)
}

/// Clé d'une paire de photos, indépendante de l'ordre.
pub fn pair_id(a: &str, b: &str) -> String {
    let (a, b) = (norm(a), norm(b));
    if a <= b {
        format!("{a}|{b}")
    } else {
        format!("{b}|{a}")
    }
}

const K256: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 (FIPS 180-4), par morceaux : verrou (empreintes) et doublons exacts (fichiers lus par blocs).
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    used: usize,
    len: u64,
}

impl Sha256 {
    pub fn new() -> Self {
        Sha256 {
            h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            buf: [0; 64],
            used: 0,
            len: 0,
        }
    }

    fn block(h: &mut [u32; 8], chunk: &[u8]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K256[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(data.len() as u64);
        if self.used > 0 {
            let take = (64 - self.used).min(data.len());
            self.buf[self.used..self.used + take].copy_from_slice(&data[..take]);
            self.used += take;
            data = &data[take..];
            if self.used == 64 {
                let b = self.buf;
                Self::block(&mut self.h, &b);
                self.used = 0;
            }
        }
        while data.len() >= 64 {
            Self::block(&mut self.h, &data[..64]);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.used = data.len();
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bits = self.len.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        while (self.used + pad.len()) % 64 != 56 {
            pad.push(0);
        }
        pad.extend_from_slice(&bits.to_be_bytes());
        self.update(&pad);
        let mut out = [0u8; 32];
        for (i, v) in self.h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut s = Sha256::new();
    s.update(data);
    s.finish()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Encodage base64 (pour afficher une image fabriquée en mémoire, sans fichier).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// ---------------------------------------------------------------------------
// Import : règles reprises du script tri_photos.py
// ---------------------------------------------------------------------------

fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())
}

/// « nom.ext » -> ("nom", ".ext") (dernier point, comme os.path.splitext).
pub fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

/// « IMG(2) » -> ("IMG", Some("(2)")).
pub fn split_copy(stem: &str) -> (&str, Option<&str>) {
    if stem.ends_with(')') {
        if let Some(i) = stem.rfind('(') {
            if all_digits(&stem[i + 1..stem.len() - 1]) {
                return (&stem[..i], Some(&stem[i..]));
            }
        }
    }
    (stem, None)
}

/// Vidéo d'une photo animée Google/Pixel : « PXL_x.MP » ou sa copie « PXL_x.MP~2 ».
pub fn is_motion_name(name: &str) -> bool {
    let low = name.to_lowercase();
    match low.rfind('.') {
        Some(i) if i > 0 => {
            let ext = &low[i + 1..];
            ext == "mp" || (ext.starts_with("mp~") && all_digits(&ext[3..]))
        }
        _ => false,
    }
}

/// Nom sous lequel un fichier importé est rangé : (radical, extension avec le point).
/// « PXL_x.MP » devient « PXL_x.MP.mp4 », à côté de sa photo « PXL_x.MP.jpg ».
pub fn final_name(name: &str) -> (String, String) {
    if is_motion_name(name) {
        let base = match name.rfind('~') {
            Some(i) if all_digits(&name[i + 1..]) => &name[..i],
            _ => name,
        };
        return (base.to_string(), ".mp4".to_string());
    }
    let (s, e) = split_ext(name);
    (s.to_string(), e.to_string())
}

const EDITED: &[&str] = &["-edited", "-modifié", "-modifie", "-bearbeitet", "-editado"];

/// Signature d'une photo pour repérer une copie recompressée : (radical, extension) normalisés.
/// Avec la date à la seconde, deux fichiers de même signature sont la même photo.
pub fn signature(name: &str) -> (String, String) {
    let (stem, ext) = final_name(name);
    let low = stem.to_lowercase();
    let mut stem = split_copy(&low).0.to_string();
    for suf in EDITED {
        if let Some(s) = stem.strip_suffix(suf) {
            stem = s.to_string();
            break;
        }
    }
    // IMG_1234_1 -> IMG_1234 (renommage pour éviter un conflit)
    if let Some(i) = stem.rfind('_') {
        if all_digits(&stem[i + 1..]) {
            let rest = &stem[..i];
            if let Some(j) = rest.rfind('_') {
                if all_digits(&rest[j + 1..]) {
                    stem = rest.to_string();
                }
            }
        }
    }
    let ext = ext.to_lowercase();
    let ext = match ext.as_str() {
        ".jpeg" => ".jpg".to_string(),
        ".tiff" => ".tif".to_string(),
        ".heif" => ".heic".to_string(),
        _ => ext,
    };
    (stem, ext)
}

const GENERIC: &[&str] = &[
    "dcim", "camera", "appareil photo", "photos", "photo", "images", "image", "pictures", "nouveau dossier", "new folder",
    "divers", "download", "downloads", "téléchargements", "telechargements", "whatsapp images", "screenshots",
    "captures d'écran", "captures", "export", "exports", "sauvegarde", "backup", "a trier", "à trier", "misc", "autres", "vrac",
];
const GOOGLE_PREFIXES: &[&str] = &[
    "photos from", "photo from", "photos de", "photo de", "photos du", "photo du", "untitled", "sans titre", "takeout",
    "google photos", "_extraits",
];

/// Nom d'événement exploitable à partir d'un nom de dossier (« 2023-07 Vacances » -> « Vacances »), ou None.
pub fn event_name(dir: &str) -> Option<String> {
    let b = dir.as_bytes();
    if b.len() == 8 && b[..3].iter().all(|c| c.is_ascii_digit()) && b[3..].iter().all(|c| c.is_ascii_alphabetic() || *c == b'_') {
        return None; // 100CANON, 101MSDCF…
    }
    let low = dir.to_lowercase();
    if GOOGLE_PREFIXES.iter().any(|p| low.starts_with(p)) {
        return None;
    }
    let name = dir.trim_start_matches(|c: char| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == '_' || c == '-').trim();
    if !name.chars().any(|c| c.is_alphabetic()) || GENERIC.contains(&name.to_lowercase().as_str()) {
        return None;
    }
    Some(name.to_string())
}

/// Date lue (« AAAA-MM-JJ HH:MM:SS… ») -> (« AAAA-MM », date à la seconde si présente).
pub fn parse_date(v: &str, max_year: i32) -> Option<(String, Option<String>)> {
    let b = v.as_bytes();
    if b.len() < 7 || !b[..4].iter().all(|c| c.is_ascii_digit()) || b[4] != b'-' || !b[5..7].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let y: i32 = v[..4].parse().ok()?;
    let m: u32 = v[5..7].parse().ok()?;
    if !(1970..=max_year).contains(&y) || !(1..=12).contains(&m) {
        return None;
    }
    let full = b.len() >= 19
        && b[7] == b'-'
        && b[10] == b' '
        && b[13] == b':'
        && b[16] == b':'
        && [8, 9, 11, 12, 14, 15, 17, 18].iter().all(|&i| b[i].is_ascii_digit());
    Some((v[..7].to_string(), if full { Some(v[..19].to_string()) } else { None }))
}

/// Clés d'une photo décrite par un JSON de Google Takeout : (titre avec n° de copie, nom tiré du json).
pub fn google_keys(title: &str, json_name: &str) -> (String, String) {
    let tl = title.to_lowercase();
    let jn = json_name.to_lowercase();
    let base = jn.strip_suffix(".json").unwrap_or(&jn);
    let (base, copy) = split_copy(base);
    const SUP: &str = ".supplemental-metad";
    let base = match base.rfind(SUP) {
        Some(i) if base[i + SUP.len()..].bytes().all(|c| c.is_ascii_lowercase()) => &base[..i],
        _ => base,
    };
    let main = match copy {
        Some(c) => {
            let (s, e) = split_ext(&tl);
            format!("{s}{c}{e}")
        }
        None => tl.clone(),
    };
    (main, base.to_string())
}

/// Noms sous lesquels Google a pu décrire une photo (copie « (1) », version retouchée).
pub fn google_variants(name: &str) -> Vec<String> {
    let nom = name.to_lowercase();
    let (stem, ext) = split_ext(&nom);
    let mut v = vec![nom.clone()];
    let (b, copy) = split_copy(stem);
    if let Some(c) = copy {
        v.push(format!("{b}{ext}{c}"));
        v.push(format!("{b}{ext}"));
    }
    for suf in EDITED {
        if let Some(s) = stem.strip_suffix(suf) {
            v.push(format!("{s}{ext}"));
        }
    }
    v
}

/// SipHash-1-3 à clés nulles : la même empreinte que l'ancien DefaultHasher de Rust,
/// figée ici pour que les noms des miniatures en cache ne changent jamais d'une version à l'autre.
pub fn siphash13(msg: &[u8]) -> u64 {
    let (mut v0, mut v1, mut v2, mut v3) = (0x736f6d6570736575u64, 0x646f72616e646f6du64, 0x6c7967656e657261u64, 0x7465646279746573u64);
    fn round(v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64) {
        *v0 = v0.wrapping_add(*v1);
        *v1 = v1.rotate_left(13) ^ *v0;
        *v0 = v0.rotate_left(32);
        *v2 = v2.wrapping_add(*v3);
        *v3 = v3.rotate_left(16) ^ *v2;
        *v0 = v0.wrapping_add(*v3);
        *v3 = v3.rotate_left(21) ^ *v0;
        *v2 = v2.wrapping_add(*v1);
        *v1 = v1.rotate_left(17) ^ *v2;
        *v2 = v2.rotate_left(32);
    }
    let mut chunks = msg.chunks_exact(8);
    for c in &mut chunks {
        let m = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]);
        v3 ^= m;
        round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }
    let mut b = (msg.len() as u64 & 0xff) << 56;
    for (i, x) in chunks.remainder().iter().enumerate() {
        b |= (*x as u64) << (8 * i);
    }
    v3 ^= b;
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= b;
    v2 ^= 0xff;
    for _ in 0..3 {
        round(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^ v1 ^ v2 ^ v3
}

/// Nom d'une miniature en cache : chemin, taille, date de modification et taille voulue.
pub fn cache_key(path: &str, size: u64, mtime: u64, max: u32) -> String {
    let mut msg = Vec::with_capacity(path.len() + 21);
    msg.extend_from_slice(path.as_bytes());
    msg.push(0xff);
    msg.extend_from_slice(&size.to_le_bytes());
    msg.extend_from_slice(&mtime.to_le_bytes());
    msg.extend_from_slice(&max.to_le_bytes());
    format!("{:016x}.jpg", siphash13(&msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv() {
        let t = "\u{feff}source;destination;statut;mois;doublon de\r\n\"D:\\a;b.jpg\";D:\\x.jpg;doublon;2023-01;\"D:\\\"\"q\"\".jpg\"\r\nD:\\c.jpg;;ignoré;;\r\n";
        let r = parse_csv(t);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0][0], "source");
        assert_eq!(r[1][0], "D:\\a;b.jpg");
        assert_eq!(r[1][4], "D:\\\"q\".jpg");
        assert_eq!(r[2][1], "");
        assert_eq!(r[2].len(), 5);
    }

    fn fake_jpeg(w: u16, h: u16, sof: u8) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        v.extend_from_slice(&[0xFF, sof, 0x00, 0x0B, 0x08]);
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&[0x01, 0x01, 0x11, 0x00, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn dims_and_raw() {
        assert_eq!(jpeg_dims(&fake_jpeg(160, 120, 0xC0)), Some((160, 120)));
        assert_eq!(jpeg_dims(&fake_jpeg(6000, 4000, 0xC3)), None);
        let mut raw = vec![0u8; 100];
        let a = raw.len();
        raw.extend(fake_jpeg(160, 120, 0xC0));
        raw.extend(vec![7u8; 50]);
        let b = raw.len();
        raw.extend(fake_jpeg(1620, 1080, 0xC0));
        raw.extend(fake_jpeg(6000, 4000, 0xC3)); // données brutes sans perte : ignorées
        raw.extend(vec![0xFF, 0xD8, 0xFF, 0x12]); // faux positif
        assert_eq!(largest_embedded_jpeg(&raw), Some(b));
        assert_ne!(Some(a), largest_embedded_jpeg(&raw));
    }

    #[test]
    fn dates() {
        assert_eq!(date_from_unix(0), "1970-01-01 00:00:00");
        assert_eq!(date_from_unix(1_700_000_000), "2023-11-14 22:13:20");
        assert_eq!(date_from_unix(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(exif_date_to_iso("2023:07:14 15:30:22"), Some("2023-07-14 15:30:22".into()));
        assert_eq!(exif_date_to_iso("0000:00:00 00:00:00"), None);
        assert_eq!(exif_date_to_iso("    "), None);
    }

    #[test]
    fn names() {
        assert_eq!(parse_album_name("2023-07"), (Some("2023-07".into()), None));
        assert_eq!(parse_album_name("2024-09 Rénovation"), (Some("2024-09".into()), Some("Rénovation".into())));
        assert_eq!(parse_album_name("Peri_15ans_web"), (None, Some("Peri_15ans_web".into())));
        assert_eq!(parse_album_name("2023-071"), (None, Some("2023-071".into())));
        assert!(is_year("2023") && !is_year("À trier"));
        assert_eq!(motion_key("PXL_20240601_101010123.MP.mp4"), Some("pxl_20240601_101010123.mp".into()));
        assert_eq!(motion_key("PXL_x.MP_1.mp4"), None);
        assert_eq!(motion_key("video.mp4"), None);
        assert_eq!(norm("D:/A/B.JPG"), "d:\\a\\b.jpg");
    }

    #[test]
    fn similar() {
        // dégradé horizontal décroissant : tous les bits à 1
        let g: Vec<u8> = (0..72).map(|i| 255 - (i % 9) as u8 * 20).collect();
        assert_eq!(dhash(&g), u64::MAX);
        let mut g2 = g.clone();
        g2[0] = 0; // un pixel change : un bit de différence
        assert_eq!(hamming(dhash(&g), dhash(&g2)), 1);
        assert!(!informative(0) && !informative(u64::MAX) && informative(0xF0F0_F0F0_F0F0_F0F0));
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha256(long)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        assert_eq!(hex(&sha256(&[b'a'; 1000])), "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3");
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(pair_id("D:\\B.jpg", "D:\\a.jpg"), pair_id("D:\\a.jpg", "D:\\B.jpg"));
    }

    #[test]
    fn sha_stream() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut s = Sha256::new();
        for c in data.chunks(37) {
            s.update(c);
        }
        assert_eq!(s.finish(), sha256(&data));
        let mut s = Sha256::new();
        s.update(b"ab");
        s.update(b"c");
        assert_eq!(hex(&s.finish()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn import_rules() {
        assert!(is_motion_name("PXL_20240601_101010123.MP") && is_motion_name("x.MP~2") && !is_motion_name("x.mp4") && !is_motion_name("x.MP~"));
        assert_eq!(final_name("PXL_1.MP~2"), ("PXL_1.MP".into(), ".mp4".into()));
        assert_eq!(final_name("PXL_1.MP.jpg"), ("PXL_1.MP".into(), ".jpg".into()));
        assert_eq!(final_name("IMG_1.CR2"), ("IMG_1".into(), ".CR2".into()));
        assert_eq!(signature("IMG_1234(1).JPEG"), ("img_1234".into(), ".jpg".into()));
        assert_eq!(signature("IMG_1234-modifié.jpg"), ("img_1234".into(), ".jpg".into()));
        assert_eq!(signature("IMG_8_1.jpg"), ("img_8".into(), ".jpg".into()));
        assert_eq!(signature("IMG_8.jpg"), ("img_8".into(), ".jpg".into()));
        assert_eq!(event_name("2023-07 Vacances Bretagne"), Some("Vacances Bretagne".into()));
        assert_eq!(event_name("Rénovation"), Some("Rénovation".into()));
        assert_eq!(event_name("100CANON"), None);
        assert_eq!(event_name("Photos from 2023"), None);
        assert_eq!(event_name("DCIM"), None);
        assert_eq!(event_name("2023-07"), None);
        assert_eq!(parse_date("2023-07-14 15:30:22+02:00", 2027), Some(("2023-07".into(), Some("2023-07-14 15:30:22".into()))));
        assert_eq!(parse_date("2023-07", 2027), Some(("2023-07".into(), None)));
        assert_eq!(parse_date("0000-00-00 00:00:00", 2027), None);
        assert_eq!(parse_date("2030-01-01", 2027), None);
        assert_eq!(parse_date("été", 2027), None);
        assert_eq!(google_keys("IMG_1234.jpg", "IMG_1234.jpg.supplemental-metadata.json"), ("img_1234.jpg".into(), "img_1234.jpg".into()));
        assert_eq!(google_keys("IMG-WA0001.jpg", "IMG-WA0001.jpg(1).json"), ("img-wa0001(1).jpg".into(), "img-wa0001.jpg".into()));
        assert_eq!(google_keys("Screenshot.png", "Screenshot.png.supplemental-metad(2).json").1, "screenshot.png");
        assert!(google_variants("IMG-WA0001(1).jpg").contains(&"img-wa0001(1).jpg".to_string()));
        assert!(google_variants("Screenshot-modifié.png").contains(&"screenshot.png".to_string()));
    }

    #[test]
    #[allow(deprecated)]
    fn cache_names_unchanged() {
        use std::hash::{Hash, Hasher};
        for (p, size, mtime, max) in [
            ("D:\\4- PHOTO_range\\2024\\2024-05\\IMG_5.jpg", 4_300_000u64, 1_714_550_400u64, 400u32),
            ("E:\\été\\photo très longue nommée ainsi.JPG", 0, 0, 2560),
            ("", 1, 2, 3),
            ("abcdefgh", 7, 8, 400),
        ] {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::borrow::Cow::<str>::Borrowed(p).hash(&mut h);
            size.hash(&mut h);
            mtime.hash(&mut h);
            max.hash(&mut h);
            assert_eq!(cache_key(p, size, mtime, max), format!("{:016x}.jpg", h.finish()));
        }
    }
}
