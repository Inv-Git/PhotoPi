"use strict";
// PhotoPi : interface. Le moteur (Rust) lit la photothèque, fait les miniatures
// et les aperçus ; ici on affiche la chronologie, la visionneuse et la revue des doublons.

const TAURI = window.__TAURI__;
const invoke = TAURI.core.invoke;
const fileSrc = (p) => TAURI.core.convertFileSrc(p);
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
};
const icon = (name) => {
  const s = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  const u = document.createElementNS("http://www.w3.org/2000/svg", "use");
  u.setAttribute("href", "#i-" + name);
  s.appendChild(u);
  return s;
};

const MONTHS = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
const nf = new Intl.NumberFormat("fr-FR");
const cap = (s) => s.charAt(0).toUpperCase() + s.slice(1);
const fmtMonth = (m) => `${MONTHS[+m.slice(5, 7) - 1]} ${m.slice(0, 4)}`;
const fmtDate = (d) => (d ? `${+d.slice(8, 10)} ${MONTHS[+d.slice(5, 7) - 1]} ${d.slice(0, 4)}, ${d.slice(11, 16).replace(":", " h ")}` : "");
const plural = (n, one, many) => `${nf.format(n)} ${n > 1 ? many : one}`;
/** Valeurs EXIF à la française : date en clair, virgule décimale. */
function fmtInfo(k, v) {
  if (/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(v)) return cap(fmtDate(v));
  if (k === "Réglages" || k === "Dimensions") return v.replace(/(\d)\.(\d)/g, "$1,$2");
  return v;
}
const parentDir = (p) => p.replace(/[\\/][^\\/]*$/, "");
const baseName = (p) => p.replace(/^.*[\\/]/, "");

// ---------------------------------------------------------------------------
// État
// ---------------------------------------------------------------------------

let settings = { library: "", reports: "", tile: 180 };
let lib = null;
let view = null; // { type: "year", year } | { type: "album", id } | { type: "dupes" }
let shown = []; // éléments affichés, dans l'ordre (pour la visionneuse)
const openYears = new Set();

function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => (t.hidden = true), 3200);
}

function confirmBox(title, text, okLabel = "Mettre à la corbeille", okClass = "danger", focusCancel = false) {
  return new Promise((resolve) => {
    $("modal-title").textContent = title;
    $("modal-text").textContent = text;
    $("modal-ok").textContent = okLabel;
    $("modal-ok").className = "btn " + okClass;
    $("modal").hidden = false;
    (focusCancel ? $("modal-cancel") : $("modal-ok")).focus();
    const done = (v) => {
      $("modal").hidden = true;
      $("modal-ok").onclick = $("modal-cancel").onclick = null;
      document.removeEventListener("keydown", onKey, true);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") { e.stopPropagation(); done(false); }
      if (e.key === "Enter") { e.stopPropagation(); e.preventDefault(); done(document.activeElement !== $("modal-cancel")); }
    };
    document.addEventListener("keydown", onKey, true);
    $("modal-ok").onclick = () => done(true);
    $("modal-cancel").onclick = () => done(false);
  });
}

function saveSettings() {
  invoke("set_settings", { settings }).catch(() => {});
}

// ---------------------------------------------------------------------------
// Chargement de la photothèque
// ---------------------------------------------------------------------------

function showWelcome(error) {
  view = null;
  $("sidebar").innerHTML = "";
  $("crumb").textContent = "";
  clearViews();
  const c = screen();
  const box = el("div", "empty");
  const logo = icon("logo");
  logo.classList.add("logo");
  box.append(logo, el("h1", null, error ? "Photothèque introuvable" : "Bienvenue dans PhotoPi"));
  box.append(el("p", null, error || "Choisis le dossier de ta photothèque (par exemple D:\\4- PHOTO_range) pour commencer."));
  const b = el("button", "btn primary", "Choisir le dossier de photos");
  b.onclick = chooseLibrary;
  box.append(b);
  c.append(box);
}

async function chooseLibrary() {
  const p = await invoke("pick_folder");
  if (!p) return;
  settings.library = p;
  saveSettings();
  fillSettings();
  view = null;
  loadLibrary();
}

async function loadLibrary() {
  if (!settings.library) return showWelcome();
  clearViews();
  const c = screen();
  const box = el("div", "loading");
  const inner = el("div");
  inner.append(el("div", "spinner"), el("div", null, "Lecture de la photothèque…"));
  inner.append(el("div", "hint", "La première fois, la lecture des dates peut prendre quelques minutes."));
  box.append(inner);
  c.append(box);
  try {
    lib = await invoke("scan_library");
  } catch (e) {
    lib = null;
    return showWelcome(String(e));
  }
  itemsByPath = new Map();
  for (const y of lib.years) for (const a of y.albums) for (const it of a.items) itemsByPath.set(it.path, it);
  for (const a of lib.others) for (const it of a.items) itemsByPath.set(it.path, it);
  for (const y of lib.years) for (const al of y.albums) for (const it of al.items) it.private = !!it.locked;
  for (const al of lib.others) for (const it of al.items) it.private = !!it.locked;
  countThumbs();
  startPregen();
  refreshImportInfo();
  renderSidebar();
  if (view && view.type === "dupes") return showDupes();
  if (view && view.type === "import") return showImport();
  if (view && view.type === "explore") return showExplore();
  if (view && view.type === "album" && findAlbum(view.id)) return showAlbum(view.id);
  if (view && view.type === "year" && lib.years.some((y) => y.year === view.year)) return showYear(view.year);
  if (lib.years.length) {
    openYears.add(lib.years[0].year);
    renderSidebar();
    return showYear(lib.years[0].year);
  }
  if (lib.others.length) return showAlbum(lib.others[0].id);
  c.innerHTML = "";
  const empty = el("div", "empty");
  empty.append(el("h1", null, "Aucune photo trouvée"), el("p", null, `Le dossier ${lib.root} ne contient pas de photos rangées par année.`));
  c.append(empty);
}

function findAlbum(id) {
  for (const y of lib.years) for (const a of y.albums) if (a.id === id) return a;
  return lib.others.find((a) => a.id === id) || null;
}

// Filtre de la chronologie : tout, photos (photos animées comprises), vidéos, ou photos secrètes
const MEDIA_FILTERS = {
  all: null,
  photos: (it) => it.kind !== "video",
  videos: (it) => it.kind === "video",
  secret: (it) => it.private,
};
let mediaFilter = "all";
try {
  const f = localStorage.getItem("photopi.filtre");
  if (f && f !== "secret" && f in MEDIA_FILTERS) mediaFilter = f;
} catch (_) {}
const visible = (items) => (MEDIA_FILTERS[mediaFilter] ? items.filter(MEDIA_FILTERS[mediaFilter]) : items);
const yearCount = (y) => y.albums.reduce((n, a) => n + visible(a.items).length, 0);

async function setFilter(f) {
  if (f === "secret" && !lockInfo.revealed) {
    if (!lockInfo.has_password) return toast("Aucune photo secrète pour l'instant (touche L dans la visionneuse)");
    await revealSecrets();
    if (!lockInfo.revealed) return;
  }
  mediaFilter = f;
  try {
    localStorage.setItem("photopi.filtre", f);
  } catch (_) {}
  for (const b of $("media-filter").children) b.classList.toggle("active", b.dataset.f === f);
  if (!lib) return;
  if (view && view.type === "year" && !yearCount(lib.years.find((y) => y.year === view.year) || { albums: [] })) {
    const first = lib.years.find((y) => yearCount(y));
    if (first) view = { type: "year", year: first.year };
  }
  if (view && view.type === "year") showYear(view.year);
  else if (view && view.type === "album") showAlbum(view.id);
  else if (view && view.type === "talbum") showThemeAlbum(view.id);
  else if (view && view.type === "explore") showExplore();
  else renderSidebar();
}
const albumLabel = (a) => (a.month && !a.event ? cap(fmtMonth(a.month)) : a.title);

// ---------------------------------------------------------------------------
// Barre latérale
// ---------------------------------------------------------------------------

function renderSidebar() {
  const sb = $("sidebar");
  sb.innerHTML = "";
  if (!lib) return;
  sb.append(el("h4", null, "Chronologie"));
  for (const y of lib.years) {
    if (!yearCount(y)) continue;
    const open = openYears.has(y.year);
    const b = el("button", "nav-item nav-year" + (open ? " open" : ""));
    if (view && view.type === "year" && view.year === y.year) b.classList.add("active");
    const chev = icon("down");
    chev.classList.add("chev");
    b.append(chev, el("span", "label", y.year), el("span", "count", nf.format(yearCount(y))));
    b.onclick = () => {
      if (view && view.type === "year" && view.year === y.year && open) openYears.delete(y.year);
      else openYears.add(y.year);
      showYear(y.year);
    };
    sb.append(b);
    if (open) {
      const months = el("div", "nav-months");
      for (const a of y.albums) {
        const n = visible(a.items).length;
        if (!n) continue;
        const m = el("button", "nav-item nav-month" + (a.event ? " event" : ""));
        m.title = a.event && a.month ? `${a.title}, ${fmtMonth(a.month)}` : albumLabel(a);
        m.append(el("span", "label", albumLabel(a)), el("span", "count", nf.format(n)));
        m.onclick = () => showYear(y.year, a.id);
        months.append(m);
      }
      sb.append(months);
    }
  }
  const others = lib.others.filter((a) => visible(a.items).length);
  if (others.length) {
    sb.append(el("h4", null, "Autres"));
    for (const a of others) {
      const b = el("button", "nav-item");
      if (view && view.type === "album" && view.id === a.id) b.classList.add("active");
      b.title = a.title;
      b.append(icon("folder"), el("span", "label", a.title), el("span", "count", nf.format(visible(a.items).length)));
      b.onclick = () => showAlbum(a.id);
      sb.append(b);
    }
  }
  const hA = el("h4", "with-btn");
  hA.append(el("span", null, "Albums"));
  const plus = el("button", "h4-btn");
  plus.title = "Nouvel album";
  plus.setAttribute("aria-label", "Nouvel album");
  plus.append(icon("plus"));
  plus.onclick = async () => {
    const al = await createAlbum();
    if (al) showThemeAlbum(al.id);
  };
  hA.append(plus);
  sb.append(hA);
  if (!albums.length) sb.append(el("p", "side-hint", "Crée un album avec +, puis ajoutes-y des photos depuis la sélection."));
  for (const al of [...albums].sort((a, b) => a.name.localeCompare(b.name, "fr"))) {
    const b = el("button", "nav-item");
    if (view && view.type === "talbum" && view.id === al.id) b.classList.add("active");
    b.title = al.name;
    b.append(icon("album"), el("span", "label", al.name), el("span", "count", nf.format(themeCount(al))));
    b.onclick = () => showThemeAlbum(al.id);
    sb.append(b);
  }
  sb.append(el("h4", null, "Outils"));
  const imp = el("button", "nav-item dupes");
  if (view && view.type === "import") imp.classList.add("active");
  imp.append(icon("import"), el("span", "label", "Importer des photos"));
  if (importInfo.pending) imp.append(el("span", "count", nf.format(importInfo.pending)));
  imp.onclick = showImport;
  sb.append(imp);
  const ex = el("button", "nav-item");
  if (view && view.type === "explore") ex.classList.add("active");
  ex.append(icon("folder"), el("span", "label", "Explorer un dossier"));
  ex.onclick = showExplore;
  sb.append(ex);
  const d = el("button", "nav-item dupes");
  if (view && view.type === "dupes") d.classList.add("active");
  d.append(icon("layers"), el("span", "label", "Doublons à vérifier"));
  if (lib.duplicates) d.append(el("span", "count", nf.format(lib.duplicates)));
  d.onclick = showDupes;
  sb.append(d);
}

// ---------------------------------------------------------------------------
// Miniatures : préparées en tâche de fond par le moteur et gardées sur le disque.
// Une miniature prête s'affiche directement ; sinon on la demande tout de suite
// (4 à la fois, les dernières apparues d'abord).
// ---------------------------------------------------------------------------

const queue = [];
let active = 0;
let itemsByPath = new Map(); // chemin -> élément de la photothèque
// Avancement global : photos qui ont leur miniature / photos au total (ne fait qu'avancer)
let thumbsReady = 0;
let thumbsTotal = 0;
let pregenRunning = false;

