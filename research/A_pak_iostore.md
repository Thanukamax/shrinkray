# A. Pak & IoStore Research for shrinkray Phase 1

Research date: 2026-05-18. Scope: Unreal Engine `.pak` versions, IoStore (`.utoc`/`.ucas`), compression options, Oodle licensing, AES encryption, the `repak` Rust crate, and a Phase 1 verdict for shrinkray.

## 1. Pak format versions

UE's classic `.pak` is a flat archive with a fixed-size footer pointing back at a central index. The header magic is `0x5A6F12E1`; immediately after the magic the footer carries a `u32` version field, followed by `index_offset (u64)`, `index_size (u64)`, and a 20-byte SHA1 of the index. For versions > 8 there is an additional 160-byte block (5 * 32 chars) listing the compression method names used by entries in the file. So **version detection is footer-driven** — seek to end, read the fixed-size tail, then parse the index. [Source: https://github.com/panzi/u4pak] [Source: https://simoncoenen.com/blog/programming/PakFiles]

Version timeline (relevant ones):
- **v3** — Initial widely-used format, UE 4.0–4.16 era, compressed blocks introduced.
- **v4** — Index-level encryption flag (`bEncryptedIndex`).
- **v5** — Relative compressed block offsets (smaller index).
- **v7** — Encryption GUID per pak (multi-key support).
- **v8** — Per-entry compression method index instead of hard-coded zlib.
- **v8A/8B** — Frozen index variant used by some 4.22+ titles.
- **v9** — Path-hash + full directory index (faster mount, partial-load friendly).
- **v10** — Used briefly by 4.25 IoStore preview branches.
- **v11** — UE 4.27 / UE 5.0+. **Important caveat:** UE 4.27 made breaking internal changes without bumping the footer version, so a "v11" header can mean either the pre-4.27 layout or the 4.27+ layout — you cannot distinguish them from the footer alone, you have to probe the index. [Source: https://github.com/panzi/u4pak]

`repak` reads versions **1–11** and writes versions **2–9 and 11**. [Source: https://github.com/trumank/repak]

## 2. IoStore (.utoc / .ucas)

IoStore is UE's newer container format. It splits content into a **`.utoc`** (table of contents: chunk IDs, offsets, sizes, compression blocks, hashes) and one or more **`.ucas`** payload files. A small "global" `.pak` is usually shipped alongside for legacy mount points and shader libraries. The split exists so the runtime can do high-throughput async I/O against SSDs (the design target was PS5's I/O stack). [Source: https://www.radgametools.com/oodlekraken.htm] [Source: https://gbatemp.net/threads/how-to-unpack-pack-utoc-ucas-in-unreal-engine-4-5-games.666431/]

Adoption: IoStore appeared as **opt-in in UE 4.25 / 4.26 / 4.27** and became the **default cooker output in UE 5.0+**. Most modern UE5 shipped titles are IoStore-first; the `.pak` next to them is often a thin stub. [Source: https://forums.unrealengine.com/t/new-unreal-engine-5-dlc-packaging-pak-utoc-ucas/600692] [Source: https://buckminsterfullerene02.github.io/dev-guide/Basis/DealingWithPaks.html]

**`repak` does NOT support IoStore** — it is a `.pak`-only library. The same author (trumank) ships a **separate** tool, **`retoc`** (https://github.com/trumank/retoc), which packs/unpacks `.utoc`/`.ucas` and converts between Zen and Legacy assets. retoc is MIT-licensed, ~370 commits, active as of January 2026, but is documented as solid on **UE 5.3+** only and shakier on UE4 IoStore games. [Source: https://github.com/trumank/retoc]

Implication for shrinkray: a UE5 game folder will typically be **mostly `.ucas`** by byte volume; touching only `.pak` would leave 80%+ of the install untouched.

## 3. Compression formats

UE supports a small named set: **Zlib, Gzip, LZ4, Oodle (Kraken / Mermaid / Selkie / Leviathan / Hydra), and Zstd**. The pak footer (v8+) stores the literal method name string per slot, and entries reference a method by index.

`repak` status (v0.2.3, Jan 2026):
- **Reads**: Zlib, Gzip, Zstd, and Oodle (via the `oodle` feature). [Source: https://github.com/trumank/repak]
- **Writes**: **No compression at all yet** — repacks are written uncompressed. [Source: https://github.com/trumank/repak]
- Encryption write also unimplemented.

**The Oodle problem.** Oodle is proprietary RAD Game Tools middleware (acquired by Epic in 2021). It is "free to use in Unreal Engine" — meaning Epic pays the per-title license fee for studios shipping via UE — but it is **not** an open license that lets a third-party tool like shrinkray bundle and redistribute `oo2core_*.dll`. [Source: https://www.unrealengine.com/en-US/blog/oodle-now-free-to-use-in-unreal-engine-via-github]

The community workaround, used by `repak`, is the **`oodle_loader`** crate: at runtime it downloads, verifies, and dynamically loads the Oodle library from **Epic's own official distribution endpoint**, so the tool never ships the DLL. This is the pattern that keeps the library legally clean. [Source: https://deepwiki.com/trumank/repak/4.1-oodle-integration]

Free decoder-only alternative exists (**`ooz`** / "Leviathan + Updated Kraken Decoder", reverse-engineered) which can decompress Kraken/Mermaid/Selkie/Leviathan, but is decode-only and of murky legal standing. [Source: https://encode.su/threads/3068-Leviathan-Updated-Kraken-Decoder]

## 4. The honest savings question

Modern UE5 titles ship Kraken-compressed pak/IoStore content. Public benchmarks:
- **Silesia / mozilla corpus:** Oodle Kraken **3.51:1** vs Zstd max **3.24:1**.
- **Mixed corpus** (cbloom): Kraken8 **3.10:1** vs Zstd-22 **2.75:1**.
- Across game-asset workloads, **Kraken consistently beats Zstd on ratio AND on decode speed.** [Source: https://fgiesen.wordpress.com/2024/08/08/oodle-kraken-etc-misconceptions/] [Source: http://cbloomrants.blogspot.com/2016/04/performance-of-oodle-kraken.html]

Therefore: **if a game already ships Oodle Kraken, repacking with anything in the free toolchain is a net loss** — Zstd is worse on ratio, Zlib is much worse. And repak today *cannot write Oodle at all*, so any repack would actively inflate the install. Realistic Phase-1 pak-repack savings on a modern UE5 title are **roughly 0% to negative**.

Where repacking *can* win:
- Removing **dev/debug paks** (PDBs, editor cooked data, shipped shader cache duplicates).
- Removing **unused languages / unused platform variants** packed into the same container.
- Re-emitting a pak with **dead/orphan entries dropped** after duplicate detection.

These are pak *trimming* wins, not pak *recompression* wins. The bytes saved come from removing entries, not from a better codec.

## 5. Encryption

A real fraction of shipped paks have **AES-256 encryption**, either on the index only (`bEncryptedIndex`, v4+) or on data blocks too. UE supports rotating per-pak GUIDs (v7+) so one game can ship many keys.

`repak` can **read** both index-encrypted and data-encrypted paks when given the key as base64 or hex, but cannot **write** encrypted paks. [Source: https://github.com/trumank/repak]

Key sourcing in practice — tools like **FModel** and UAssetGUI do **not** crack the AES; they pull keys from community-maintained lists:
- `FModel/Unreal-Game-Keys` GitHub repo (per-game `0x…` hex keys, contributor PRs). [Source: https://github.com/FModel/Unreal-Game-Keys]
- Game-specific archives (e.g. `dippyshere/fortnite-aes-archive`).
- `gildor.org` forum threads.
- Manual extraction via memory dump / executable scan (guides like `Cracko298/UE4-AES-Key-Extracting-Guide`). [Source: https://github.com/Cracko298/UE4-AES-Key-Extracting-Guide]

For shrinkray this means: **encrypted paks must be either (a) skipped, (b) processed only when the user supplies a key, or (c) looked up against a bundled key DB.** Bundling a third-party key DB is legally grey and per-game; shrinkray should default to **skip + warn**, with an optional "paste AES key" field in the UI.

## 6. The `repak` crate (May 2026 snapshot)

- **Repo:** https://github.com/trumank/repak — 499 stars, 64 forks, 13 open issues, last push 2026-02-20, last updated 2026-05-13. **Actively maintained.** [Source: `gh api repos/trumank/repak`]
- **Latest release:** v0.2.3, January 2026. [Source: https://github.com/trumank/repak]
- **License:** dual MIT / Apache-2.0 — fully compatible with shrinkray.
- **Capabilities:** read pak v1–11; write v2–9 and v11; read Zlib/Gzip/Zstd/Oodle; read AES-encrypted index and data; CLI binary and library API.
- **Cannot do:** write compression of any kind, write encryption, anything IoStore (use `retoc` for that), distinguish v11 pre-4.27 vs v11 4.27+ from the header alone.
- **Oodle:** opt-in `oodle` feature flag (default ON in `repak_cli`, default OFF in the library); uses `oodle_loader` to fetch the DLL at runtime from Epic.
- Note: the `docs.rs/repak` page still shows a stale `0.1.0` placeholder — trust the GitHub releases page, not docs.rs, for the real version.

## 7. Phase 1 recommendation

**Verdict: deprioritize pak-repack as the first feature. Move texture and audio repack ahead of it.**

Reasoning:
1. **Write compression is unimplemented in repak.** Even a "perfect" repak Phase 1 today would emit uncompressed paks, which is worse than what shipped. That alone disqualifies pak-repack as a savings feature in v1.
2. **Even if we wait for repak write-Zstd, Zstd loses to Kraken on game data.** Modern UE5 titles already use Kraken, so re-emitting at Zstd is a regression, not a win.
3. **Free Oodle write is not available.** `oodle_loader` enables *reading* Kraken via Epic's runtime DLL; redistributing the encoder in shrinkray is not licensed, and there is no widely accepted free Kraken *encoder*.
4. **IoStore is the actual surface area for UE5 games.** Most install bytes live in `.ucas`, which `repak` does not touch. Pak-repack on a UE5 title would skip 80%+ of bytes.
5. **Real install-size wins are upstream of the container:** **texture recompression (BC1/BC3/BC7 with mip culling), audio re-encode (Vorbis/Opus at lower bitrate), and pak/IoStore trimming (delete unused languages, dev shaders, PDBs)** dwarf any codec swap inside the pak.

**Suggested Phase 1 ordering for shrinkray:**
1. **Scan + report** (no writes): walk the install, classify entries by type and size, surface the top 50 fattest assets and identify languages / unused platform shader caches. This is a pure win, zero risk, demoable, and unblocks every later phase.
2. **Texture repack** (BC7 / mip-cull on oversized albedo+normal). This is where the real percentage savings live.
3. **Audio repack** (lower-bitrate Vorbis/Opus for non-critical SFX).
4. **Pak / IoStore *trimming*** via `repak` (read) + `retoc` (read) — *delete* unused entries and re-emit. Even uncompressed re-emit is a win if we removed 30% of entries.
5. **Pak/IoStore recompression** — defer until either `repak` ships write-Zstd *and* Epic relaxes Oodle redistribution, or until `oodle_loader` exposes encode. Until then, this stage doesn't earn its complexity.

Hard rule: shrinkray should **never recompress with a worse codec than the source** without an explicit user opt-in, and the UI should show predicted delta before any write.
