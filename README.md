# Siksik

Siksik is a desktop app that makes images smaller without losing quality. It's written in Rust and runs on Windows, macOS and Linux.

Drop in images or whole folders, or pick them with a file browser, and Siksik compresses them. You can also have it watch folders so new images are compressed automatically.

| Format | What Siksik does | Typical saving |
|---|---|---|
| **JPEG** | Repacks the existing image data with better (optimised Huffman + progressive) encoding using MozJPEG. The pixels stay exactly the same. | 5–15 % |
| **PNG** | Tries many filter, colour-type and deflate strategies with oxipng. The pixels stay exactly the same. | 10–40 % |
| **WebP** (lossless) | Re-encodes at maximum lossless effort with libwebp. Lossy and animated WebPs are left alone. | 10–35 % |
| **JPEG, Smart mode** (optional) | A JPEGmini-style re-encode with MozJPEG trellis quantisation. It is visually lossless rather than bit-exact. | 30–70 % |

Siksik only keeps a result if it is actually smaller. If an image is already optimal, it is left alone.

## Features

- **Drag and drop** single images, many images or folders onto the window. You can also use **Choose images** (multi-select) or **Choose folder**.
- **Watch folders.** Images created in or moved into these folders are compressed automatically while Siksik is running. Siksik waits until a file has finished copying, and it never processes its own output twice.
- **Output:** save to the same folder or another folder. With another folder, the sub-folder structure is mirrored.
- **Overwrite originals or create a copy.** Copies are named `photo-siksik.jpg`, and the suffix can be changed. In another folder, a copy that would clash gets a number: `photo (1).jpg`.
- **Statistics:** successful and failed counts for this session, plus all-time image counts and bytes saved.
- **Light, dark or system theme.**
- Optionally keeps metadata. Colour profiles and photo rotation are always kept.
- Keeps each file's original modified date, and uses all CPU cores.
- **Custom app icon.** See [Changing the app icon](#changing-the-app-icon).

## Install

### Option A: download an installer (easiest)

Download the file for your system from the **Releases** page:

| System | File | Notes |
|---|---|---|
| Windows 10/11 | `Siksik_x.y.z_x64-setup.exe` (or `.msi`) | Double-click it. The `.exe` installs for your user only, so no admin rights are needed. |
| macOS (Intel and Apple Silicon) | `Siksik_x.y.z_universal.dmg` | Open it and drag Siksik into Applications. The build isn't signed, so the first time you open it, right-click the app and choose **Open**. |
| Debian / Ubuntu | `siksik_x.y.z_amd64.deb` | `sudo apt install ./siksik_*.deb` |
| Fedora / openSUSE | `siksik-x.y.z-1.x86_64.rpm` | `sudo dnf install ./siksik-*.rpm` |
| Any Linux | `Siksik_x.y.z_amd64.AppImage` | `chmod +x Siksik*.AppImage` and run it. |

**To produce these installers yourself:** push this folder to a GitHub repository, then push a tag:

```sh
git tag v1.0.0 && git push --tags
```

The workflow in `.github/workflows/release.yml` builds all of the installers above on GitHub's machines and attaches them to a draft release. You can also start it by hand from the **Actions** tab.

### Option B: build from source

You need:

1. **Rust:** https://rustup.rs
2. **Node.js 18+:** https://nodejs.org (used only for the Tauri build tool)
3. **NASM**, which speeds up MozJPEG:
   - Windows: `winget install NASM.NASM`
   - macOS: `brew install nasm`
   - Linux: `sudo apt install nasm`
4. Platform extras:
   - **Windows:** "Desktop development with C++" from the Visual Studio Build Tools. WebView2 is already part of Windows 10/11.
   - **macOS:** `xcode-select --install`
   - **Debian/Ubuntu:**
     ```sh
     sudo apt install libwebkit2gtk-4.1-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev cmake
     ```

Then:

```sh
npm install
npm run build      # installers end up in src-tauri/target/release/bundle/
npm run dev        # or run it in development mode
```

## Changing the app icon

There are two ways:

1. **Inside the app (no rebuild).** Go to **Settings → Appearance → App icon** and choose any PNG. This changes the window and taskbar icon on Windows and Linux. Use **Reset** to go back to the default.

2. **Everywhere: installer, shortcuts, Dock, .exe and in-app logo.** Replace the icon at build time with one command. Use a square PNG (1024×1024 is best) or an SVG:

   ```sh
   ./scripts/set-icon.sh  path/to/my-icon.png      # macOS / Linux
   .\scripts\set-icon.ps1 C:\path\to\my-icon.png   # Windows PowerShell
   npm run build
   ```

   This generates every size and format each platform needs (`.ico`, `.icns`, the PNG sizes, and the sidebar logo). The default icon's source is `app-icon.svg` if you'd like to edit it instead.

## Where Siksik stores its data

Settings and all-time statistics are kept in `settings.json` and `stats.json` in:

- Windows: `%APPDATA%\Siksik\`
- macOS: `~/Library/Application Support/Siksik/`
- Linux: `~/.config/Siksik/`

## Project layout

```
src/                 UI (plain HTML/CSS/JS – no build step)
src-tauri/src/
  lib.rs             app setup, commands, statistics
  compress.rs        file discovery, output naming, PNG/WebP, safe writes
  jpeg.rs            MozJPEG lossless transcoding + Smart mode
  watcher.rs         folder watching (notify)
  settings.rs        settings & stats persistence
scripts/set-icon.*   icon replacement
.github/workflows/   cross-platform installer builds
```

## Open-source libraries

- [Tauri 2](https://tauri.app): cross-platform desktop shell (MIT/Apache-2.0)
- [oxipng](https://github.com/shssoichiro/oxipng): lossless PNG optimiser (MIT)
- [MozJPEG](https://github.com/mozilla/mozjpeg) via [mozjpeg-sys](https://crates.io/crates/mozjpeg-sys) / [mozjpeg](https://crates.io/crates/mozjpeg): JPEG optimisation (BSD-style/IJG)
- [libwebp](https://chromium.googlesource.com/webm/libwebp) via [webp](https://crates.io/crates/webp): WebP encoding (BSD)
- [notify](https://github.com/notify-rs/notify): cross-platform file watching (CC0/MIT/Apache-2.0)
- [rayon](https://github.com/rayon-rs/rayon), [walkdir](https://github.com/BurntSushi/walkdir), [filetime](https://github.com/alexcrichton/filetime), [serde](https://serde.rs)

Running the engine's tests on a folder of images:

```sh
cd src-tauri && SIKSIK_TEST_DIR=/path/to/images cargo test --release -- --nocapture
```