function countThumbs() {
  thumbsReady = thumbsTotal = 0;
  for (const it of itemsByPath.values()) {
    if (it.kind === "video" || it.private) continue;
    thumbsTotal++;
    if (it.thumb) thumbsReady++;
  }
}
const tilesByPath = new Map(); // chemin -> tuiles affichées
const tileItem = new WeakMap(); // tuile -> élément

/** Une miniature vient d'être prête : on la montre partout où la photo est affichée. */
function setReady(item, thumb) {
  if (!item.thumb) thumbsReady++;
  item.thumb = thumb;
  for (const t of tilesByPath.get(item.path) || []) {
    const img = t.querySelector("img.thumb");
    if (img && !img.getAttribute("src")) img.src = fileSrc(thumb);
  }
}

function pump() {
  while (active < 4 && queue.length) {
    const job = queue.pop();
    if (!job.tile.isConnected) continue;
    active++;
    job.run().finally(() => {
      active--;
      pump();
    });
  }
}

const observer = new IntersectionObserver(
  (entries) => {
    for (const e of entries) {
      if (!e.isIntersecting) continue;
      observer.unobserve(e.target);
      loadTile(e.target);
    }
  },
  { root: $("content"), rootMargin: "600px 0px" }
);

function placeholder(tile, item) {
  const ph = el("div", "ph");
  ph.append(icon(item.kind === "video" ? "film" : "image"));
  ph.append(el("span", null, item.name.split(".").pop().toUpperCase()));
  tile.prepend(ph);
}

function loadTile(tile) {
  const item = tileItem.get(tile);
  if (!item) return;
  if (item.kind === "video") {
    const v = document.createElement("video");
    v.muted = true;
    v.preload = "metadata";
    v.addEventListener("loadeddata", () => v.classList.add("loaded"), { once: true });
    v.addEventListener("error", () => placeholder(tile, item), { once: true });
    v.src = fileSrc(item.path) + "#t=0.5";
    tile.prepend(v);
    return;
  }
  const img = new Image();
  img.className = "thumb";
  img.decoding = "async";
  img.alt = "";
  img.onload = () => img.classList.add("loaded");
  tile.prepend(img);
  if (item.private) {
    if (item.dataThumb) {
      img.src = item.dataThumb;
      return;
    }
    queue.push({
      tile,
      run: () =>
        invoke("private_image", { path: item.path, max: 400 })
          .then((d) => (img.src = item.dataThumb = d))
          .catch(() => {
            img.remove();
            placeholder(tile, item);
          }),
    });
    return pump();
  }
  if (item.thumb) {
    img.src = fileSrc(item.thumb);
    return;
  }
  queue.push({
    tile,
    run: () =>
      item.thumb
        ? Promise.resolve()
        : invoke("thumb", { path: item.path })
            .then((p) => setReady(item, p))
            .catch(() => {
              img.remove();
              placeholder(tile, item);
            }),
  });
  pump();
}

function makeTile(item, idx) {
  const t = el("div", "tile");
  t.dataset.idx = idx;
  tileItem.set(t, item);
  if (!tilesByPath.has(item.path)) tilesByPath.set(item.path, []);
  tilesByPath.get(item.path).push(t);
  t.tabIndex = 0;
  t.setAttribute("role", "button");
  t.setAttribute("aria-label", item.name);
  const badges = el("div", "badges");
  if (item.kind === "video") {
    const b = el("span", "badge");
    b.append(icon("play"));
    badges.append(b);
  }
  if (item.private) {
    const b = el("span", "badge");
    b.append(icon("lock"));
    b.title = "Photo secrète";
    badges.append(b);
  }
  if (item.kind === "raw" || item.raw) badges.append(el("span", "badge", "RAW"));
  if (item.motion) {
    const b = el("span", "badge");
    b.append(icon("live"));
    b.title = "Photo animée";
    badges.append(b);
    t.addEventListener("mouseenter", () => playMotion(t, item));
    t.addEventListener("mouseleave", () => stopMotion(t));
  }
  if (badges.childElementCount) t.append(badges);
  const check = el("button", "check");
  check.setAttribute("aria-label", "Sélectionner");
  check.append(icon("check"));
  check.addEventListener("click", (e) => {
    e.stopPropagation();
    selectClick(t, { ctrlKey: true, shiftKey: e.shiftKey });
  });
  t.append(check);
  if (selected.has(item)) t.classList.add("selected");
  t.addEventListener("click", (e) => {
    if (e.ctrlKey || e.metaKey || e.shiftKey || selected.size) selectClick(t, e);
    else openViewer(+t.dataset.idx);
  });
  t.addEventListener("keydown", (e) => {
    if (e.key === "Enter") openViewer(+t.dataset.idx);
  });
  observer.observe(t);
  return t;
}

function playMotion(tile, item) {
  let v = tile.querySelector(".motion-v");
  if (!v) {
    v = document.createElement("video");
    v.className = "motion-v";
    v.muted = true;
    v.loop = true;
    v.playsInline = true;
    v.src = fileSrc(item.motion);
    v.addEventListener("playing", () => v.classList.add("playing"));
    tile.insertBefore(v, tile.querySelector(".badges"));
  }
  v.play().catch(() => {});
}

function stopMotion(tile) {
  const v = tile.querySelector(".motion-v");
  if (!v) return;
  v.pause();
  v.classList.remove("playing");
  v.currentTime = 0;
}

// ---------------------------------------------------------------------------
// Préparation des miniatures en tâche de fond
// ---------------------------------------------------------------------------

/** Lance (ou relance) la préparation des miniatures manquantes ; `first` passe devant. */
function startPregen(first = []) {
  if (!lib) return;
  const seen = new Set();
  const paths = [];
  const add = (it) => {
    if (it.kind === "video" || it.private || it.explore || it.thumb || seen.has(it.path)) return;
    seen.add(it.path);
    paths.push(it.path);
  };
  first.forEach(add);
  for (const y of lib.years) for (const a of y.albums) a.items.forEach(add);
  for (const a of lib.others) a.items.forEach(add);
  pregenRunning = paths.length > 0;
  showPregen();
  invoke("pregen_thumbs", { paths }).catch(() => {});
}

/** La vue affichée passe en priorité (si des miniatures y manquent). */
function prioritize(items) {
  clearTimeout(prioritize.timer);
  if (!items.some((it) => it.kind !== "video" && !it.private && !it.explore && !it.thumb)) return;
  prioritize.timer = setTimeout(() => startPregen(items), 300);
}

function showPregen() {
  const box = $("pregen");
  if (!pregenRunning || !thumbsTotal || thumbsReady >= thumbsTotal) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  $("pregen-text").textContent = `Miniatures : ${nf.format(thumbsReady)} / ${nf.format(thumbsTotal)}`;
  $("pregen-bar").style.width = ((thumbsReady / thumbsTotal) * 100).toFixed(1) + "%";
}

TAURI.event.listen("thumbs", (e) => {
  const { done, total, items } = e.payload;
  for (const [path, thumb] of items) {
    const it = itemsByPath.get(path);
    if (it) setReady(it, thumb);
  }
  if (done >= total) pregenRunning = false;
  showPregen();
});

// ---------------------------------------------------------------------------
// Vues : année (mois en sections) et album
// ---------------------------------------------------------------------------

function sectionFor(a, startIdx, lazy = false) {
  const items = visible(a.items);
  const s = el("section", "section");
  s.dataset.album = a.id;
  const head = el("div", "section-head");
  if (a.event) {
    head.append(el("h2", null, a.title), el("span", "tag", "Événement"));
    if (a.month) head.append(el("span", "sub", cap(fmtMonth(a.month))));
  } else {
    head.append(el("h2", null, albumLabel(a)));
  }
  head.append(el("span", "sub", plural(items.length, "élément", "éléments")));
  const lockBtn = el("button", "head-btn lock-album");
  lockBtn.append(icon("lock"), el("span", null, "Rendre secret"));
  lockBtn.title = "Rendre secrètes toutes les photos de cet album";
  lockBtn.onclick = () => lockAlbum(a, items);
  const selBtn = el("button", "head-btn");
  selBtn.append(icon("check"), el("span", null, "Tout sélectionner"));
  selBtn.onclick = () => selectItems(items);
  head.append(selBtn, lockBtn);
  s.append(head);
  const g = el("div", "grid");
  g.dataset.count = items.length;
  if (lazy) {
    g.classList.add("pending");
    g.style.minHeight = estHeight(items.length) + "px";
    g._fill = () => {
      g.classList.remove("pending");
      g.style.minHeight = "";
      items.forEach((it, i) => g.append(makeTile(it, startIdx + i)));
      prioritize(items);
    };
    sectionObserver.observe(g);
  } else {
    items.forEach((it, i) => g.append(makeTile(it, startIdx + i)));
  }
  s.append(g);
  return s;
}

/** Hauteur estimée d'une grille pas encore construite (même calcul que la grille CSS). */
function estHeight(n) {
  const c = $("content");
  const cs = getComputedStyle(c);
  const w = c.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const gap = 4;
  const cols = Math.max(1, Math.floor((w + gap) / (settings.tile + gap)));
  const side = (w - gap * (cols - 1)) / cols;
  const rows = Math.ceil(n / cols);
  return Math.round(rows * side + (rows - 1) * gap);
}

const sectionObserver = new IntersectionObserver(
  (entries) => {
    for (const e of entries) {
      if (!e.isIntersecting || !e.target._fill) continue;
      sectionObserver.unobserve(e.target);
      const fill = e.target._fill;
      e.target._fill = null;
      fill();
    }
  },
  { root: $("content"), rootMargin: "1500px 0px" }
);

/** Recalcule la hauteur des mois pas encore construits (taille des miniatures, fenêtre). */
function reestimate() {
  for (const g of document.querySelectorAll(".grid.pending")) {
    const n = +g.dataset.count || 0;
    g.style.minHeight = estHeight(n) + "px";
  }
  if (current && current.startsWith("c|")) layoutScrubber();
}

// Les dernières vues visitées restent construites (simplement masquées) :
// revenir sur une année est instantané, sans rien recharger.
const views = new Map(); // clé -> { div, shown, scroll }, de la plus ancienne à la plus récente
const MAX_VIEWS = 8;
let current = null;

function hideAll() {
  const c = $("content");
  c.classList.remove("chrono");
  $("scrubber").hidden = true;
  if (current && views.has(current)) views.get(current).scroll = c.scrollTop;
  for (const v of views.values()) v.div.hidden = true;
  $("screen").hidden = true;
}

/** Écran simple (accueil, chargement, doublons) à la place des vues. */
function screen() {
  clearSelection();
  hideAll();
  current = null;
  const s = $("screen");
  s.innerHTML = "";
  s.hidden = false;
  $("content").scrollTop = 0;
  return s;
}

function activate(key, build) {
  if (key !== current) clearSelection();
  hideAll();
  let v = views.get(key);
  if (v) {
    views.delete(key); // remise en fin de liste (la plus récente)
  } else {
    const div = el("div", "view");
    const list = [];
    build(div, list);
    $("content").append(div);
    v = { div, shown: list, scroll: 0 };
  }
  views.set(key, v);
  while (views.size > MAX_VIEWS) {
    const [oldKey, old] = views.entries().next().value;
    old.div.remove();
    views.delete(oldKey);
  }
  v.div.hidden = false;
  current = key;
  shown = v.shown;
  prioritize(shown);
  return v;
}

/** Oublie toutes les vues construites (nouvelle lecture, fichiers supprimés). */
function clearViews() {
  for (const v of views.values()) v.div.remove();
  views.clear();
  tilesByPath.clear();
  queue.length = 0;
  observer.disconnect();
  sectionObserver.disconnect();
  current = null;
}

// Chronologie continue : toutes les années à la suite, les mois construits à l'approche de l'écran.
function showYear(year, albumId) {
  if (!lib.years.some((x) => x.year === year)) return;
  view = { type: "year", year };
  openYears.add(year);
  renderSidebar();
  $("crumb").textContent = year;
  const v = activate("c|" + mediaFilter, (div, list) => {
    for (const y of lib.years) {
      const n = yearCount(y);
      if (!n) continue;
      const head = el("div", "year-head chrono-year");
      head.dataset.year = y.year;
      head.append(el("h1", null, y.year), el("span", null, plural(n, "élément", "éléments")));
      div.append(head);
      for (const a of y.albums) {
        const items = visible(a.items);
        if (!items.length) continue;
        const s = sectionFor(a, list.length, true);
        s.dataset.year = y.year;
        div.append(s);
        list.push(...items);
      }
    }
    if (!list.length) div.append(el("p", "empty-filter", "Rien à afficher pour ce filtre."));
  });
  $("content").classList.add("chrono");
  $("scrubber").hidden = false;
  requestAnimationFrame(layoutScrubber);
  const target = albumId
    ? v.div.querySelector(`section[data-album="${CSS.escape(albumId)}"]`)
    : v.div.querySelector(`.year-head[data-year="${CSS.escape(year)}"]`);
  if (target) scrollToEl(target);
}

