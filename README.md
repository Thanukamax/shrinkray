# shrinkray

> Cut Unreal Engine game folder size by astronomical amounts. Drop a game folder, get a smaller game folder.

## Why

UE games ship with predictable bloat:

- Textures in formats with 30–50% recompression headroom (BC1 → BC7 ASTC, mipmap strip, format conversion)
- Audio at WAV / high-bitrate Vorbis that re-encodes to Opus at 30–60% reduction
- Pak files using default zlib when zstd/oodle compress 5–15% tighter
- Localization assets for languages the user will never play (20–40% on heavily localized titles)
- Asset-flip / RPG Maker UE port bloat (sometimes 80% unused)

Existing tooling (UnrealPak, umodel, FModel, UAssetGUI, repak) **reads** UE assets. Nobody ships the integrated "analyze → apply → smaller game" workflow.

## Realistic targets

| Game type | Typical savings |
|---|---|
| Asset-flip indie / RPG Maker UE ports | 50–80% |
| Heavily-modded titles | 30–60% |
| Mid-tier indie | 20–40% |
| Already-optimized AAA | 5–15% |

## Roadmap

| Phase | What | State |
|---|---|---|
| 0 | Scaffold + folder census by file extension, rough savings estimate | ✅ |
| 1 | Pak unpack/repack (`repak`), real texture recompression (BC1/3/7 via `intel_tex_2`), Opus audio re-encode | ⏳ |
| 2 | In-pak asset detection (`.uasset` parsing) | ⏳ |
| 3 | Localization stripping (language picker, audio + text + UI textures) | ⏳ |
| 4 | Unused asset manifest crossref (orphan detection) | ⏳ |
| 5 | CLI binary alongside GUI (shared core crate) | ⏳ |
| 6 | Movie/Bink re-encode, shader cache strip | ⏳ |

## Stack

- **Tauri v2** + **Bun** + **Vite** + **React 19** + **TypeScript** (mirrors [vn2apk](https://github.com/Thanukamax/vn2apk) + [wgpu-shader-explorer](https://github.com/Thanukamax/wgpu-shader-explorer) pattern)
- Rust backend: `walkdir`, `tauri-plugin-dialog`. Phase 1 adds `repak`, `image`, `texpresso`, `intel_tex_2`, `symphonia`, `opus`, `rayon`.

## Quick start

```bash
bun install
cd src-tauri && cargo fetch && cd ..
bun run tauri dev
```

Drop a UE game folder in the picker. The current scaffold walks the tree, classifies textures / audio / paks by extension, and reports a rough savings estimate. Real recompression lands in Phase 1.

## Layout

```
src/                   # React frontend (folder picker, report panel)
src-tauri/
  src/main.rs          # bin entry → lib::run()
  src/lib.rs           # Tauri builder + analyze_folder command
  src/analyze.rs       # folder census (V0)
  src/pak.rs           # pak read/write (Phase 1 stub)
  src/texture.rs       # texture detect + encode (Phase 1 stub)
  src/audio.rs         # audio detect + encode (Phase 1 stub)
  Cargo.toml
  tauri.conf.json
.github/workflows/build.yml
```

## License
MIT. See [LICENSE](LICENSE).
