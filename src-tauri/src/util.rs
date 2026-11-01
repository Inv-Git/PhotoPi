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

/// SHA-256 (FIPS 180-4), pour le verrou : empreintes des chemins et du mot de passe.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
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
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
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
}