/** Va à un repère ; les mois construits en route peuvent décaler un peu : on rectifie. */
function scrollToEl(target) {
  const c = $("content");
  const go = () => (c.scrollTop = Math.max(0, target.offsetTop - 8));
  go();
  requestAnimationFrame(() => requestAnimationFrame(go));
  setTimeout(go, 120);
}

// --- réglette des années (bord droit) --------------------------------------------

function chronoView() {
  return current && current.startsWith("c|") ? views.get(current) : null;
}

function layoutScrubber() {
  const v = chronoView();
  const box = $("scrub-years");
  box.innerHTML = "";
  if (!v) return;
  const c = $("content");
  const total = Math.max(1, c.scrollHeight - c.clientHeight);
  const h = box.clientHeight;
  let last = -100;
  for (const head of v.div.querySelectorAll(".year-head[data-year]")) {
    const y = Math.min(1, (head.offsetTop - 8) / total) * h;
    if (y - last < 16) continue;
    last = y;
    const s = el("span", null, head.dataset.year);
    s.style.top = y + "px";
    s.dataset.year = head.dataset.year;
    box.append(s);
  }
  syncScrubber();
}

/** Année et mois à une position de défilement donnée. */
function positionInfo(top) {
  const v = chronoView();
  if (!v) return null;
  let year = null;
  let month = null;
  for (const s of v.div.children) {
    if (s.offsetTop > top + 60) break;
    if (s.dataset.year) year = s.dataset.year;
    if (s.tagName === "SECTION") month = s.querySelector("h2") ? s.querySelector(".section-head").firstChild.textContent : null;
  }
  return { year, month };
}

function syncScrubber() {
  const v = chronoView();
  if (!v) return;
  const c = $("content");
  const box = $("scrub-years");
  const frac = c.scrollTop / Math.max(1, c.scrollHeight - c.clientHeight);
  $("scrub-mark").style.top = 10 + frac * box.clientHeight + "px";
  const info = positionInfo(c.scrollTop);
  if (info && info.year && view && view.type === "year" && view.year !== info.year) {
    view.year = info.year;
    $("crumb").textContent = info.year;
    for (const b of $("sidebar").querySelectorAll(".nav-year")) b.classList.toggle("active", b.querySelector(".label").textContent === info.year);
  }
  for (const s of box.children) s.classList.toggle("on", info && s.dataset.year === info.year);
  return info;
}

function initScrubber() {
  const c = $("content");
  const bar = $("scrubber");
  const bubble = $("scrub-bubble");
  let raf = 0;
  c.addEventListener("scroll", () => {
    if (!chronoView() || raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      syncScrubber();
    });
  });
  const seek = (clientY) => {
    const box = $("scrub-years").getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientY - box.top) / box.height));
    c.scrollTop = frac * (c.scrollHeight - c.clientHeight);
    const info = syncScrubber();
    bubble.textContent = info ? info.month || info.year || "" : "";
    bubble.style.top = clientY - bar.getBoundingClientRect().top + "px";
    bubble.hidden = !bubble.textContent;
  };
  let dragging = false;
  bar.addEventListener("mousedown", (e) => {
    dragging = true;
    bar.classList.add("drag");
    seek(e.clientY);
    e.preventDefault();
  });
  window.addEventListener("mousemove", (e) => dragging && seek(e.clientY));
  window.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    bar.classList.remove("drag");
    bubble.hidden = true;
  });
  bar.addEventListener("wheel", (e) => (c.scrollTop += e.deltaY), { passive: true });
  let rt = 0;
  window.addEventListener("resize", () => {
    clearTimeout(rt);
    rt = setTimeout(reestimate, 150);
  });
}

function showAlbum(id) {
  const a = findAlbum(id);
  if (!a) return;
  view = { type: "album", id };
  renderSidebar();
  $("crumb").textContent = a.title;
  const v = activate("a:" + id + "|" + mediaFilter, (div, list) => {
    list.push(...visible(a.items));
    div.append(sectionFor(a, 0));
  });
  $("content").scrollTop = v.scroll;
}

/** Reconstruit la vue affichée (après une suppression), à la même position. */
function rerender() {
  const scroll = $("content").scrollTop;
  clearViews();
  if (!view) return;
  if (view.type === "year") showYear(view.year);
  else if (view.type === "album") showAlbum(view.id);
  else if (view.type === "talbum") showThemeAlbum(view.id);
  else if (view.type === "explore") showExplore();
  $("content").scrollTop = scroll;
}

/** Retire des fichiers de la photothèque en mémoire (après une mise à la corbeille). */
function forget(paths) {
  const gone = new Set(paths);
  const keep = (it) => !gone.has(it.path);
  for (const y of lib.years) {
    for (const a of y.albums) a.items = a.items.filter(keep);
    y.albums = y.albums.filter((a) => a.items.length);
  }
  lib.years = lib.years.filter((y) => y.albums.length);
  for (const a of lib.others) a.items = a.items.filter(keep);
  lib.others = lib.others.filter((a) => a.items.length);
  if (view && view.type === "year" && !lib.years.some((y) => y.year === view.year)) {
    view = lib.years.length ? { type: "year", year: lib.years[0].year } : null;
  }
}

// ---------------------------------------------------------------------------
// Albums thématiques : des listes de photos enregistrées dans PhotoPi.
// Une photo peut être dans plusieurs albums ; les fichiers ne bougent pas.
// ---------------------------------------------------------------------------

let albums = []; // [{ id, name, paths: [chemin, …] }]
const albumById = (id) => albums.find((a) => a.id === id);

function saveAlbums() {
  invoke("albums_set", { albums }).catch((e) => toast("Albums non enregistrés : " + e));
}

/** Photos d'un album présentes dans la photothèque (les secrètes masquées n'y sont pas). */
function themeItems(al) {
  const items = al.paths.map((p) => itemsByPath.get(p)).filter(Boolean);
  return items.sort((a, b) => (a.date || "").localeCompare(b.date || "") || a.name.localeCompare(b.name));
}

function themeCount(al) {
  const f = MEDIA_FILTERS[mediaFilter];
  let n = 0;
  for (const p of al.paths) {
    const it = itemsByPath.get(p);
    if (it && (!f || f(it))) n++;
  }
  return n;
}

/** Oublie les vues construites d'un album (ou de tous). */
function dropThemeViews(id) {
  const prefix = "t:" + (id ? id + "|" : "");
  for (const [k, v] of [...views]) {
    if (k.startsWith(prefix) && k !== current) {
      v.div.remove();
      views.delete(k);
    }
  }
}

async function askAlbumName(title, value = "", okLabel = "Créer") {
  const v = await askPassword({
    title,
    fields: [{ label: "Nom de l'album", type: "text", value }],
    okLabel,
    check: ([n]) => (n.trim() ? null : "Donne un nom à l'album."),
  });
  return v ? v[0].trim() : null;
}

async function createAlbum() {
  const name = await askAlbumName("Nouvel album");
  if (!name) return null;
  const al = { id: "al" + Date.now().toString(36), name, paths: [] };
  albums.push(al);
  saveAlbums();
  renderSidebar();
  return al;
}

/** Fait choisir un album (ou en créer un). Renvoie l'album, ou null. */
function pickAlbum() {
  return new Promise((resolve) => {
    const list = $("pick-list");
    list.innerHTML = "";
    const done = (al) => {
      $("pick-modal").hidden = true;
      document.removeEventListener("keydown", onKey, true);
      resolve(al);
    };
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        done(null);
      }
    };
    for (const al of [...albums].sort((a, b) => a.name.localeCompare(b.name, "fr"))) {
      const b = el("button");
      b.append(icon("album"), el("span", null, al.name), el("span", "n", nf.format(al.paths.length)));
      b.onclick = () => done(al);
      list.append(b);
    }
    const nw = el("button", "new");
    nw.append(icon("plus"), el("span", null, "Nouvel album…"));
    nw.onclick = async () => {
      $("pick-modal").hidden = true;
      document.removeEventListener("keydown", onKey, true);
      resolve(await createAlbum());
    };
    list.append(nw);
    $("pick-cancel").onclick = () => done(null);
    document.addEventListener("keydown", onKey, true);
    $("pick-modal").hidden = false;
    list.firstChild.focus();
  });
}

async function addToAlbum(items) {
  if (items.some((it) => it.explore)) return;
  if (!items.length) return;
  const al = await pickAlbum();
  if (!al) return;
  const have = new Set(al.paths);
  let n = 0;
  for (const it of items) {
    if (have.has(it.path)) continue;
    al.paths.push(it.path);
    have.add(it.path);
    n++;
  }
  saveAlbums();
  dropThemeViews(al.id);
  if (view && view.type !== "talbum") clearSelection();
  renderSidebar();
  toast(n ? `${plural(n, "élément ajouté", "éléments ajoutés")} à « ${al.name} »` : `Déjà dans « ${al.name} »`);
}

function removeFromAlbum(items) {
  const al = view && view.type === "talbum" && albumById(view.id);
  if (!al || !items.length) return;
  const gone = new Set(items.map((it) => it.path));
  al.paths = al.paths.filter((p) => !gone.has(p));
  saveAlbums();
  clearSelection();
  rerender();
  toast(`${plural(gone.size, "élément retiré", "éléments retirés")} de « ${al.name} » (les fichiers restent dans la photothèque)`);
}

/** Une photo mise à la corbeille sort de tous ses albums. */
function removeFromAlbums(paths) {
  const gone = new Set(paths);
  let changed = false;
  for (const al of albums) {
    const n = al.paths.length;
    al.paths = al.paths.filter((p) => !gone.has(p));
    if (al.paths.length !== n) changed = true;
  }
  if (changed) {
    saveAlbums();
    dropThemeViews();
  }
}

async function renameAlbum(al) {
  const name = await askAlbumName("Renommer l'album", al.name, "Renommer");
  if (!name || name === al.name) return;
  al.name = name;
  saveAlbums();
  rerender();
}

async function deleteAlbum(al) {
  const ok = await confirmBox(
    `Supprimer l'album « ${al.name} » ?`,
    "Seul l'album est supprimé : ses photos restent dans la photothèque, à leur place.",
    "Supprimer l'album",
    "danger",
    true
  );
  if (!ok) return;
  albums = albums.filter((a) => a !== al);
  saveAlbums();
  const y = lib.years.find((x) => yearCount(x));
  view = y ? { type: "year", year: y.year } : null;
  rerender();
  renderSidebar();
  toast("Album supprimé");
}

function showThemeAlbum(id) {
  const al = albumById(id);
  if (!al || !lib) return;
  view = { type: "talbum", id };
  renderSidebar();
  $("crumb").textContent = al.name;
  const v = activate("t:" + id + "|" + mediaFilter, (div, list) => {
    const items = visible(themeItems(al));
    const head = el("div", "year-head");
    head.append(el("h1", null, al.name), el("span", null, plural(items.length, "élément", "éléments")));
    const ren = el("button", "btn", "Renommer");
    ren.onclick = () => renameAlbum(al);
    const del = el("button", "btn", "Supprimer l'album");
    del.onclick = () => deleteAlbum(al);
    head.append(ren, del);
    div.append(head);
    if (!items.length) {
      div.append(el("p", "empty-filter", al.paths.length ? "Rien à afficher pour ce filtre." : "Album vide : sélectionne des photos, puis « Ajouter à un album » dans la barre du bas."));
      return;
    }
    list.push(...items);
    const g = el("div", "grid");
    g.style.marginTop = "14px";
    items.forEach((it, i) => g.append(makeTile(it, i)));
    div.append(g);
  });
  $("content").scrollTop = v.scroll;
}

// ---------------------------------------------------------------------------
// Sélection multiple
// ---------------------------------------------------------------------------

const selected = new Set(); // éléments sélectionnés (dans la vue affichée)
let anchor = -1; // dernier élément cliqué, pour Maj + clic

