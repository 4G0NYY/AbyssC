# 🌌 AbyssC

> *She compressed the massive, world-threatening All-Devouring Narwhal into a tiny, glowing orb with a simple gesture of her hand.*

**AbyssC** — *AbyssCompress* — is a performance-first, modular compression engine written entirely in Rust. It takes what the surface struggles to hold and folds it down to something smaller. Space is not sacred here; it is merely something to be condensed.

The surface measures archives by familiarity. The Abyss measures them by **power** — throughput, and the ratio of what remains. That is the only metric that matters.

---

## ⚔️ Power

One command, many codecs. The format is chosen by the extension you name — nothing more is asked of you.

- **Seven codecs.** `zstd`, `lz4`, `gzip`, `xz`, `bzip2`, `brotli`, and raw `store`.
- **Three containers.** A single compressed stream, a `tar` bundle, or a portable `zip`.
- **Streaming by nature.** Bytes flow through 1 MiB buffers. Nothing is held whole in memory — a 100 GB file costs the same RAM as a 100 KB one.
- **Multithreaded `zstd`.** It claims every core you give it, or as many as you permit.
- **Whole directories.** `tar.*` and `.zip` swallow entire trees. The single streams take one file, as is their nature.
- **Inspect without touching.** List an archive's contents without unfolding it.

---

## 🏛️ Lineage — Architecture

The engine does not tangle its concerns. An **archive format** is the union of two independent ideas:

```
            Format
          ╱        ╲
   Container        Codec
   (layout)         (compression)
   ┌─────────┐      ┌──────────────────────────────────┐
   │  Raw    │      │ Store · Gzip · Zstd · Lz4 ·       │
   │  Tar    │  ×   │ Xz · Bzip2 · Brotli              │
   │  Zip    │      └──────────────────────────────────┘
   └─────────┘
```

- **`archive_engine`** — the disciplined core. A library, no voice of its own.
  - `codec.rs` — every algorithm behind one inversion-of-control API. Each codec wraps a stream and handles its own finalization. The container layer never learns their secrets.
  - `format.rs` — `Format = Container + Codec`, with extension detection.
  - `compress.rs` / `decompress.rs` — dispatch over `Raw`, `Tar`, `Zip`.
  - `zip_archive.rs` — the `zip` path, which bundles and compresses in one pass.
  - `listing.rs` — reads an archive's table of contents.
- **`orchestrator`** — the hand that gestures. A thin CLI (`abyssc`) that resolves a format and calls the core.
- **`abyss_gui`** — the same hand, made visible. A sleek windowed front-end (`abyssc-gui`), peer to the CLI, that calls the very same core.

Adding a codec touches one file and one detection table. The rest of the engine does not stir. A new front-end adds a crate and touches nothing else — the GUI did not change a single line of the engine's logic, only *observed* it through a thread-safe `Progress` counter.

---

## 🜂 Summoning — Build

Requires a Rust toolchain (edition 2024; Rust 1.85+). On Windows, the native codecs (`zstd`, `xz`, `bzip2`) compile bundled C through MSVC — install the **VC++ Build Tools**.

```sh
git clone <this-repo> AbyssC
cd AbyssC
cargo build --release
```

The blade is forged at `target/release/abyssc` (`abyssc.exe` on Windows), and the
window at `target/release/abyssc-gui`.

```sh
cargo test --release           # round-trip every format, byte-for-byte
cargo run --release -p abyss_gui   # open the window
```

---

## 🜍 Binding — The Installer

