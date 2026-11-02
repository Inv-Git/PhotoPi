<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" alt="PhotoPi logo">
</p>

<h1 align="center">PhotoPi</h1>

<p align="center">
  A fast viewer for your Windows photo library, organised by year and month, with no account and no cloud.<br>
  <a href="README.fr.md">Version française</a>
</p>

---

## Why PhotoPi

PhotoPi is the little sibling of [MusicPi](https://github.com/Inv-Git/MusicPi): your photos, your folders, on your own drive. It is designed for a library organised as `YYYY/YYYY-MM/`, and it never changes your files unless you ask (only the Windows Recycle Bin is used, always with a confirmation).

The interface is in French for now.

## Features

**Timeline**
- The whole library in one continuous scroll, newest first, with a year scrubber to travel through time
- Thumbnails generated in the background and kept on disk: nothing reloads when you come back to a month
- Events (`YYYY-MM Name` folders) highlighted, other folders in their own section
- All / Photos / Videos / Secret filter

**Viewer**
- Full screen, arrow keys, wheel zoom around the cursor, actual size on double-click
- RAW files (embedded preview), videos, smartphone motion photos (played on hover and with Space)
- Shooting info: date, camera, lens, settings, GPS

**Organising**
- Built-in import: drop photos, videos or Google Takeout archives into an inbox folder, PhotoPi sorts them by month and sets duplicates aside (preview before anything moves)
- Import from another folder or drive: only photos missing from the library are copied, the source is never changed
- Browse any folder or whole drive read-only, and tick images to add them to the library
- Date undated photos by hand ("À trier"): they are moved into the chosen month's folder
- Multi-selection (check boxes, Ctrl + click, Shift + click, Ctrl + A)
- Thematic albums: a photo can be in several albums, files never move
- Secret photos: hidden in PhotoPi behind a password (files are neither changed nor encrypted)
- Review of the duplicates left by the sorting script, and comparison of similar photos (keep one, keep the other, or "they are different")

## Building

### Requirements

- Windows 10 or 11 (WebView2 is already there)
- [Rust](https://rustup.rs/) (stable)
- [Node.js](https://nodejs.org/) 18 or newer

### Run and build

```
npm install
npm run dev
```

To build the installer:

```
npm run build
```

The installer ends up in `src-tauri/target/release/bundle/nsis/`.

## Data

Settings, the date index, albums and the secret photo list live in `%APPDATA%\fr.clem.photopi`. Thumbnails live in `%LOCALAPPDATA%\fr.clem.photopi`. The secret photo list only holds hashes, no file names and no password.

## Credits

- [Tauri](https://tauri.app/): desktop app framework
- [Lucide](https://lucide.dev/): icons (ISC license)
- [Manrope](https://github.com/sharanda/manrope): typeface (SIL Open Font License)
- Rust crates: image, kamadak-exif, trash, serde

## License

PhotoPi's source code is under the [MIT license](LICENSE).