function selectClick(tile, e) {
  const idx = +tile.dataset.idx;
  const item = shown[idx];
  if (!item) return;
  if (e.shiftKey && anchor >= 0 && selected.size) {
    const [a, b] = anchor < idx ? [anchor, idx] : [idx, anchor];
    for (let i = a; i <= b; i++) selected.add(shown[i]);
  } else if (selected.has(item)) {
    selected.delete(item);
  } else {
    selected.add(item);
  }
  anchor = selected.size ? idx : -1;
  syncSelection();
}

function selectItems(items) {
  const all = items.every((it) => selected.has(it));
  for (const it of items) all ? selected.delete(it) : selected.add(it);
  syncSelection();
}

function clearSelection() {
  if (!selected.size && anchor < 0) return;
  selected.clear();
  anchor = -1;
  syncSelection();
}

/** Met à jour les vignettes et la barre d'actions. */
function syncSelection() {
  const v = views.get(current);
  if (v) {
    v.div.classList.toggle("selecting", selected.size > 0);
    for (const t of v.div.querySelectorAll(".tile")) t.classList.toggle("selected", selected.has(tileItem.get(t)));
  }
  const bar = $("selbar");
  if (!selected.size) {
    bar.hidden = true;
    return;
  }
  const items = [...selected];
  const priv = items.filter((it) => it.private).length;
  $("sel-count").textContent = plural(items.length, "élément sélectionné", "éléments sélectionnés");
  $("sel-lock").hidden = priv === items.length;
  $("sel-unlock").hidden = !priv;
  $("sel-unalbum").hidden = !(view && view.type === "talbum");
  // explorateur : seulement l'ajout à la photothèque (copie), rien d'autre
  const exploring = !!(view && view.type === "explore");
  $("sel-import").hidden = !exploring;
  if (exploring) for (const id of ["sel-date", "sel-album", "sel-unalbum", "sel-lock", "sel-unlock", "sel-trash"]) $(id).hidden = true;
  else $("sel-date").hidden = $("sel-album").hidden = $("sel-trash").hidden = false;
  bar.hidden = false;
}

const filesOf = (items) => items.flatMap((it) => [it.path, it.raw, it.motion].filter(Boolean));

async function trashSelection() {
  if (view && view.type === "explore") return;
  const items = [...selected];
  if (!items.length) return;
  const files = filesOf(items);
  const extra = files.length > items.length ? ` (avec leurs RAW et vidéos associés : ${nf.format(files.length)} fichiers)` : "";
  const ok = await confirmBox(
    `Mettre ${plural(items.length, "élément", "éléments")} à la corbeille ?`,
    `${plural(items.length, "élément sélectionné", "éléments sélectionnés")}${extra}.\n\nTu pourras les récupérer dans la corbeille de Windows.`
  );
  if (!ok) return;
  const done = new Set();
  try {
    for (let i = 0; i < items.length; i += 50) {
      const chunk = items.slice(i, i + 50);
      await invoke("trash_files", { paths: filesOf(chunk) });
      chunk.forEach((it) => done.add(it));
    }
  } catch (e) {
    toast("Échec : " + e);
  }
  const gone = [...done];
  lib.locked = Math.max(0, lib.locked - gone.filter((it) => it.private).length);
  forget(gone.map((it) => it.path));
  removeFromAlbums(gone.map((it) => it.path));
  for (const it of gone) itemsByPath.delete(it.path);
  countThumbs();
  clearSelection();
  rerender();
  renderSidebar();
  if (gone.length) toast(`${plural(gone.length, "élément envoyé", "éléments envoyés")} à la corbeille`);
}

async function lockSelection() {
  if (view && view.type === "explore") return;
  const items = [...selected].filter((it) => !it.private);
  if (!items.length) return;
  if (!(await lockItems(items))) return;
  clearSelection();
  rerender();
  renderSidebar();
  toast(`${plural(items.length, "élément rendu secret", "éléments rendus secrets")}`);
}

async function unlockSelection() {
  const items = [...selected].filter((it) => it.private);
  if (!items.length || !privatePw) return;
  const files = filesOf(items);
  try {
    await invoke("unlock_items", { password: privatePw, paths: files });
  } catch (e) {
    return toast(String(e));
  }
  for (const it of items) {
    it.private = it.locked = false;
    delete it.dataThumb;
  }
  lib.locked = Math.max(0, lib.locked - items.length);
  lockInfo.count = Math.max(0, lockInfo.count - files.length);
  countThumbs();
  fillSettings();
  clearSelection();
  rerender();
  toast(`${plural(items.length, "élément retiré", "éléments retirés")} des photos secrètes`);
}

// ---------------------------------------------------------------------------
// Visionneuse
// ---------------------------------------------------------------------------

let vIdx = -1;
let vToken = 0;
const zoom = { s: 1, x: 0, y: 0 };
let infoOpen = false;

function openViewer(i) {
  vIdx = i;
  $("viewer").hidden = false;
  showCurrent();
}

function closeViewer() {
  $("viewer").hidden = true;
  for (const v of [$("v-video"), $("v-motion")]) {
    v.pause();
    v.removeAttribute("src");
    v.load();
  }
  const cur = views.get(current);
  const tile = cur && cur.div.querySelector(`.tile[data-idx="${vIdx}"]`);
  if (tile) {
    tile.scrollIntoView({ block: "nearest" });
    tile.focus({ preventScroll: true });
  }
}

function applyZoom(showLabel) {
  const img = $("v-img");
  img.style.transform = zoom.s === 1 ? "" : `translate(${zoom.x}px, ${zoom.y}px) scale(${zoom.s})`;
  $("v-stage").classList.toggle("zoomed", zoom.s > 1);
  if (showLabel && img.naturalWidth) {
    const pct = Math.round(((zoom.s * img.clientWidth) / img.naturalWidth) * 100);
    const z = $("v-zoom");
    z.textContent = `${pct} %`;
    z.hidden = false;
    clearTimeout(applyZoom.timer);
    applyZoom.timer = setTimeout(() => (z.hidden = true), 900);
  }
}

function resetZoom() {
  zoom.s = 1;
  zoom.x = zoom.y = 0;
  applyZoom(false);
}

/** Zoome à l'échelle s autour du point (clientX, clientY). */
function zoomAt(s, cx, cy) {
  const r = $("v-stage").getBoundingClientRect();
  const px = cx - (r.left + r.width / 2);
  const py = cy - (r.top + r.height / 2);
  s = Math.min(Math.max(s, 1), 16);
  zoom.x = px - (s * (px - zoom.x)) / zoom.s;
  zoom.y = py - (s * (py - zoom.y)) / zoom.s;
  zoom.s = s;
  if (s === 1) zoom.x = zoom.y = 0;
  applyZoom(true);
}

function stageMsg(content) {
  const m = $("v-msg");
  m.innerHTML = "";
  if (content == null) {
    m.hidden = true;
    return;
  }
  if (content === "loading") m.append(el("div", "spinner"));
  else m.append(el("div", null, content));
  m.hidden = false;
}

async function showCurrent() {
  const it = shown[vIdx];
  if (!it) return closeViewer();
  const token = ++vToken;
  const img = $("v-img");
  const video = $("v-video");
  const motion = $("v-motion");
  resetZoom();
  video.pause();
  motion.pause();
  motion.hidden = true;
  video.hidden = true;
  img.hidden = true;
  img.removeAttribute("src");
  $("v-date").textContent = it.date ? cap(fmtDate(it.date)) : "Date inconnue";
  const extra = [it.raw ? "+ RAW" : "", it.motion ? "photo animée" : ""].filter(Boolean).join(", ");
  $("v-name").textContent = `${it.name}${extra ? " (" + extra + ")" : ""} · ${vIdx + 1} / ${nf.format(shown.length)}`;
  $("v-prev").disabled = vIdx <= 0;
  $("v-next").disabled = vIdx >= shown.length - 1;
  $("v-live").hidden = !it.motion;
  $("v-live").classList.remove("active");
  if (infoOpen) loadInfo(it);

  if (it.kind === "video") {
    stageMsg(null);
    video.hidden = false;
    video.src = fileSrc(it.path);
    video.play().catch(() => {});
    return;
  }
  for (const id of ["v-lock", "v-album", "v-trash"]) $(id).hidden = !!it.explore;
  $("v-lock").querySelector("use").setAttribute("href", it.private ? "#i-unlock" : "#i-lock");
  $("v-lock").title = it.private ? "Retirer des photos secrètes (L)" : "Rendre secrète (L)";
  stageMsg("loading");
  try {
    const p = it.private && !displayable(it.path) ? await invoke("private_image", { path: it.path, max: 2560 }) : await invoke("display", { path: it.path });
    if (token !== vToken) return;
    img.onload = () => {
      if (token !== vToken) return;
      stageMsg(null);
      img.hidden = false;
    };
    img.onerror = () => token === vToken && stageMsg("Impossible d'afficher cette image.");
    img.src = p.startsWith("data:") ? p : fileSrc(p);
  } catch (e) {
    if (token === vToken) stageMsg(String(e));
  }
  preload(vIdx + 1);
  preload(vIdx - 1);
}

const displayable = (p) => /\.(jpe?g|png|gif|webp|bmp)$/i.test(p);

function preload(i) {
  const it = shown[i];
  if (!it || it.kind === "video" || (it.private && !displayable(it.path))) return;
  invoke("display", { path: it.path })
    .then((p) => {
      new Image().src = fileSrc(p);
    })
    .catch(() => {});
}

function step(d) {
  const n = vIdx + d;
  if (n < 0 || n >= shown.length) return;
  vIdx = n;
  showCurrent();
}

function toggleMotion() {
  const it = shown[vIdx];
  if (!it || !it.motion) return;
  const motion = $("v-motion");
  const img = $("v-img");
  if (!motion.hidden) {
    motion.pause();
    motion.hidden = true;
    img.hidden = false;
    $("v-live").classList.remove("active");
    return;
  }
  resetZoom();
  motion.src = fileSrc(it.motion);
  motion.onended = () => {
    motion.hidden = true;
    img.hidden = false;
    $("v-live").classList.remove("active");
  };
  img.hidden = true;
  motion.hidden = false;
  $("v-live").classList.add("active");
  motion.play().catch(() => {});
}

async function loadInfo(it) {
  const box = $("v-info");
  box.innerHTML = "";
  const dl = el("dl");
  const add = (k, v) => {
    if (!v) return;
    dl.append(el("dt", null, k), el("dd", null, v));
  };
  add("Fichier", it.name);
  if (it.raw) add("RAW associé", baseName(it.raw));
  if (it.motion) add("Photo animée", baseName(it.motion));
  box.append(dl);
  try {
    const rows = await invoke("photo_info", { path: it.path });
    if (shown[vIdx] !== it) return;
    for (const [k, v] of rows) add(k, fmtInfo(k, v));
    if (it.raw && !rows.some(([k]) => k === "Appareil")) {
      const rawRows = await invoke("photo_info", { path: it.raw });
      for (const [k, v] of rawRows) if (k === "Appareil" || k === "Réglages" || k === "Objectif") add(k, fmtInfo(k, v));
    }
  } catch (_) {}
  add("Dossier", parentDir(it.path));
}

function toggleInfo() {
  infoOpen = !infoOpen;
  $("v-info").hidden = !infoOpen;
  $("viewer").classList.toggle("info-open", infoOpen);
  $("v-info-btn").classList.toggle("active", infoOpen);
  if (infoOpen && shown[vIdx]) loadInfo(shown[vIdx]);
  resetZoom();
}

/** Après qu'une photo a quitté la vue (corbeille, verrou) : on passe à la suivante. */
function afterRemoval(message) {
  const idx = vIdx;
  rerender();
  renderSidebar();
  toast(message);
  if (!shown.length) return closeViewer();
  vIdx = Math.min(idx, shown.length - 1);
  showCurrent();
}

async function trashCurrent() {
  const it = shown[vIdx];
  if (!it || it.explore) return;
  const files = [it.path, it.raw, it.motion].filter(Boolean);
  const list = files.map(baseName).join("\n");
  const ok = await confirmBox(
    files.length > 1 ? `Mettre ces ${files.length} fichiers à la corbeille ?` : "Mettre cette photo à la corbeille ?",
    `${list}\n\nTu pourras la récupérer dans la corbeille de Windows.`
  );
  if (!ok) return;
  try {
    await invoke("trash_files", { paths: files });
  } catch (e) {
    return toast("Échec : " + e);
  }
  if (it.private) lib.locked = Math.max(0, lib.locked - 1);
  forget([it.path]);
  removeFromAlbums([it.path]);
  afterRemoval("Envoyé à la corbeille");
}

