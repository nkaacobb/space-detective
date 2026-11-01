# Space Detective

A Rust desktop app that shows where your disk space went. Pick a folder or drive and every file
becomes a box sized by how much space it takes. Folders become boxes holding their contents.

```
cargo run --release
cargo run --release -- "D:\Some\Folder"   # scan a folder straight away
```

The Rust version is pinned in `rust-toolchain.toml`, and rustup installs it automatically.

## Using it

- **Browse…**, type a path and press Enter, or click a drive button to start a scan.
- **Hover** a box to see its name, path and size.
- **Click** a box to select it. The right-hand panel shows its details.
- **Double-click** a box to zoom into its folder. Use **⬆ Up**, the breadcrumb path, Backspace or the
  mouse back button to zoom out.
- **Right-click** a box to zoom in, show it in Explorer or copy its path.
- Colours show file types. The biggest types get their own colours, listed under *File types*.

Symlinks and junctions aren't followed, so nothing is counted twice. Sizes are file sizes, not the
space allocated on disk.

## Making installers

The version comes from `version` in `Cargo.toml`. Bump it before each release.

**Windows** (on a Windows PC with [Inno Setup](https://jrsoftware.org/isdl.php) installed):

```
powershell -ExecutionPolicy Bypass -File packaging\windows\build-installer.ps1
```

This makes `dist\SpaceDetective-Setup-<version>.exe`. It installs for the current user without
needing admin rights, adds a Start menu entry and an optional desktop shortcut, and has an
uninstaller. The C runtime is linked in, so it needs nothing else on the PC.

**macOS** (on a Mac with Rust and the Xcode command line tools):

```
bash packaging/macos/build-dmg.sh
```

This makes `dist/SpaceDetective-<version>.dmg` with a universal app for Apple Silicon and Intel Macs.

**Both at once, on GitHub:** `.github/workflows/release.yml` builds both on GitHub's Windows and
Mac machines. Run it from the repository's Actions tab, or push a tag such as `v0.1.0` to get a
draft release with both files attached.

### Warnings people will see

Neither download is code-signed yet, so:

- **Windows** SmartScreen says "Windows protected your PC". People click *More info* → *Run anyway*.
  Signing the installer (for example with Azure Trusted Signing, or a code-signing certificate)
  removes this.
- **macOS** says the app can't be checked for malware. People open *System Settings › Privacy &
  Security* and click *Open Anyway*. An Apple Developer account ($99/year) lets `build-dmg.sh`
  sign and notarize the app (see the top of that script), which removes this.

The app icon is drawn by `packaging/make_icons.py` (needs Pillow). It writes everything in `assets/`.
