# C — Audio Re-encoding for shrinkray (Phase 1)

Research note for the Rust-based UE4/UE5 game folder shrinker. Scope: what cooked
audio actually looks like on disk, what the realistic savings are, and which
Rust crates can do the work without bundling FFmpeg.

## 1. How UE Stores Cooked Audio

A `USoundWave` asset on disk is the same three-file pattern as every other
cooked UE asset: a `.uasset` header, a `.uexp` exports blob, and a `.ubulk`
bulk-data file. The audio bytes themselves live across the `.uexp`/`.ubulk`
split — for larger sounds, the `.uexp` carries a chunk table plus chunk 0, and
the remaining ~256 KiB chunks (`0x40000` bytes each, padded) live in `.ubulk`.
Header metadata (codec tag, sample rate, channels, loop info) sits in the
`USoundWave` properties inside `.uexp`.
[Source: https://forums.unrealengine.com/t/what-are-uexp-and-ubulk-files/390611]
[Source: https://github.com/timhok/ue2wav]

The chunks are **raw codec packets, not full container files** — no Ogg pages,
no RIFF, no Bink container header. To produce a playable file you have to
combine the chunk-table info from `.uexp` with the packet payload from
`.ubulk` and wrap it yourself. That's exactly what `ue2wav` does for Bink and
what CUE4Parse does generically. [Source: https://github.com/timhok/ue2wav]
[Source: https://github.com/FabianFG/CUE4Parse/blob/master/CUE4Parse-Conversion/Sounds/SoundDecoder.cs]

### Platform codecs
Per UE docs and forum activity, the actual codec on disk depends on platform
and project settings:
- **Bink Audio (BINKA / RADA)** — default for UE5 desktop since the
  RAD/Epic acquisition; previous default was "Platform Specific".
  [Source: https://forums.unrealengine.com/t/all-imported-sounds-defaults-to-bink-audio-can-one-change-this/626066]
  [Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/bink-audio]
- **Ogg Vorbis** — historic Windows/Linux/Mac default in UE4 and still common
  in any UE5 project that overrode the default.
- **Opus** — option on PC, default on Switch.
- **ADPCM / PCM** — small-clip/console paths.
- A typical UE5 build cook will list required formats like
  `['BINKA','ADPCM','PCM','OPUS','RADA']`.
  [Source: https://forums.unrealengine.com/t/failed-to-find-these-required-audioformats-binka-adpcm-pcm-opus-rada-after-build/2275635]

**Verdict for "what's on disk in a typical shipped UE5 PC game":** mostly
Bink Audio (BINKA) for SFX/voice and either Bink or Vorbis for music,
depending on when the project was set up. Mid-2022-onward UE5 projects are
Bink-by-default. UE4 holdovers are Vorbis-by-default.

## 2. The "Audio Is Already Compressed" Reality

Shipped UE audio is lossy at roughly 96–128 kbps Vorbis or comparable Bink
quality. Industry norm for UE/Vorbis is "Compression Quality 40–60 → 96–128
kbps", with 128 kbps being the de-facto default.
[Source: https://moldstud.com/articles/p-understanding-unreal-engines-sound-wave-assets-a-deep-dive-into-audio-mastery]

The efficiency math against Opus:
- Vorbis transparency: ~150–170 kbps.
- Opus transparency: ~128 kbps; at 96 kbps Opus already rivals AAC@128.
- Opus is ~20–30% more efficient than Vorbis at low/mid bitrates.
  [Source: https://opus-codec.org/comparison/]
  [Source: https://en.wikipedia.org/wiki/Opus_(audio_format)]

So if you take Vorbis@128 → Opus@96 you get roughly a 25% per-file shrink, but
you also pay a generational quality hit (lossy → decode → lossy). For a
1 GB audio bucket that's ~250 MB saved — real but not heroic. The README's
"30–60% of audio" estimate is **optimistic for the Vorbis case**: it only
holds if the source happens to be overcooked (192+ kbps music) or if you're
willing to drop perceptual quality below transparency. Bulk Vorbis → Opus
re-encoding on a normally-cooked game is closer to a flat **20–25% audio
savings, with a quality cost**.

## 3. Where Audio Re-encoding *Does* Win

Mass re-encoding lossy → lossy is the bad case. The good cases are:
1. **Uncompressed WAV / PCM `USoundWave`** — rare in shipped AAA but common
   in Marketplace asset packs, indie titles, and modded folders. PCM at
   1411 kbps → Opus at 96 kbps is a ~93% per-file cut. This is the headline
   number that justifies a re-encode feature at all.
2. **FLAC sources** — same story: ~700–900 kbps lossless → ~96 kbps Opus is
   ~85% savings with no perceivable hit on game audio.
3. **Over-cooked music** — shipping music at 192+ kbps Vorbis is common when
   the dev didn't tune per-asset settings. Down to 128 kbps Opus is a clean
   ~33% trim, and music gets less critical-listening attention than dialogue.

Phase 1 should target **(1) and (2) only**, and add an opt-in flag for (3).
Touching every Vorbis/Bink file by default is the path to user complaints
about audio quality with weak headline savings.

## 4. Rust Crate Landscape

### Decode: `symphonia`
- Pure Rust, MPL-2.0, maintained by Philip Deljanov, 3.2k stars,
  last push 2026-05-15.
  [Source: gh api repos/pdeljanov/Symphonia]
- Decodes Vorbis, MP3, AAC, FLAC, ALAC, ADPCM, AIFF, CAF, MP1/2/3, MP4, OGG,
  WAV, WebM, MKV.
  [Source: https://github.com/pdeljanov/Symphonia]
- **No encoding. No roadmap for encoding.** Roadmap items are C API + WASM.
- No Bink decoder (Bink is proprietary).

### Encode: `opus` (libopus bindings)
- High-level safe bindings for libopus, v0.3.1, MIT/Apache-2.0.
  [Source: https://docs.rs/opus/latest/opus/]
- Encoder + Decoder + multi-stream up to 255 channels + Repacketizer.
- **Requires libopus system dep** — adds a build-time dependency, but libopus
  itself is BSD, so distribution is fine.
- Alternative: `audiopus` (links libopus 1.3, similar surface), `opus-codec`
  (vendors libopus 1.5.2 via git subtree — easier static builds).

### Vorbis encoder in Rust
- `vorbis_rs` v0.5.5, BSD-3-Clause, bindings to `libvorbisenc` + `libvorbis`
  + `vorbisfile`. Encoder and decoder both present.
  [Source: https://docs.rs/vorbis_rs/latest/vorbis_rs/]
- `lewton` is decode-only.
- **No production-grade pure-Rust Vorbis encoder exists.** `bschwind/opus` is
  a WIP pure-Rust Opus encoder/decoder but is explicitly experimental.
  [Source: gh search repos "rust opus encoder"]

### Why not bundle FFmpeg and shell out?
Pros: handles literally every codec including Bink decode (via the existing
binkadec stub), well-tested. Cons:
- LGPL-2.1+ baseline, with optional GPL pieces (libx264, libx265, several
  filters). Static linking the LGPL build requires either matching license or
  shipping object files for relink — operationally a pain for a Tauri app.
  [Source: https://www.ffmpeg.org/legal.html]
- Dynamic linking is the clean path: ship FFmpeg DLLs/dylibs/so's alongside,
  document replaceability. But that's +20–40 MB of binary baggage on top of
  Tauri's ~10 MB target.
- A `ffmpeg` system-binary shell-out works (no link issue, treated as a
  separate program) but pushes a dependency onto the user.

For shrinkray's "small Tauri binary" preference, the right answer is
**libopus + libvorbis bindings statically linked** (both BSD/permissive) and
**no FFmpeg** in Phase 1. Reserve FFmpeg-shell-out as an optional Phase 3
escape hatch for codecs we can't otherwise read.

## 5. The `.uasset` / `.uexp` Audio Round-trip Problem

Same shape as the texture problem from note A/B: decoding is the easy half,
**writing back is the hard half**. To replace audio in a cooked `USoundWave`
you must:
1. Parse the `.uasset` summary + `USoundWave` export properties.
2. Read the chunk table from `.uexp` (offset, padded size, real size per chunk).
3. Locate chunks 1..N in `.ubulk`.
4. Decode the original codec → PCM.
5. Re-encode PCM → target codec (probably Opus) packetized to ≤256 KiB chunks.
6. Rewrite chunk table, patch `.uexp`, rewrite `.ubulk`, fix sizes/offsets in
   the `.uasset` summary so name-table and export-map offsets still line up.
7. If the asset is inside a `.pak` or IoStore container, rewrite the entry
   and its TOC hashes (see note A).

CUE4Parse has all the read-side primitives but is read-only for cooked
output. There is **no existing Rust crate that round-trips a `USoundWave`**.
shrinkray would need to write this — it's the same general "fork
CUE4Parse semantics into Rust" effort the texture path needs, just for the
sound asset type.
[Source: https://github.com/FabianFG/CUE4Parse/blob/master/CUE4Parse-Conversion/Sounds/SoundDecoder.cs]

## 6. Bink Audio (`.bk2` / BINKA)

Bink Video and Bink Audio are now bundled free with UE since 2021 (Epic
acquired RAD).
[Source: https://www.unrealengine.com/en-US/blog/bink-video-and-bink-audio-now-available-in-unreal-engine-for-free]
**Decoder ships in UE (`BinkAudioDecoder`), encoder requires the RAD Bink
SDK**, which is not freely redistributable for non-UE consumers. There is no
Rust binding. ue2wav shells out to a stand-alone `binkadec.exe` for decode.
[Source: https://github.com/timhok/ue2wav]

For shrinkray that means:
- **Can't re-encode to Bink** without licensing the SDK.
- *Could* decode Bink → PCM → Opus (if we got a decoder), but that's lossy →
  lossy on a codec the dev specifically chose, so quality risk is high.
- The clean Phase 1 stance: **skip Bink entirely**. Surface it in the scan
  report ("X MB Bink audio detected, not touched") so the user knows what's
  excluded.

## 7. Localization-Only Audio

UE puts localized sound waves under `Content/L10N/<lang>/...`, with the cue
graph kept language-agnostic and only the wave package swapped.
[Source: https://forums.unrealengine.com/t/how-do-i-localize-audio-tracks/474986]
[Source: https://dev.epicgames.com/documentation/en-us/unreal-engine/localization-overview-for-unreal-engine]

For a fully-voiced AAA UE5 game shipping 8 dub languages, voice audio can be
the single largest pak chunk. Dropping 7 languages is **100% quality
preservation for the user** and often 30–60% of the total audio bucket
outright — frequently more than every re-encoding trick combined. This
overlaps Stream's localization stripping but the audio is the heavy share.

Phase 1 should treat **L10N stripping as a first-class audio feature**, not a
re-encoding feature. The user picks "keep en-US" and shrinkray drops the
other `L10N/` trees from the pak/IoStore container. No decoding involved.

## Phase 1 Scope Verdict

**Do in Phase 1 (high value, low risk):**
- **Localization audio stripping** — biggest single audio win, zero quality
  loss for the user, no codec round-trip needed. (Shares plumbing with the
  Stream/L10N pak rewriter.)
- **PCM/WAV → Opus@96** for any uncompressed `USoundWave` (mostly mods +
  Marketplace), via `symphonia` decode + `opus` encode. Headline ~85–93%
  per-file shrink.
- **FLAC → Opus@96** same path.
- **Scan + report** Bink and Vorbis audio sizes so the user sees them in the
  preview, but **don't re-encode them by default**.

**Defer to Phase 2+ (medium value, real risk):**
- **Vorbis → Opus** behind an opt-in `--recompress-vorbis` flag with a clear
  "lossy→lossy quality warning". Realistic savings: 20–25%, not 30–60%.
- **Over-cooked music down-bitrate** (any source ≥192 kbps → 128 kbps),
  opt-in.

**Skip entirely (not worth Phase 1 cost):**
- **Bink Audio re-encode** — no free SDK, no Rust crate, decode-only via
  external binary, lossy→lossy risk. Report-only.
- **FFmpeg bundling** — license/binary-size cost too high for Phase 1.

**Honest take:** the README's "30–60% of audio" headline only holds if (a) the
game ships meaningful uncompressed PCM (uncommon for AAA, common for indie/
modded), or (b) the user accepts L10N stripping. The pure "re-encode all
Vorbis" pitch is a ~20% trickle with quality risk and complex `.uexp`/`.ubulk`
round-trip code. **Lead with L10N stripping + PCM/FLAC recompression. Treat
Vorbis recompression as an optional power-user toggle. Skip Bink.**