function initViewer() {
  const stage = $("v-stage");
  const img = $("v-img");
  $("v-close").onclick = closeViewer;
  $("v-prev").onclick = () => step(-1);
  $("v-next").onclick = () => step(1);
  $("v-info-btn").onclick = toggleInfo;
  $("v-trash").onclick = trashCurrent;
  $("v-live").onclick = toggleMotion;
  $("v-lock").onclick = toggleLock;
  $("v-album").onclick = () => shown[vIdx] && addToAlbum([shown[vIdx]]);
  $("v-folder").onclick = () => shown[vIdx] && invoke("show_in_folder", { path: shown[vIdx].path });

  stage.addEventListener(
    "wheel",
    (e) => {
      if (img.hidden) return;
      e.preventDefault();
      zoomAt(zoom.s * (e.deltaY < 0 ? 1.2 : 1 / 1.2), e.clientX, e.clientY);
    },
    { passive: false }
  );
  stage.addEventListener("dblclick", (e) => {
    if (img.hidden || !img.naturalWidth) return;
    if (zoom.s > 1) return resetZoom();
    const full = img.naturalWidth / img.clientWidth;
    zoomAt(full > 1.05 ? full : 2.5, e.clientX, e.clientY);
  });
  let drag = null;
  stage.addEventListener("mousedown", (e) => {
    if (zoom.s <= 1 || e.button !== 0) return;
    drag = { x: e.clientX - zoom.x, y: e.clientY - zoom.y };
    stage.classList.add("dragging");
    e.preventDefault();
  });
  window.addEventListener("mousemove", (e) => {
    if (!drag) return;
    zoom.x = e.clientX - drag.x;
    zoom.y = e.clientY - drag.y;
    applyZoom(false);
  });
  window.addEventListener("mouseup", () => {
    drag = null;
    stage.classList.remove("dragging");
  });
}

// ---------------------------------------------------------------------------
// Revue des doublons
// ---------------------------------------------------------------------------

let dupes = [];
let dFilter = "all";
let dIdx = 0;
let dToken = 0;

const DUP_FILTERS = [
  ["all", "Tous"],
  ["doublon", "Identiques"],
  ["quasi-doublon", "Recompressés"],
  ["inconnu", "Sans correspondance"],
];
const KIND_LABEL = { doublon: "Identique", "quasi-doublon": "Recompressé", inconnu: "Sans correspondance" };

const dList = () => (dFilter === "all" ? dupes : dupes.filter((d) => d.kind === dFilter));

async function showDupes() {
  view = { type: "dupes" };
  renderSidebar();
  $("crumb").textContent = "Doublons à vérifier";
  const c = screen();
  shown = [];
  const box = el("div", "loading");
  const inner = el("div");
  inner.append(el("div", "spinner"), el("div", null, "Lecture des rapports de tri…"));
  box.append(inner);
  c.append(box);
  try {
    dupes = await invoke("duplicates");
  } catch (e) {
    dupes = [];
    toast(String(e));
  }
  if (view.type !== "dupes") return;
  dIdx = 0;
  renderDupes();
}

function isVideo(p) {
  return /\.(mp4|mov|m4v|avi|mkv|mts|m2ts|3gp|wmv|mpg|mpeg|webm)$/i.test(p);
}

/** Image ou vidéo d'un panneau ; `alive()` dit si le panneau est toujours affiché. */
function paneMedia(path, alive) {
  const box = el("div", "pane-media");
  if (isVideo(path)) {
    const v = document.createElement("video");
    v.controls = true;
    v.muted = true;
    v.preload = "metadata";
    v.src = fileSrc(path);
    box.append(v);
    return box;
  }
  const sp = el("div", "spinner");
  box.append(sp);
  invoke("display", { path })
    .then((p) => {
      if (!alive()) return;
      const img = new Image();
      img.alt = "";
      img.onload = () => sp.remove();
      img.src = fileSrc(p);
      box.append(img);
    })
    .catch((e) => {
      sp.remove();
      box.append(el("div", "missing", String(e)));
    });
  return box;
}

function pane(kind, path, size) {
  const p = el("div", "pane " + kind);
  const head = el("div", "pane-head");
  head.append(el("span", null, kind === "dup" ? "Mis de côté" : "Conservé"));
  head.append(el("span", "pill", kind === "dup" ? "sera supprimé" : "reste dans la photothèque"));
  p.append(head);
  if (!path) {
    const m = el("div", "pane-media");
    const miss = el("div", "missing");
    miss.append(icon("alert"), el("div", null, "Exemplaire conservé introuvable : il n'apparaît dans aucun rapport de tri, ou il a été déplacé."));
    m.append(miss);
    p.append(m, el("dl", "pane-meta"));
    return p;
  }
  const t = dToken;
  p.append(paneMedia(path, () => t === dToken));
  const meta = el("dl", "pane-meta");
  meta.dataset.path = path;
  meta.append(el("dt", null, "Fichier"), el("dd", null, baseName(path)));
  meta.lastChild.title = path;
  meta.append(el("dt", null, "Dossier"), el("dd", null, parentDir(path)));
  meta.lastChild.title = parentDir(path);
  p.append(meta);
  return p;
}

async function fillMeta(metaDup, metaKeep, d, token) {
  const get = (p) => (p ? invoke("photo_info", { path: p }).catch(() => []) : Promise.resolve([]));
  const [a, b] = await Promise.all([get(d.dup), get(d.keep)]);
  if (token !== dToken) return;
  const pick = ["Prise de vue", "Dimensions", "Taille", "Appareil"];
  const ma = Object.fromEntries(a.map(([k, v]) => [k, fmtInfo(k, v)]));
  const mb = Object.fromEntries(b.map(([k, v]) => [k, fmtInfo(k, v)]));
  for (const k of pick) {
    if (ma[k]) {
      metaDup.append(el("dt", null, k), el("dd", d.keep && mb[k] && mb[k] !== ma[k] ? "diff" : null, ma[k]));
    }
    if (mb[k] && metaKeep) {
      metaKeep.append(el("dt", null, k), el("dd", ma[k] && ma[k] !== mb[k] ? "diff" : null, mb[k]));
    }
  }
}

function renderDupes() {
  const c = screen();
  if (!dupes.length) {
    const box = el("div", "empty");
    const ok = icon("check");
    ok.classList.add("logo");
    box.append(ok, el("h1", null, "Aucun doublon à vérifier"));
    box.append(el("p", null, "Le dossier _doublons_a_verifier de ta photothèque est vide."));
    c.append(box);
    return;
  }
  const list = dList();
  if (dIdx >= list.length) dIdx = Math.max(0, list.length - 1);
  const token = ++dToken;
  const wrap = el("div", "review");

  const head = el("div", "dupes-head");
  head.append(el("h1", null, "Revue des doublons"));
  head.append(el("span", "progress", list.length ? `${nf.format(dIdx + 1)} / ${nf.format(list.length)}` : "0"));
  const exact = dupes.filter((d) => d.kind === "doublon" && d.keep);
  if (exact.length) {
    const bulk = el("button", "btn", exact.length > 1 ? `Supprimer les ${nf.format(exact.length)} identiques` : "Supprimer l'identique");
    bulk.title = "Contenu identique octet par octet à l'exemplaire conservé : aucune perte possible.";
    bulk.onclick = () => trashMany(exact);
    head.append(bulk);
  }
  wrap.append(head);

  const filters = el("div", "filters");
  for (const [k, label] of DUP_FILTERS) {
    const n = k === "all" ? dupes.length : dupes.filter((d) => d.kind === k).length;
    if (k !== "all" && !n) continue;
    const b = el("button", dFilter === k ? "active" : null, label);
    b.append(el("span", "n", nf.format(n)));
    b.onclick = () => {
      dFilter = k;
      dIdx = 0;
      renderDupes();
    };
    filters.append(b);
  }
  wrap.append(filters);

  const d = list[dIdx];
  if (!d) {
    wrap.append(el("div", "empty", "Rien dans cette catégorie."));
    c.append(wrap);
    return;
  }
  const pair = el("div", "pair");
  const pDup = pane("dup", d.dup);
  const pKeep = pane("keep", d.keep);
  pair.append(pDup, pKeep);
  wrap.append(pair);
  fillMeta(pDup.querySelector(".pane-meta"), d.keep ? pKeep.querySelector(".pane-meta") : null, d, token);

  const actions = el("div", "dupes-actions");
  actions.append(el("span", "kind", KIND_LABEL[d.kind] || d.kind));
  const prev = el("button", "btn", "Précédent");
  prev.disabled = dIdx === 0;
  prev.onclick = () => dupStep(-1);
  const skip = el("button", "btn", "Garder les deux");
  skip.title = "Passer au suivant sans rien supprimer";
  skip.onclick = () => dupStep(1);
  const del = el("button", "btn danger", "Supprimer le doublon");
  del.onclick = trashDup;
  actions.append(prev, skip, del, el("span", "kbd", "Suppr : supprimer · → : garder les deux · ← : précédent"));
  wrap.append(actions);
  c.append(wrap);
}

function dupStep(n) {
  const list = dList();
  const next = dIdx + n;
  if (next < 0) return;
  if (next >= list.length) return toast("Fin de la liste");
  dIdx = next;
  renderDupes();
}

async function trashDup() {
  const d = dList()[dIdx];
  if (!d) return;
  if (!d.keep) {
    const ok = await confirmBox("Exemplaire conservé introuvable", `Mettre quand même ${baseName(d.dup)} à la corbeille ?`);
    if (!ok) return;
  }
  try {
    await invoke("trash_files", { paths: [d.dup] });
  } catch (e) {
    return toast("Échec : " + e);
  }
  dupes = dupes.filter((x) => x !== d);
  if (lib) lib.duplicates = Math.max(0, lib.duplicates - 1);
  renderSidebar();
  renderDupes();
}

async function trashMany(items) {
  const ok = await confirmBox(
    `Supprimer ${plural(items.length, "doublon identique", "doublons identiques")} ?`,
    "Ces fichiers sont identiques, octet par octet, à un exemplaire conservé dans ta photothèque. Ils seront mis dans la corbeille de Windows."
  );
  if (!ok) return;
  const paths = items.map((d) => d.dup);
  const done = new Set();
  try {
    for (let i = 0; i < paths.length; i += 100) {
      const chunk = paths.slice(i, i + 100);
      await invoke("trash_files", { paths: chunk });
      chunk.forEach((p) => done.add(p));
    }
  } catch (e) {
    toast("Échec : " + e);
  }
  dupes = dupes.filter((d) => !done.has(d.dup));
  if (lib) lib.duplicates = Math.max(0, lib.duplicates - done.size);
  dIdx = 0;
  renderSidebar();
  renderDupes();
  if (done.size) toast(`${plural(done.size, "fichier envoyé", "fichiers envoyés")} à la corbeille`);
}

// ---------------------------------------------------------------------------
// Photos similaires : paires proposées une par une, Clem tranche
// ---------------------------------------------------------------------------

let simPairs = [];
let sIdx = 0;
let sToken = 0;
let simLevel = 8;
let simWaiting = 0; // photos sans miniature (pas encore comparables)

const SIM_LEVELS = [
  [4, "Quasi identiques"],
  [8, "Très proches"],
  [12, "Ressemblantes"],
];
const resemblance = (d) => (d <= 2 ? "Quasi identiques" : d <= 5 ? "Très proches" : d <= 8 ? "Proches" : "Ressemblantes");

async function showSimilar() {
  if (!lib) return;
  view = { type: "similar" };
  renderSidebar();
  $("crumb").textContent = "Photos similaires";
  const c = screen();
  const box = el("div", "loading");
  const inner = el("div");
  inner.append(el("div", "spinner"), el("div", null, "Analyse des photos…"));
  inner.append(el("div", "hint", "La première fois, chaque miniature est analysée : cela peut prendre une minute."));
  box.append(inner);
  c.append(box);
  const items = [];
  simWaiting = 0;
  for (const it of itemsByPath.values()) {
    if (it.kind === "video" || it.private) continue;
    if (it.thumb) items.push([it.path, it.thumb]);
    else simWaiting++;
  }
  try {
    simPairs = await invoke("similar_pairs", { items, maxDist: simLevel });
  } catch (e) {
    simPairs = [];
    toast(String(e));
  }
  if (view.type !== "similar") return;
  sIdx = 0;
  renderSimilar();
}

