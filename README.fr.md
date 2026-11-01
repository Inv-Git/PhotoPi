<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" alt="Logo PhotoPi">
</p>

<h1 align="center">PhotoPi</h1>

<p align="center">
  Une visionneuse rapide pour ta photothèque Windows, rangée par année et par mois, sans compte ni cloud.<br>
  <a href="README.md">English version</a>
</p>

---

## Pourquoi PhotoPi

PhotoPi est le petit frère de [MusicPi](https://github.com/Inv-Git/MusicPi) : tes photos, tes dossiers, sur ton disque. Il est pensé pour une photothèque rangée en `AAAA/AAAA-MM/`, et ne modifie jamais tes fichiers sans que tu le demandes (seule la corbeille de Windows est utilisée, toujours avec confirmation).

## Fonctions

**Chronologie**
- Toute la photothèque d'un seul tenant, de la plus récente à la plus ancienne, avec une réglette des années pour voyager dans le temps
- Miniatures préparées en tâche de fond et gardées sur le disque : rien ne se recharge quand tu reviens sur un mois
- Événements (dossiers `AAAA-MM Nom`) mis en avant, autres dossiers dans une section à part
- Filtre Tout / Photos / Vidéos / Secrètes

**Visionneuse**
- Plein écran, flèches du clavier, zoom à la molette autour du curseur, taille réelle au double-clic
- RAW (aperçu intégré), vidéos, photos animées des smartphones (lues au survol et avec Espace)
- Informations de prise de vue : date, appareil, objectif, réglages, GPS

**Organisation**
- Sélection multiple (cases, Ctrl + clic, Maj + clic, Ctrl + A)
- Albums thématiques : une photo peut être dans plusieurs albums, les fichiers ne bougent pas
- Photos secrètes : masquées dans PhotoPi derrière un mot de passe (les fichiers ne sont ni modifiés ni chiffrés)
- Revue des doublons laissés par le script de tri, et comparaison des photos similaires (garder l'une, garder l'autre ou « elles sont différentes »)

## Compiler

### Prérequis

- Windows 10 ou 11 (WebView2 y est déjà)
- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18 ou plus récent

### Lancer et compiler

```
npm install
npm run dev
```

Pour fabriquer l'installateur :

```
npm run build
```

L'installateur se trouve ensuite dans `src-tauri/target/release/bundle/nsis/`.

## Données

Les réglages, l'index des dates, les albums et la liste des photos secrètes sont dans `%APPDATA%\fr.clem.photopi`. Les miniatures sont dans `%LOCALAPPDATA%\fr.clem.photopi`. La liste des photos secrètes ne contient que des empreintes, ni noms de fichiers ni mot de passe.

## Crédits

- [Tauri](https://tauri.app/) : cadre de l'application
- [Lucide](https://lucide.dev/) : icônes (licence ISC)
- [Manrope](https://github.com/sharanda/manrope) : police (SIL Open Font License)
- Crates Rust : image, kamadak-exif, trash, serde

## Licence

Le code de PhotoPi est sous [licence MIT](LICENSE).
