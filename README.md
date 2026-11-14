# BPM Detector

A small desktop app: drop an audio file on the window, press **Detect BPM**, and read the tempo
with two decimals.

Besides the tempo, the app shows:

- **Confidence**: High, Medium or Low, depending on how clearly the beat repeats across the track.
- **Half and double tempo**: half/double-time readings are ambiguous by nature (87 vs 174).
- **Tempo stability**: the two halves of the track are measured separately. When they disagree,
  the app shows both values.

Supported files: WAV, AIFF, MP3, FLAC, M4A (AAC-LC / ALAC) and OGG Vorbis, at least 6 seconds long.

## Download

Ready-made builds are attached to each [release](../../releases). The builds are not signed with
a paid developer certificate, so each system asks for confirmation the first time.

### macOS (Apple Silicon and Intel)

1. Unzip `BPM-Detector-macOS-universal.zip`.
2. Move `BPM Detector.app` to `/Applications`.
3. If macOS refuses to open it, go to *System Settings → Privacy & Security* and choose
   **Open Anyway**. Alternatively, run:

   ```sh
   xattr -dr com.apple.quarantine "/Applications/BPM Detector.app"
   ```

### Windows

Run `BPM-Detector-Windows-x86_64.exe`. If SmartScreen appears, choose **More info → Run anyway**.

### Linux (x86_64)

Extract `BPM-Detector-Linux-x86_64.tar.gz` and run `bpm-detector`. It needs an X11 or Wayland
desktop.

## How it works

1. The audio is decoded with [Symphonia](https://github.com/pdeljanov/Symphonia) and mixed down to mono.
2. An onset envelope (log-band spectral flux) is computed every ~2.7–2.9 ms.
3. The coarse tempo is the strongest autocorrelation peak between 60 and 200 BPM. When two
   readings compete (for example 86 and 172), a log-normal preference centred on 130 BPM picks one.
4. The beat period is refined on the autocorrelation peaks at 1, 2, 4, 8, … beats, up to half the
   track. Tracking stops where a peak collapses (tempo changes, live playing). The autocorrelation
   is interpolated exactly on a grid 8× finer than the envelope. The final period is a
   least-squares fit through the peaks at every whole number of beats.
5. The same refinement runs on each half of the track to check that the tempo is steady.

## Accuracy

- Synthetic drum tracks (`tests/synthetic_tracks.rs`): within 0.0001 BPM at 85–174.3 BPM and
  44.1–96 kHz.
- 23 finished tracks with a known project tempo (80–175 BPM):
  - 22 are correct to two decimals: mean error 0.0009 BPM, max 0.005 BPM (on a 30-second clip;
    full-length tracks stay within 0.002 BPM).
  - 1 live-drummed track at 175 BPM reads as 87.50 (half time). The exact double is shown next
    to it.

## Build

You need a stable [Rust toolchain](https://rustup.rs) on every platform. The app was built and
tested by hand on macOS (Apple Silicon). On Windows and Linux, the release workflow builds it and
runs the test suite, but nobody has tried the app by hand there yet.

### macOS

Requirements: Xcode Command Line Tools (`xcode-select --install`), for `swiftc`, `sips`,
`iconutil` and `codesign`.

```sh
./scripts/bundle.sh
```

The script:

- builds the release binary;
- draws the icon (`packaging/make_icon.swift`);
- assembles `target/bundle/BPM Detector.app` and signs it ad hoc;
- installs it in `/Applications`, replacing any previous copy.

To keep it in the Dock, drag it there from `/Applications`.

Options:

- `--universal`: build one binary for both Apple Silicon and Intel Macs.
- `--no-install`: leave the app in `target/bundle` without installing it.

### Windows

Requirements: the MSVC toolchain. Install Visual Studio Build Tools with the
*Desktop development with C++* workload before running `rustup`.

```powershell
cargo build --release
```

The app is `target\release\bpm-detector.exe`. Start it, then right-click its taskbar icon and
choose **Pin to taskbar**.

### Linux

Requirements on Debian/Ubuntu (other distributions need the equivalent packages):

```sh
sudo apt-get install libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

```sh
cargo build --release
```

The app is `target/release/bpm-detector`. To add it to the application launcher, create
`~/.local/share/applications/bpm-detector.desktop`, using the absolute path of the binary:

```ini
[Desktop Entry]
Type=Application
Name=BPM Detector
Exec=/absolute/path/to/bpm-detector
Terminal=false
Categories=AudioVideo;Audio;
```

### Run without packaging (any platform)

```sh
cargo run --release
```

## Development

```sh
cargo test --release
cargo run --release --example analyze -- path/to/track.wav another.mp3
```

- Run the tests in release mode: they synthesize several minutes of audio.
- The `analyze` example prints the tempo with five decimals, plus confidence, stability and
  timing.

## Releasing

Bump `version` in `Cargo.toml` and `CFBundleShortVersionString` in `packaging/Info.plist`, then
push a tag:

```sh
git tag -a v1.0.1 -m "BPM Detector 1.0.1"
git push origin v1.0.1
```

`.github/workflows/release.yml` then:

- builds the app on macOS (universal), Windows and Linux;
- runs the tests on all three;
- publishes a GitHub release with the three builds attached.