function simPane(path, side, better) {
  const it = itemsByPath.get(path);
  const p = el("div", "pane sim");
  const head = el("div", "pane-head");
  head.append(el("span", null, side === "a" ? "Photo de gauche" : "Photo de droite"));
  if (better) head.append(el("span", "pill", "meilleure définition"));
  p.append(head);
  const t = sToken;
  p.append(paneMedia(path, () => t === sToken));
  const meta = el("dl", "pane-meta");
  meta.append(el("dt", null, "Fichier"), el("dd", null, baseName(path)));
  meta.lastChild.title = path;
  meta.append(el("dt", null, "Dossier"), el("dd", null, parentDir(path)));
  meta.lastChild.title = parentDir(path);
  if (it && (it.raw || it.motion)) {
    meta.append(el("dt", null, "Avec"), el("dd", null, [it.raw ? "son RAW" : "", it.motion ? "sa photo animée" : ""].filter(Boolean).join(" et ")));
  }
  p.append(meta);
  return p;
}

const pixels = (v) => {
  const m = /(\d[\d\s]*) × (\d[\d\s]*) px/.exec(v || "");
  return m ? +m[1].replace(/\s/g, "") * +m[2].replace(/\s/g, "") : 0;
};

function renderSimilar() {
  const c = screen();
  const token = ++sToken;
  const wrap = el("div", "review");

  const head = el("div", "dupes-head");
  head.append(el("h1", null, "Photos similaires"));
  head.append(el("span", "progress", simPairs.length ? `${nf.format(sIdx + 1)} / ${nf.format(simPairs.length)}` : "0"));
  wrap.append(head);

  const filters = el("div", "filters");
  for (const [lvl, label] of SIM_LEVELS) {
    const b = el("button", simLevel === lvl ? "active" : null, label);
    b.onclick = () => {
      simLevel = lvl;
      showSimilar();
    };
    filters.append(b);
  }
  wrap.append(filters);

  const pair = simPairs[sIdx];
  if (!pair) {
    const box = el("div", "empty");
    const ok = icon("check");
    ok.classList.add("logo");
    box.append(ok, el("h1", null, "Aucune paire à examiner"));
    const more = simWaiting ? ` ${plural(simWaiting, "photo n'a", "photos n'ont")} pas encore de miniature : reviens quand leur préparation sera terminée.` : "";
    box.append(el("p", null, `Rien d'autre à comparer avec ce réglage. Essaie « Ressemblantes » pour élargir la recherche.${more}`));
    wrap.append(box);
    c.append(wrap);
    return;
  }
  const pa = simPane(pair.a, "a", false);
  const pb = simPane(pair.b, "b", false);
  const row = el("div", "pair");
  row.append(pa, pb);
  wrap.append(row);
  // infos et meilleure définition
  Promise.all([invoke("photo_info", { path: pair.a }).catch(() => []), invoke("photo_info", { path: pair.b }).catch(() => [])]).then(([ia, ib]) => {
    if (token !== sToken) return;
    const ma = Object.fromEntries(ia.map(([k, v]) => [k, fmtInfo(k, v)]));
    const mb = Object.fromEntries(ib.map(([k, v]) => [k, fmtInfo(k, v)]));
    for (const k of ["Prise de vue", "Dimensions", "Taille", "Appareil"]) {
      if (ma[k]) pa.querySelector(".pane-meta").append(el("dt", null, k), el("dd", mb[k] && mb[k] !== ma[k] ? "diff" : null, ma[k]));
      if (mb[k]) pb.querySelector(".pane-meta").append(el("dt", null, k), el("dd", ma[k] && ma[k] !== mb[k] ? "diff" : null, mb[k]));
    }
    const [xa, xb] = [pixels(ma.Dimensions), pixels(mb.Dimensions)];
    const better = xa !== xb ? (xa > xb ? pa : pb) : null;
    if (better) better.querySelector(".pane-head").append(el("span", "pill", "meilleure définition"));
  });

  const actions = el("div", "dupes-actions");
  actions.append(el("span", "score", resemblance(pair.dist)));
  const left = el("button", "btn keep", "Garder la gauche");
  left.title = "Ce sont des doublons : la photo de droite va à la corbeille";
  left.onclick = () => simKeep("a");
  const right = el("button", "btn keep", "Garder la droite");
  right.title = "Ce sont des doublons : la photo de gauche va à la corbeille";
  right.onclick = () => simKeep("b");
  const no = el("button", "btn", "Non, elles sont différentes");
  no.title = "Ne plus jamais proposer cette paire";
  no.onclick = simNo;
  const skip = el("button", "btn", "Passer");
  skip.onclick = () => simStep(1);
  actions.append(left, right, no, skip, el("span", "kbd", "G : garder la gauche · D : garder la droite · N : différentes · → : passer"));
  wrap.append(actions);
  c.append(wrap);
}

function simStep(n) {
  const next = sIdx + n;
  if (next < 0) return;
  if (next >= simPairs.length) return toast("Fin de la liste");
  sIdx = next;
  renderSimilar();
}

async function simKeep(side) {
  const pair = simPairs[sIdx];
  if (!pair) return;
  const gone = side === "a" ? pair.b : pair.a;
  const it = itemsByPath.get(gone);
  const files = [gone, it && it.raw, it && it.motion].filter(Boolean);
  try {
    await invoke("trash_files", { paths: files });
  } catch (e) {
    return toast("Échec : " + e);
  }
  forget([gone]);
  removeFromAlbums([gone]);
  itemsByPath.delete(gone);
  clearViews();
  simPairs = simPairs.filter((p) => p.a !== gone && p.b !== gone);
  renderSidebar();
  renderSimilar();
  toast(`${baseName(gone)} envoyée à la corbeille`);
}

function simNo() {
  const pair = simPairs[sIdx];
  if (!pair) return;
  invoke("similar_ignore", { a: pair.a, b: pair.b }).catch(() => {});
  simPairs.splice(sIdx, 1);
  renderSimilar();
}

// ---------------------------------------------------------------------------
// Photos secrètes (dans PhotoPi seulement, les fichiers ne sont pas touchés).
// Masquées à chaque démarrage ; « Afficher » dans les Paramètres, avec le mot de passe,
// les remet dans la chronologie, à leur place, jusqu'à « Masquer » ou la fermeture.
// ---------------------------------------------------------------------------

let lockInfo = { has_password: false, count: 0, revealed: false };
let privatePw = null; // gardé en mémoire le temps de l'affichage seulement

/** Fenêtre de saisie. fields : [{ label }]. check(valeurs) renvoie un message d'erreur ou rien. */
function askPassword({ title, text, fields, okLabel = "Valider", check }) {
  return new Promise((resolve) => {
    $("pw-title").textContent = title;
    $("pw-text").textContent = text || "";
    $("pw-text").hidden = !text;
    $("pw-ok").textContent = okLabel;
    $("pw-error").textContent = "";
    const box = $("pw-fields");
    box.innerHTML = "";
    const inputs = fields.map((f, i) => {
      const id = "pw-in-" + i;
      const label = el("label", "field-label", f.label);
      label.htmlFor = id;
      const input = el("input", "input");
      input.type = f.type || "password";
      if (f.max) input.max = f.max;
      if (f.value) input.value = f.value;
      input.id = id;
      input.autocomplete = "off";
      box.append(label, input);
      return input;
    });
    $("pw-modal").hidden = false;
    inputs[0].focus();
    inputs[0].select();
    const done = (v) => {
      $("pw-modal").hidden = true;
      $("pw-form").onsubmit = $("pw-cancel").onclick = null;
      document.removeEventListener("keydown", onKey, true);
      resolve(v);
    };
    const onKey = (e) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        done(null);
      }
    };
    document.addEventListener("keydown", onKey, true);
    $("pw-cancel").onclick = () => done(null);
    $("pw-form").onsubmit = async (e) => {
      e.preventDefault();
      const values = inputs.map((i) => i.value);
      $("pw-ok").disabled = true;
      const err = check ? await check(values) : null;
      $("pw-ok").disabled = false;
      if (err) {
        $("pw-error").textContent = err;
        return;
      }
      done(values);
    };
  });
}

/** S'assure qu'un mot de passe existe (le fait choisir la première fois). */
async function ensurePassword() {
  if (lockInfo.has_password) return true;
  const v = await askPassword({
    title: "Choisis un mot de passe",
    text: "Il servira à afficher tes photos secrètes, depuis les Paramètres. Il n'y a pas de récupération possible : garde-le précieusement.",
    fields: [{ label: "Mot de passe" }, { label: "Confirmation" }],
    check: async ([a, b]) => {
      if (a.length < 4) return "Au moins 4 caractères.";
      if (a !== b) return "Les deux saisies ne correspondent pas.";
      try {
        await invoke("lock_set_password", { old: "", new: a });
      } catch (e) {
        return String(e);
      }
      return null;
    },
  });
  if (!v) return false;
  lockInfo.has_password = true;
  fillSettings();
  return true;
}

/**
 * Rend des éléments secrets. Photos secrètes masquées : ils quittent la chronologie ;
 * affichées : ils restent en place, avec un cadenas. Renvoie true si c'est fait.
 */
async function lockItems(items) {
  if (!(await ensurePassword())) return false;
  const files = items.flatMap((it) => [it.path, it.raw, it.motion].filter(Boolean));
  try {
    await invoke("lock_items", { paths: files });
  } catch (e) {
    toast(String(e));
    return false;
  }
  // rendre secret masque toujours : si les secrètes étaient affichées, on les masque toutes
  if (lockInfo.revealed) {
    await invoke("lock_hide").catch(() => {});
    lockInfo.revealed = false;
    privatePw = null;
    if (mediaFilter === "secret") {
      mediaFilter = "all";
      for (const b of $("media-filter").children) b.classList.toggle("active", b.dataset.f === "all");
    }
  }
  forget(items.map((it) => it.path));
  for (const it of items) itemsByPath.delete(it.path);
  for (const y of lib.years) for (const al of y.albums) al.items = al.items.filter((it) => !it.private);
  for (const al of lib.others) al.items = al.items.filter((it) => !it.private);
  for (const [p, it] of itemsByPath) if (it.private) itemsByPath.delete(p);
  forget([]);
  lib.locked += items.length;
  lockInfo.count += files.length;
  countThumbs();
  fillSettings();
  return true;
}

async function lockAlbum(a, shownItems = a.items) {
  const items = shownItems.filter((it) => !it.private);
  if (!items.length) return toast("Tout cet album est déjà secret");
  const name = a.month && !a.event ? cap(fmtMonth(a.month)) : a.title;
  const ok = await confirmBox(
    `Rendre « ${name} » secret ?`,
    `${plural(items.length, "élément sera masqué", "éléments seront masqués")} de la chronologie. Pour les revoir : Paramètres, « Photos secrètes », Afficher, avec ton mot de passe. Les fichiers ne sont pas modifiés.`,
    "Rendre secret",
    "primary",
    true
  );
  if (!ok) return;
  if (await lockItems(items)) {
    rerender();
    renderSidebar();
    toast(`${plural(items.length, "élément rendu secret", "éléments rendus secrets")}`);
  }
}

async function toggleLock() {
  const it = shown[vIdx];
  if (!it || it.explore) return;
  if (!it.private) {
    if (await lockItems([it])) afterRemoval("Photo rendue secrète");
    return;
  }
  const files = [it.path, it.raw, it.motion].filter(Boolean);
  try {
    await invoke("unlock_items", { password: privatePw, paths: files });
  } catch (e) {
    return toast(String(e));
  }
  it.private = it.locked = false;
  delete it.dataThumb;
  lib.locked = Math.max(0, lib.locked - 1);
  lockInfo.count = Math.max(0, lockInfo.count - files.length);
  countThumbs();
  fillSettings();
  rerender();
  showCurrent();
  toast("Photo retirée des photos secrètes");
}

async function revealSecrets() {
  const v = await askPassword({
    title: "Afficher les photos secrètes",
    text: "Elles réapparaîtront dans la chronologie jusqu'à ce que tu les masques ou que tu fermes PhotoPi.",
    fields: [{ label: "Mot de passe" }],
    okLabel: "Afficher",
    check: async ([pw]) => {
      try {
        await invoke("lock_reveal", { password: pw });
      } catch (e) {
        return String(e);
      }
      return null;
    },
  });
  if (!v) return;
  lockInfo.revealed = true;
  privatePw = v[0];
  fillSettings();
  await loadLibrary();
  toast("Photos secrètes affichées");
}