To bind AbyssC to a Windows machine, there is a clean [Inno Setup](https://jrsoftware.org/isinfo.php)
installer in [`installer/`](installer/). It builds the release binaries, then forges a single `Setup.exe`:

```powershell
# Requires Inno Setup 6 (winget install JRSoftware.InnoSetup)
installer\build.ps1
# → installer\dist\AbyssC-<version>-Setup.exe
```

The installer reads its version straight from the workspace `Cargo.toml`, so it
never drifts from the binaries. Running it (as administrator) will:

- Install **`abyssc.exe`** (CLI) and **`abyssc-gui.exe`** (GUI) to *Program Files*.
- Add the CLI to the system **PATH** (idempotent — and removed again on uninstall).
- Place **Start Menu** shortcuts (and an optional desktop icon).
- Carve a cascading **AbyssC** entry into the right-click menu of files and folders:
  *Compress with AbyssC* · *Extract with AbyssC* · *Open in AbyssC Commander*.

Each verb hands the selected path to the GUI (`--compress`, `--extract`,
`--browse`), which opens straight into the right mode. A tidy uninstaller undoes
all of it — binaries, PATH entry, and registry keys alike.

---

## 🔮 Incantations — Usage

```
abyssc compress  -o <archive> [opts] <inputs...>   (alias: c)
abyssc extract   -i <archive> [-o <dir>]           (alias: x)
abyssc list      -i <archive>                      (alias: l)
abyssc help                                         the field guide
abyssc version                                      the banner (alias: v)
abyssc <command> --help                             precise detail
```

### Compress

The output extension decides the format. Speak the name; the engine understands.

```sh
abyssc compress -o backup.tar.zst  project/ notes.txt   # bundle a tree → zstd
abyssc compress -o data.lz4        data.bin             # raw velocity
abyssc compress -o data.zst -l 19 -t 8  data.bin        # crush it, eight cores
abyssc compress -o site.tar.br     www/                 # brotli a directory
```

### Extract

```sh
abyssc extract -i backup.tar.zst -o ./restored
abyssc extract -i data.lz4                       # → ./data  (name derived from archive)
```

### List

Look into the orb without breaking it open.

```sh
abyssc list -i backup.tar.zst
```

```
Archive: backup.tar.zst [tar.zstd]
          SIZE  NAME
         <dir>  project/
          13 B  project/src/main.rs
      5.86 KiB  project/README
  2 file(s), 1 dir(s), 5.87 KiB uncompressed
```

### Version

```sh
abyssc version    # or: abyssc v
```

```
      _    _                    ____
     / \  | |__  _   _ ___ ___  / ___|
    / _ \ | '_ \| | | / __/ __|| |
   / ___ \| |_) | |_| \__ \__ \| |___
  /_/   \_\_.__/ \__, |___/___/ \____|
                 |___/

  AbyssC v0.3.0  —  compression from the depths
```

The version is declared once, in the workspace root (`[workspace.package]`), and inherited by every crate — so the banner, `-V`, and the crate metadata can never drift apart.

---

## 🪟 Visage — The Window

For those who prefer to gesture rather than incant, there is `abyssc-gui` — a
sleek, dark, **Abyssal** desktop application, built on [Iced](https://iced.rs):
pure Rust, GPU-rendered, **no webview**. It follows WinRAR's familiar shape and
strips away its clutter.

```sh
cargo run --release -p abyss_gui
```

- **Three modes.** *Compress* (gather sources, choose a form, fold them),
  *Extract* (open an archive, peer inside, unfold it), and *Commander*.
- **The Commander.** A file browser that treats archives as folders. Step into a
  `.tar.zst` and walk its directories as though they lay open on disk — nothing
  is ever decompressed to look inside. From within, extract the whole thing in a
  click; from the filesystem, send any file straight to *Compress*.
- **Drag the world in.** Drop files and folders straight onto the window.
- **It never freezes.** The engine crunches on a worker thread while the window
  stays fluid; a live bar reflects a lock-free `Progress` counter polled from the
  UI. A 100 GB fold draws at the same frame rate as a 100 KB one.
- **One palette.** Frost-cyan and abyssal violet on near-black — Skirk's colors,
  not a surface dweller's.
- **Stays current.** On launch it quietly asks GitHub for the latest release; if a
  newer depth has surfaced, a small banner offers to fetch it. No telemetry, no
  account, no server of our own — just a single anonymous request, failing silent
  when offline.

The GUI shares the engine with the CLI exactly; neither knows the other exists.

---

## 📜 Forms — Formats

| Extension              | Codec    | Container | Disposition                                   |
| ---------------------- | -------- | --------- | --------------------------------------------- |
| `.zst`, `.tar.zst`     | zstd     | raw / tar | Balance of speed and ratio. Multithreaded.    |
| `.lz4`, `.tar.lz4`     | lz4      | raw / tar | Raw velocity. The fastest blade.              |
| `.gz`, `.tar.gz`       | gzip     | raw / tar | The old, ubiquitous standard.                 |
| `.xz`, `.tar.xz`       | xz/lzma  | raw / tar | Patient. Crushes hardest, moves slowest.      |
| `.bz2`, `.tar.bz2`     | bzip2    | raw / tar | Legacy weight.                                |
| `.br`, `.tar.br`       | brotli   | raw / tar | The web's chosen ratio.                       |
| `.zip`                 | deflate  | zip       | Portable. Compresses per entry.               |
| `.tar`                 | store    | tar       | Bundle only. No compression.                  |

Short aliases (`.tgz`, `.tzst`, `.txz`, `.tbz2`) are recognized. Use `--format <name>` to override detection.

**Single streams** (`.zst`, `.gz`, `.lz4`, `.xz`, `.bz2`, `.br`) compress exactly **one file**. To fold a directory or several files, name a `tar.*` or `.zip` target.

---

## 🎚️ Effort — Levels

`-l, --level` sets effort: higher means smaller and slower. Each codec keeps its own scale; the value is clamped to what it understands, so one flag serves all.

| Codec  | Range          | Default | Notes                                   |
| ------ | -------------- | ------- | --------------------------------------- |
| zstd   | 1 – 22         | 3       | `-l 19`+ for serious ratio.             |
| gzip   | 0 – 9          | 6       |                                         |
| xz     | 0 – 9          | 6       |                                         |
| bzip2  | 1 – 9          | 9       |                                         |
| brotli | 0 – 11         | 6       |                                         |
| lz4    | —              | —       | Ignores level. It has one speed: fast.  |

`-t, --threads` directs `zstd`'s workers. `0` (default) claims every core.

---

## 📊 Measured Power

50 MB of mixed compressible/incompressible data, default levels, one machine. *Your depths will differ.*

| Format | Ratio  | Throughput   |
| ------ | ------ | ------------ |
| `lz4`  | 5.1 %  | ~1600 MB/s   |
| `gz`   | 5.4 %  | ~915 MB/s    |
| `zip`  | 5.4 %  | ~920 MB/s    |
| `zst`  | 4.7 %  | ~250 MB/s    |
| `br`   | 4.7 %  | ~248 MB/s    |
| `xz`   | 4.8 %  | ~20 MB/s     |
| `bz2`  | 4.8 %  | ~6 MB/s      |

Read it plainly: **`lz4`** when speed is everything, **`zst`** for balance (and far better ratio at higher levels), **`xz`** when you have time and want the bytes gone.

---

## 🌑 Closing

> *"It is only natural that those without power have no voice."*

AbyssC has no interest in the politics of file formats. It compresses, it extracts, it does so quickly. The rest is surface noise.