async function hideSecrets() {
  await invoke("lock_hide").catch(() => {});
  lockInfo.revealed = false;
  privatePw = null;
  if (mediaFilter === "secret") await setFilter("all");
  fillSettings();
  await loadLibrary();
  toast("Photos secrètes masquées");
}

// ---------------------------------------------------------------------------
// Import : dossier d'arrivée en vrac -> photothèque rangée par mois
// ---------------------------------------------------------------------------

let importInfo = { dir: "", pending: 0, exiftool: true };
let importState = "idle"; // idle | analyzing | ready | applying
let importSummary = null;
let importSource = ""; // vide : dossier d'arrivée ; sinon : autre dossier, copié sans doublons

const IMPORT_STEPS = {
  extract: "Extraction des archives",
  scan: "Inventaire du dossier",
  dates: "Lecture des dates de prise de vue",
  dups: "Recherche des doublons",
  quasi: "Recherche des photos recompressées",
  plan: "Préparation du rangement",
  move: "Rangement des fichiers",
  copy: "Copie des nouvelles photos",
};

async function refreshImportInfo() {
  try {
    importInfo = await invoke("import_info");
  } catch (_) {}
  renderSidebar();
  if (view && view.type === "import" && importState === "idle") renderImport();
}

TAURI.event.listen("import", (e) => {
  const { step, done, total } = e.payload;
  const box = document.querySelector("#screen .progress-box");
  if (!box) return;
  box.querySelector(".text").textContent = (IMPORT_STEPS[step] || step) + (total ? ` : ${nf.format(done)} / ${nf.format(total)}` : "…");
  const bar = box.querySelector(".bar-lg");
  bar.classList.toggle("indeterminate", !total);
  bar.querySelector("i").style.width = total ? ((done / total) * 100).toFixed(1) + "%" : "";
});

function showImport() {
  view = { type: "import" };
  renderSidebar();
  $("crumb").textContent = "Importer des photos";
  if (importState === "idle") refreshImportInfo();
  renderImport();
}

function progressBox() {
  const box = el("div", "progress-box");
  box.append(el("div", "text", "Préparation…"));
  const bar = el("div", "bar-lg indeterminate");
  bar.append(el("i"));
  box.append(bar);
  return box;
}

function renderImport() {
  if (!view || view.type !== "import") return;
  const c = screen();
  const wrap = el("div", "import");
  wrap.append(el("h1", null, "Importer des photos"));
  wrap.append(
    el(
      "p",
      "lead",
      "Dépose tes photos, vidéos, RAW ou archives Google Takeout en vrac dans le dossier d'arrivée. PhotoPi les range par mois dans ta photothèque, garde les dossiers bien nommés comme événements et met les doublons de côté. Rien n'est déplacé avant ta validation."
    )
  );

  const card = el("div", "card");
  card.append(el("span", "label", "Dossier d'arrivée"));
  const row = el("div", "path-row");
  const code = el("code", null, importInfo.dir || "Choisis d'abord ta photothèque");
  code.title = importInfo.dir;
  const open = el("button", "btn", "Ouvrir");
  open.title = "Ouvrir le dossier dans l'explorateur pour y déposer des photos";
  open.disabled = !importInfo.dir;
  open.onclick = () => invoke("open_folder", { path: importInfo.dir });
  const change = el("button", "btn", "Changer");
  change.onclick = async () => {
    const p = await invoke("pick_folder");
    if (!p) return;
    settings.import_dir = p;
    saveSettings();
    importState = "idle";
    refreshImportInfo();
  };
  row.append(code, open, change);
  card.append(row);
  card.append(
    el(
      "p",
      "hint",
      importInfo.pending
        ? `${plural(importInfo.pending, "élément en attente", "éléments en attente")} (photos, vidéos et archives).`
        : "Le dossier est vide pour l'instant. Dépose-y des photos, puis clique sur Actualiser."
    )
  );
  wrap.append(card);

  if (!importInfo.exiftool) {
    const n = el("div", "notice");
    n.append(icon("alert"));
    const t = el("div");
    t.append(document.createTextNode("ExifTool n'est pas installé : les dates des vidéos ne pourront pas être lues (elles iront dans « À trier »). Pour l'installer, dans PowerShell : "));
    t.append(el("code", null, "winget install -e --id OliverBetz.ExifTool"));
    t.append(document.createTextNode(", puis relance PhotoPi."));
    n.append(t);
    wrap.append(n);
  }

  const other = el("div", "card");
  other.append(el("span", "label", "Importer depuis un autre dossier"));
  other.append(
    el(
      "p",
      "hint",
      "Un ancien disque, une sauvegarde… PhotoPi copie seulement les photos et vidéos qui ne sont pas déjà dans ta photothèque. Le dossier choisi n'est jamais modifié."
    )
  );
  const pick = el("button", "btn", importSource ? "Choisir un autre dossier" : "Choisir un dossier…");
  pick.disabled = importState === "analyzing" || importState === "applying";
  pick.onclick = async () => {
    const p = await invoke("pick_folder");
    if (!p) return;
    importSource = p;
    analyzeImport();
  };
  const otherActions = el("div", "actions");
  otherActions.append(pick);
  if (importSource) otherActions.append(el("code", "src-path", importSource));
  other.append(otherActions);
  wrap.append(other);

  const zone = el("div", "card");
  if (importSource && importState !== "idle") zone.append(el("span", "label", "Copie depuis " + importSource));
  else if (importState !== "idle") zone.append(el("span", "label", "Dossier d'arrivée"));
  if (importState === "analyzing" || importState === "applying") {
    zone.append(progressBox());
  } else if (importState === "ready" && importSummary) {
    renderImportSummary(zone, importSummary);
  } else {
    const actions = el("div", "actions");
    const go = el("button", "btn primary", "Analyser");
    go.disabled = !importInfo.pending;
    go.onclick = analyzeImport;
    const refresh = el("button", "btn", "Actualiser");
    refresh.onclick = refreshImportInfo;
    actions.append(go, refresh);
    zone.append(actions);
  }
  wrap.append(zone);
  c.append(wrap);
}

function renderImportSummary(zone, s) {
  const toMove = s.sorted + s.in_events + s.unsorted;
  if (!s.files) {
    zone.append(el("p", null, "Aucune photo ni vidéo trouvée dans le dossier d'arrivée."));
  } else {
    const stats = el("div", "stats");
    const stat = (n, label, warn) => {
      const d = el("div", "stat" + (warn && n ? " warn" : ""));
      d.append(el("b", null, nf.format(n)), el("span", null, label));
      stats.append(d);
    };
    stat(s.sorted, "rangés par mois");
    stat(s.in_events, "dans des événements");
    stat(s.unsorted, "sans date (« À trier »)", true);
    stat(s.dups, importSource ? "déjà dans ta photothèque" : "doublons identiques");
    stat(s.quasi, importSource ? "déjà présentes, recompressées" : "doublons recompressés");
    stat(s.ignored, "autres fichiers, laissés en place");
    zone.append(stats);
    if (s.events.length) {
      zone.append(el("span", "label", "Événements"));
      const ul = el("ul", "ev-list");
      for (const [m, name, n] of s.events) {
        const li = el("li");
        li.append(el("span", null, m ? cap(fmtMonth(m)) : "Sans date"), el("b", null, name), el("span", null, nf.format(n)));
        ul.append(li);
      }
      zone.append(ul);
    }
    if (s.dups + s.quasi && !importSource) zone.append(el("p", "hint", "Les doublons iront dans « Doublons à vérifier » : rien n'est supprimé."));
    if (s.dups + s.quasi && importSource) zone.append(el("p", "hint", "Les photos déjà présentes ne sont pas copiées."));
  }
  const actions = el("div", "actions");
  if (s.files) {
    const go = importSource
      ? el("button", "btn primary", `Copier ${plural(toMove, "nouvel élément", "nouveaux éléments")}`)
      : el("button", "btn primary", `Ranger ${plural(s.files, "élément", "éléments")}`);
    go.disabled = importSource ? !toMove : false;
    go.title = `${nf.format(toMove)} dans la photothèque, ${nf.format(s.dups + s.quasi)} dans les doublons à vérifier`;
    go.onclick = applyImport;
    actions.append(go);
  }
  const cancel = el("button", "btn", "Annuler");
  cancel.onclick = () => {
    importState = "idle";
    importSummary = null;
    importSource = "";
    renderImport();
  };
  actions.append(cancel);
  zone.append(actions);
}

async function analyzeImport() {
  importState = "analyzing";
  renderImport();
  try {
    importSummary = await invoke("import_analyze", { source: importSource || null });
    importState = "ready";
  } catch (e) {
    importState = "idle";
    toast(String(e));
  }
  renderImport();
}

async function applyImport() {
  importState = "applying";
  renderImport();
  let res = null;
  try {
    res = await invoke("import_apply");
  } catch (e) {
    toast(String(e));
  }
  const copied = !!importSource;
  importState = "idle";
  importSummary = null;
  importSource = "";
  if (!res) return renderImport();
  const msg =
    `${plural(res.moved, copied ? "élément copié" : "élément rangé", copied ? "éléments copiés" : "éléments rangés")}` +
    (res.skipped ? `, ${nf.format(res.skipped)} ignorés (déjà présents ou introuvables)` : "");
  await loadLibrary();
  toast(msg);
}

// ---------------------------------------------------------------------------
// Explorer un dossier ou un disque, en lecture seule
// ---------------------------------------------------------------------------

let exploreRoot = "";
let exploreData = null; // [{ path, items }]
let exploreBusy = false;
let exploreSmall = false; // afficher les images de moins de 20 ko
const SMALL = 20 * 1024;
try {
  exploreRoot = localStorage.getItem("photopi.explorer") || "";
  exploreSmall = localStorage.getItem("photopi.explorer.petites") === "1";
} catch (_) {}

TAURI.event.listen("explore", (e) => {
  const box = document.querySelector("#screen .progress-box");
  if (!box || !exploreBusy) return;
  const { seen, found } = e.payload;
  box.querySelector(".text").textContent = `${plural(seen, "fichier parcouru", "fichiers parcourus")}, ${plural(found, "photo ou vidéo trouvée", "photos ou vidéos trouvées")}…`;
});

async function chooseExploreRoot() {
  const p = await invoke("pick_folder");
  if (!p) return;
  exploreRoot = p;
  try {
    localStorage.setItem("photopi.explorer", p);
  } catch (_) {}
  exploreData = null;
  scanExplore();
}

function dropExploreViews() {
  for (const [k, v] of [...views]) {
    if (k.startsWith("x|")) {
      if (k === current) current = null;
      v.div.remove();
      views.delete(k);
    }
  }
}

async function scanExplore() {
  if (!exploreRoot) return showExplore();
  view = { type: "explore" };
  exploreBusy = true;
  dropExploreViews();
  renderSidebar();
  $("crumb").textContent = "Explorer un dossier";
  const c = screen();
  const wrap = el("div", "import");
  wrap.append(el("h1", null, "Explorer un dossier"));
  wrap.append(el("p", "lead", exploreRoot));
  const card = el("div", "card");
  card.append(progressBox());
  wrap.append(card);
  c.append(wrap);
  try {
    const folders = await invoke("explore_scan", { path: exploreRoot });
    for (const f of folders) for (const it of f.items) it.explore = true;
    exploreData = folders;
  } catch (e) {
    exploreData = null;
    toast(String(e));
  }
  exploreBusy = false;
  if (view && view.type === "explore") showExplore();
}

/** Dater à la main : la sélection est déplacée dans le dossier du mois choisi. */
async function dateSelection() {
  const items = [...selected];
  if (!items.length || (view && view.type === "explore")) return;
  const files = (it) => [it.path, it.raw, it.motion].filter(Boolean);
  const guess = items.map((it) => it.date).filter(Boolean).sort()[0];
  const now = new Date();
  const max = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}`;
  const v = await askPassword({
    title: items.length > 1 ? `Dater ${nf.format(items.length)} éléments` : "Dater cet élément",
    text: "Choisis le mois de prise de vue. Les fichiers seront déplacés dans le dossier de ce mois ; ils ne sont pas modifiés.",
    fields: [{ label: "Mois", type: "month", value: guess ? guess.slice(0, 7) : "", max }],
    okLabel: "Ranger dans ce mois",
    check: ([m]) => (/^\d{4}-\d{2}$/.test(m) ? null : "Choisis un mois."),
  });
  if (!v) return;
  const month = v[0];
  let moved;
  try {
    moved = await invoke("date_items", { groups: items.map(files), month });
  } catch (e) {
    return toast(String(e));
  }
  // les albums suivent les photos déplacées
  const map = new Map(moved);
  let changed = false;
  for (const al of albums) {
    al.paths = al.paths.map((p) => {
      if (!map.has(p)) return p;
      changed = true;
      return map.get(p);
    });
  }
  if (changed) saveAlbums();
  clearSelection();
  const n = items.filter((it) => map.has(it.path)).length;
  await loadLibrary();
  toast(`${plural(n, "élément rangé", "éléments rangés")} en ${fmtMonth(month)}`);
}

/** Copie la sélection dans le dossier d'arrivée, puis lance l'analyse d'import. */
async function copySelectionToLibrary() {
  const items = [...selected];
  if (!items.length) return;
  const bar = $("sel-import");
  bar.disabled = true;
  let n = 0;
  try {
    n = await invoke("explore_copy", { paths: items.map((it) => it.path) });
  } catch (e) {
    bar.disabled = false;
    return toast("Copie impossible : " + e);
  }
  bar.disabled = false;
  clearSelection();
  toast(n ? `${plural(n, "fichier copié", "fichiers copiés")} : vérifie le résumé, puis « Ranger »` : "Déjà copiés dans le dossier d'arrivée");
  importState = "idle";
  importSummary = null;
  importSource = "";
  try {
    importInfo = await invoke("import_info");
  } catch (_) {}
  showImport();
  analyzeImport();
}

function showExplore() {
  view = { type: "explore" };
  renderSidebar();
  $("crumb").textContent = "Explorer un dossier";
  if (exploreBusy) return;
  if (!exploreData) {
    if (exploreRoot) return scanExplore();
    const c = screen();
    const wrap = el("div", "import");
    wrap.append(el("h1", null, "Explorer un dossier"));
    wrap.append(
      el(
        "p",
        "lead",
        "Choisis un dossier ou un disque entier : PhotoPi y cherche toutes les photos et vidéos, sous-dossiers compris, et les affiche par dossier. C'est en lecture seule : rien n'est déplacé, renommé ni ajouté à ta photothèque. Ta photothèque triée et les dossiers système ne sont pas parcourus."
      )
    );
    const b = el("button", "btn primary", "Choisir un dossier ou un disque");
    b.onclick = chooseExploreRoot;
    wrap.append(b);
    c.append(wrap);
    return;
  }
  const rootName = exploreRoot.replace(/[\\/]+$/, "");
  const rel = (p) => {
    const r = p.slice(rootName.length).replace(/^[\\/]+/, "");
    return r || rootName;
  };
  const keep = (it) => exploreSmall || it.kind === "video" || it.size >= SMALL;
  const v = activate("x|" + exploreRoot + "|" + mediaFilter + "|" + (exploreSmall ? 1 : 0), (div, list) => {
    div.classList.add("explore-view");
    const folders = exploreData.map((f) => ({ f, items: visible(f.items.filter(keep)) })).filter((x) => x.items.length);
    const total = folders.reduce((n, x) => n + x.items.length, 0);
    const hidden = exploreData.reduce((n, f) => n + f.items.filter((it) => !keep(it)).length, 0);
    const head = el("div", "explore-head");
    head.append(el("h1", null, rootName));
    const ro = el("span", "readonly");
    ro.append(icon("lock"), el("span", null, "Lecture seule"));
    head.append(ro, el("span", "sub", `${plural(total, "élément", "éléments")} dans ${plural(folders.length, "dossier", "dossiers")}`));
    const small = el("button", "btn", exploreSmall ? "Masquer les petites images" : `Afficher les petites images (${nf.format(hidden)})`);
    small.title = "Images de moins de 20 ko : icônes, vignettes de sites web…";
    small.onclick = () => {
      exploreSmall = !exploreSmall;
      try {
        localStorage.setItem("photopi.explorer.petites", exploreSmall ? "1" : "0");
      } catch (_) {}
      showExplore();
    };
    const again = el("button", "btn", "Relire");
    again.onclick = () => {
      exploreData = null;
      scanExplore();
    };
    const change = el("button", "btn", "Changer de dossier");
    change.onclick = chooseExploreRoot;
    head.append(small, again, change);
    div.append(head);
    if (!total) {
      div.append(el("p", "empty-filter", "Aucune photo ni vidéo à afficher ici."));
      return;
    }
    for (const { f, items } of folders) {
      const a = { id: f.path, title: rel(f.path), month: null, event: false, items };
      div.append(sectionFor(a, list.length, true));
      list.push(...items);
    }
  });
  $("content").scrollTop = v.scroll;
}

// ---------------------------------------------------------------------------
// Paramètres
// ---------------------------------------------------------------------------

function fillSettings() {
  if ($("set-lock")) {
    const n = lib ? lib.locked : 0;
    $("set-lock").textContent = !lockInfo.has_password
      ? "Aucune photo secrète"
      : `${lockInfo.revealed ? "Affichées" : "Masquées"}${n ? " (" + nf.format(n) + ")" : ""}`;
    const sw = $("set-secret");
    sw.checked = lockInfo.revealed;
    sw.disabled = !lockInfo.has_password;
    sw.title = lockInfo.has_password ? "" : "Rends d'abord une photo secrète : touche L dans la visionneuse";
    sw.closest(".switch-row").querySelector("use").setAttribute("href", lockInfo.revealed ? "#i-unlock" : "#i-lock");
    $("set-lock-btn").textContent = lockInfo.has_password ? "Mot de passe" : "Choisir un mot de passe";
  }
  $("set-library").textContent = settings.library || "Aucun";
  $("set-library").title = settings.library;
  $("set-reports").textContent = settings.reports || "Aucun";
  $("set-reports").title = settings.reports;
}

function fmtBytes(n) {
  if (n >= 1e9) return (n / 1e9).toFixed(2).replace(".", ",") + " Go";
  if (n >= 1e6) return (n / 1e6).toFixed(0) + " Mo";
  return Math.round(n / 1e3) + " ko";
}

function initSettings() {
  const panel = $("settings");
  $("btn-settings").onclick = (e) => {
    e.stopPropagation();
    panel.hidden = !panel.hidden;
    $("btn-settings").classList.toggle("active", !panel.hidden);
    if (!panel.hidden) {
      fillSettings();
      $("set-cache").textContent = "…";
      invoke("cache_size").then((n) => ($("set-cache").textContent = fmtBytes(n))).catch(() => {});
    }
  };
  document.addEventListener("mousedown", (e) => {
    if (!panel.hidden && !panel.contains(e.target) && !$("btn-settings").contains(e.target) && $("modal").hidden && $("pw-modal").hidden) {
      panel.hidden = true;
      $("btn-settings").classList.remove("active");
    }
  });
  $("set-library-btn").onclick = chooseLibrary;
  $("set-secret").onchange = (e) => {
    const want = e.target.checked;
    e.target.checked = lockInfo.revealed; // ne bascule qu'une fois le mot de passe accepté
    if (want) revealSecrets();
    else hideSecrets();
  };
  $("set-lock-btn").onclick = async () => {
    panel.hidden = true;
    $("btn-settings").classList.remove("active");
    if (!lockInfo.has_password) return ensurePassword();
    const v = await askPassword({
      title: "Changer le mot de passe",
      fields: [{ label: "Mot de passe actuel" }, { label: "Nouveau mot de passe" }, { label: "Confirmation" }],
      check: async ([old, a, b]) => {
        if (a.length < 4) return "Au moins 4 caractères.";
        if (a !== b) return "Les deux saisies ne correspondent pas.";
        try {
          await invoke("lock_set_password", { old, new: a });
        } catch (e) {
          return String(e);
        }
        return null;
      },
    });
    if (v) {
      if (privatePw) privatePw = v[1];
      toast("Mot de passe changé");
    }
  };
  $("set-similar-btn").onclick = () => {
    panel.hidden = true;
    $("btn-settings").classList.remove("active");
    showSimilar();
  };
  $("set-reports-btn").onclick = async () => {
    const p = await invoke("pick_folder");
    if (!p) return;
    settings.reports = p;
    saveSettings();
    fillSettings();
    if (view && view.type === "dupes") showDupes();
  };
  $("set-cache-btn").onclick = async () => {
    await invoke("clear_cache").catch(() => {});
    for (const it of itemsByPath.values()) it.thumb = null;
    countThumbs();
    $("set-cache").textContent = fmtBytes(0);
    if (lib) {
      rerender();
      startPregen();
    }
    toast("Cache vidé : les miniatures sont en cours de recréation");
  };
  $("btn-rescan").onclick = () => {
    if (settings.library) loadLibrary();
  };
  const tile = $("tile");
  tile.value = settings.tile;
  document.documentElement.style.setProperty("--tile", settings.tile + "px");
  tile.addEventListener("input", () => {
    settings.tile = +tile.value;
    document.documentElement.style.setProperty("--tile", settings.tile + "px");
    clearTimeout(tile._t);
    tile._t = setTimeout(reestimate, 120);
  });
  tile.addEventListener("change", saveSettings);
}

// ---------------------------------------------------------------------------
// Clavier, fenêtre, démarrage
// ---------------------------------------------------------------------------

document.addEventListener("keydown", (e) => {
  if (!$("modal").hidden || !$("pw-modal").hidden || !$("pick-modal").hidden) return;
  if (!$("viewer").hidden) {
    if (e.key === "Escape") closeViewer();
    else if (e.key === "ArrowLeft") step(-1);
    else if (e.key === "ArrowRight") step(1);
    else if (e.key === "i" || e.key === "I") toggleInfo();
    else if (e.key === "Delete") trashCurrent();
    else if (e.key === " " && shown[vIdx] && shown[vIdx].motion) toggleMotion();
    else if (e.key === "l" || e.key === "L") toggleLock();
    else return;
    e.preventDefault();
    return;
  }
  if (current && view && (view.type === "year" || view.type === "album" || view.type === "talbum" || view.type === "explore") && !(e.target instanceof HTMLInputElement)) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      selectItems(shown);
      e.preventDefault();
      return;
    }
    if (selected.size && e.key === "Escape") {
      clearSelection();
      e.preventDefault();
      return;
    }
    if (selected.size && e.key === "Delete") {
      trashSelection();
      e.preventDefault();
      return;
    }
  }
  if (view && view.type === "similar" && !(e.target instanceof HTMLInputElement)) {
    const k = e.key.toLowerCase();
    if (k === "g") simKeep("a");
    else if (k === "d") simKeep("b");
    else if (k === "n") simNo();
    else if (e.key === "ArrowRight") simStep(1);
    else if (e.key === "ArrowLeft") simStep(-1);
    else return;
    e.preventDefault();
    return;
  }
  if (view && view.type === "dupes" && !(e.target instanceof HTMLInputElement)) {
    if (e.key === "Delete") trashDup();
    else if (e.key === "ArrowRight") dupStep(1);
    else if (e.key === "ArrowLeft") dupStep(-1);
    else return;
    e.preventDefault();
  }
});

{
  const win = TAURI.window.getCurrentWindow();
  $("win-min").addEventListener("click", () => win.minimize());
  $("win-max").addEventListener("click", () => win.toggleMaximize());
  $("win-close").addEventListener("click", () => win.close());
  const syncMax = async () => {
    const max = await win.isMaximized();
    $("win-max").querySelector("use").setAttribute("href", max ? "#i-copy" : "#i-square");
    $("win-max").title = max ? "Restaurer" : "Agrandir";
  };
  win.onResized(syncMax);
  syncMax();
}

(async function start() {
  try {
    settings = Object.assign(settings, await invoke("get_settings"));
    lockInfo = await invoke("lock_status");
    const a = await invoke("albums_get");
    albums = Array.isArray(a) ? a : [];
  } catch (_) {}
  initSettings();
  initViewer();
  initScrubber();
  for (const b of $("media-filter").children) {
    b.classList.toggle("active", b.dataset.f === mediaFilter);
    b.onclick = () => setFilter(b.dataset.f);
  }
  $("sel-album").onclick = () => addToAlbum([...selected]);
  $("sel-import").onclick = copySelectionToLibrary;
  $("sel-date").onclick = dateSelection;
  $("sel-unalbum").onclick = () => removeFromAlbum([...selected]);
  $("sel-lock").onclick = lockSelection;
  $("sel-unlock").onclick = unlockSelection;
  $("sel-trash").onclick = trashSelection;
  $("sel-clear").onclick = clearSelection;
  fillSettings();
  loadLibrary();
})();
