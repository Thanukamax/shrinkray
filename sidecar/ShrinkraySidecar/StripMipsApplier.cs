using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Pak;
using CUE4Parse.UE4.Versions;
using UAssetAPI;
using UAssetAPI.UnrealTypes;

namespace Shrinkray.Sidecar;

// apply_strip_mips — v0.6 write-side. Takes a pak path + list of asset paths
// with per-asset target max-dim, plus the engine version. For each target:
//
//   1. Extract the .uasset/.uexp/.ubulk triple from the pak via
//      `provider.SavePackage(path)` (CUE4Parse helper that finds the .uexp
//      and .ubulk siblings of a package).
//   2. Write the triple to a tempdir so we can hand UAssetAPI a path
//      (UAssetAPI 1.1.0 doesn't expose a stream constructor that takes the
//      bulk-data sibling streams, so disk is the simplest interop).
//   3. Open via `new UAsset(uassetPath, engineVer)`. UAssetAPI loads the
//      Texture2D export as a raw export whose Extras byte buffer contains
//      the FTexturePlatformData blob — we surgically rewrite that buffer.
//   4. Locate the FTexturePlatformData header within Extras (3×int32
//      preamble + SizeX + SizeY + NumSlices + FString PixelFormat +
//      FirstMip + NumMips + FTexture2DMipMap[NumMips]).
//   5. Walk the mip array, recording each mip's byte range, BulkData flags,
//      ElementCount, SizeOnDisk, OffsetInFile. We need this to splice out
//      the top mips and rebuild the .ubulk file.
//   6. Determine drop count by reusing the planner's formula
//      (firstKept = first mip with max(w,h) <= max_dim).
//   7. Splice FTexture2DMipMap[0..firstKept] out of Extras. Update
//      NumMips (subtract drop count) and SizeX/SizeY (shift right by drop
//      count). Patch each surviving mip's OffsetInFile field to point
//      at its new position in the shortened .ubulk.
//   8. Generate the new .ubulk by concatenating only the surviving mips'
//      payload bytes (read from the original .ubulk at the original
//      OffsetInFile positions).
//   9. Write the modified UAsset back via `asset.Write(out uasset, out uexp)`
//      and write the new .ubulk file separately. Read all three back as
//      byte buffers and return them keyed by their pak-relative path so
//      the Rust side can substitute them in the pak rewrite.
//
// Scope for v0.6:
//   * .ubulk-external case only (cooked games typical). Inline-payload mips
//     are passed through unchanged (we report them as "skipped: inline payload"
//     in the result so the user knows).
//   * 2D textures only — TextureCube / Volume / Array deferred to v0.7+.
//   * Encrypted paks are not supported (matches v0.5 planner).
//
// What the caller (Rust) does with the result:
//   * Substitute the returned bytes into the pak via repak::PakBuilder, all
//     other entries copied through unchanged.
//   * Record the original triple bytes in `backup.rs` via record_pak_entry_replace
//     so `shrinkray restore` can re-inject them later.

public sealed record StripTarget(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("max_dim")] int MaxDim);

public sealed record StripAppliedFile(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("bytes_base64")] string BytesBase64);

public sealed record StripAppliedTexture(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("export_name")] string ExportName,
    [property: JsonPropertyName("drop_mip_count")] int DropMipCount,
    [property: JsonPropertyName("kept_mip_count")] int KeptMipCount,
    [property: JsonPropertyName("original_top_dim")] int OriginalTopDim,
    [property: JsonPropertyName("kept_top_dim")] int KeptTopDim,
    [property: JsonPropertyName("saved_bytes")] long SavedBytes,
    [property: JsonPropertyName("pixel_format")] string PixelFormat,
    [property: JsonPropertyName("compression_settings")] string? CompressionSettings,
    [property: JsonPropertyName("files")] IReadOnlyList<StripAppliedFile> Files,
    [property: JsonPropertyName("original_files")] IReadOnlyList<StripAppliedFile> OriginalFiles);

public sealed record StripSkipped(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("reason")] string Reason);

public sealed record ApplyStripMipsResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("engine_version")] string EngineVersion,
    [property: JsonPropertyName("applied")] IReadOnlyList<StripAppliedTexture> Applied,
    [property: JsonPropertyName("skipped")] IReadOnlyList<StripSkipped> Skipped,
    [property: JsonPropertyName("total_saved_bytes")] long TotalSavedBytes);

public static class StripMipsApplier
{
    public static ApplyStripMipsResult Apply(
        string pakPath,
        IReadOnlyList<StripTarget> targets,
        EGame game,
        EngineVersion engineVer)
    {
        if (!File.Exists(pakPath))
            throw new FileNotFoundException($"pak not found: {pakPath}");
        if (targets.Count == 0)
        {
            return new ApplyStripMipsResult(pakPath, engineVer.ToString(),
                Array.Empty<StripAppliedTexture>(), Array.Empty<StripSkipped>(), 0);
        }

        var pakDir = Path.GetDirectoryName(Path.GetFullPath(pakPath))
            ?? throw new InvalidOperationException("pak has no parent directory");
        var versions = new VersionContainer(game);

        // We use DefaultFileProvider so SavePackage works (it walks the
        // provider's pak-mounted file tree to find .uexp/.ubulk siblings).
        using var provider = new DefaultFileProvider(
            new DirectoryInfo(pakDir), SearchOption.TopDirectoryOnly, versions);
        provider.Initialize();
        provider.MountAsync().GetAwaiter().GetResult();
        provider.PostMount();
        provider.LoadVirtualPaths();

        var tempRoot = Path.Combine(Path.GetTempPath(),
            "shrinkray-apply-" + Guid.NewGuid().ToString("N").Substring(0, 12));
        Directory.CreateDirectory(tempRoot);

        var applied = new List<StripAppliedTexture>();
        var skipped = new List<StripSkipped>();
        long totalSaved = 0;

        try
        {
            foreach (var target in targets)
            {
                try
                {
                    var result = ApplyOne(provider, target, engineVer, tempRoot);
                    if (result.applied is not null)
                    {
                        applied.Add(result.applied);
                        totalSaved += result.applied.SavedBytes;
                    }
                    else if (result.skipped is not null)
                    {
                        skipped.Add(result.skipped);
                    }
                }
                catch (Exception ex)
                {
                    Console.Error.WriteLine($"[apply_strip_mips] {target.AssetPath} failed: {ex.GetType().Name}: {ex.Message}");
                    Console.Error.WriteLine($"  stack: {ex.StackTrace?.Split('\n').FirstOrDefault()?.Trim()}");
                    skipped.Add(new StripSkipped(target.AssetPath, $"{ex.GetType().Name}: {ex.Message}"));
                }
            }
        }
        finally
        {
            try { Directory.Delete(tempRoot, recursive: true); } catch { /* ignore */ }
        }

        return new ApplyStripMipsResult(
            PakPath: pakPath,
            EngineVersion: engineVer.ToString(),
            Applied: applied,
            Skipped: skipped,
            TotalSavedBytes: totalSaved);
    }

    private static (StripAppliedTexture? applied, StripSkipped? skipped) ApplyOne(
        DefaultFileProvider provider,
        StripTarget target,
        EngineVersion engineVer,
        string tempRoot)
    {
        if (!provider.Files.TryGetValue(target.AssetPath, out var gameFile))
            return (null, new StripSkipped(target.AssetPath, "asset not found in pak"));

        // SavePackage returns dict<full_path, bytes> for .uasset + .uexp + .ubulk.
        // Keys are full pak-relative paths.
        var pkgBytes = provider.SavePackage(gameFile);
        if (pkgBytes.Count == 0)
            return (null, new StripSkipped(target.AssetPath, "no package bytes"));

        // Mirror the triple into a tempdir under the same relative layout
        // so UAssetAPI can find the .uexp/.ubulk siblings by stripping the
        // extension.
        var tempPkgDir = Path.Combine(tempRoot, "pkg-" + Guid.NewGuid().ToString("N").Substring(0, 8));
        Directory.CreateDirectory(tempPkgDir);
        string? uassetTempPath = null;
        string? ubulkTempPath = null;
        var pakToTempMap = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var originalFiles = new List<StripAppliedFile>();
        foreach (var (pakPath, bytes) in pkgBytes)
        {
            var leaf = Path.GetFileName(pakPath);
            var tempPath = Path.Combine(tempPkgDir, leaf);
            File.WriteAllBytes(tempPath, bytes);
            pakToTempMap[pakPath] = tempPath;
            originalFiles.Add(new StripAppliedFile(pakPath, Convert.ToBase64String(bytes)));
            if (leaf.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase))
                uassetTempPath = tempPath;
            else if (leaf.EndsWith(".ubulk", StringComparison.OrdinalIgnoreCase))
                ubulkTempPath = tempPath;
        }
        if (uassetTempPath is null)
            return (null, new StripSkipped(target.AssetPath, "no .uasset in package"));

        UAsset asset;
        try { asset = new UAsset(uassetTempPath, engineVer); }
        catch (Exception ex)
        {
            return (null, new StripSkipped(target.AssetPath, $"UAssetAPI load failed: {ex.Message}"));
        }

        // Find the Texture2D export. Same heuristic as the probe.
        int texIdx = -1;
        UAssetAPI.ExportTypes.Export? tex = null;
        for (int i = 0; i < asset.Exports.Count; i++)
        {
            var e = asset.Exports[i];
            var className = e.GetExportClassType().ToString();
            if (className.Contains("Texture2D"))
            {
                texIdx = i; tex = e; break;
            }
        }
        if (tex is null) return (null, new StripSkipped(target.AssetPath, "no Texture2D export"));

        var extras = tex.Extras;
        if (extras is null || extras.Length < 32)
            return (null, new StripSkipped(target.AssetPath, "Extras too short"));

        if (!TryParsePlatformData(extras, out var parsed, out var parseError))
        {
            // Dump first 256 bytes in 16-byte rows for binary triage.
            var dumpLen = Math.Min(256, extras.Length);
            Console.Error.WriteLine($"[apply_strip_mips] {target.AssetPath} parse failed: {parseError}");
            Console.Error.WriteLine($"  extras.Length={extras.Length}");
            for (int row = 0; row < dumpLen; row += 16)
            {
                int rowEnd = Math.Min(row + 16, dumpLen);
                var hex = new System.Text.StringBuilder();
                var ascii = new System.Text.StringBuilder();
                for (int i = row; i < rowEnd; i++)
                {
                    hex.Append(extras[i].ToString("x2")).Append(' ');
                    ascii.Append(extras[i] >= 32 && extras[i] < 127 ? (char)extras[i] : '.');
                }
                Console.Error.WriteLine($"  {row:x3}: {hex,-48} {ascii}");
            }
            return (null, new StripSkipped(target.AssetPath,
                $"FTexturePlatformData parse failed: {parseError} (extras.Length={extras.Length})"));
        }

        // How many top mips to drop = first mip index whose max(w,h) <= max_dim.
        int firstKept = parsed.Mips.Count;
        for (int i = 0; i < parsed.Mips.Count; i++)
        {
            int w = parsed.Mips[i].MipWidth;
            int h = parsed.Mips[i].MipHeight;
            if (Math.Max(w, h) <= target.MaxDim) { firstKept = i; break; }
        }
        if (firstKept <= 0)
            return (null, new StripSkipped(target.AssetPath, "already at or below target dim"));
        if (firstKept >= parsed.Mips.Count)
            return (null, new StripSkipped(target.AssetPath, "no mip small enough for target dim"));

        // Inline-payload mips are now supported: the splice preserves their
        // byte-exact payload in-band, and their OffsetInFile is informational
        // (CUE4Parse's GetBulkArchive doesn't use it for inline reads).
        // We only ever DROP top mips (which are ubulk-stored — large enough to
        // not be inlined), so the inline-payload tail rides through untouched.

        // Compute savings (bytes saved = sum of dropped mips' SizeOnDisk).
        long savedBytes = 0;
        for (int i = 0; i < firstKept; i++) savedBytes += parsed.Mips[i].SizeOnDisk;

        // Build the new Extras buffer: keep everything up to the start of
        // the mip array, write modified header fields, then keep the
        // surviving mips with patched offsets.
        var newExtras = SpliceMipsAndPatchHeader(extras, parsed, firstKept,
            out var newMipOffsetsInUbulk);

        // Generate the new .ubulk = concatenated payloads of surviving mips,
        // sourced from the original .ubulk at the original OffsetInFile positions.
        byte[]? newUbulkBytes = null;
        if (ubulkTempPath is not null && File.Exists(ubulkTempPath))
        {
            var origUbulk = File.ReadAllBytes(ubulkTempPath);
            using var ms = new MemoryStream();
            for (int i = firstKept; i < parsed.Mips.Count; i++)
            {
                var mip = parsed.Mips[i];
                if (mip.IsInlinePayload) continue;
                // OffsetInFile in the cook indexes into the .ubulk; copy
                // mip.SizeOnDisk bytes from there.
                long off = mip.OffsetInFile;
                long sz = mip.SizeOnDisk;
                if (off < 0 || off + sz > origUbulk.Length)
                {
                    // Defensive — bail without modifying anything.
                    return (null, new StripSkipped(target.AssetPath,
                        $"ubulk offset/size out of range for mip {i}: off={off} size={sz} ubulk_len={origUbulk.Length}"));
                }
                ms.Write(origUbulk, (int)off, (int)sz);
            }
            newUbulkBytes = ms.ToArray();
        }

        // Replace Extras and rewrite. UAssetAPI's Write() emits uasset+uexp;
        // .ubulk we write ourselves above.
        tex.Extras = newExtras;
        asset.Write(out var newUasset, out var newUexp);

        // Read the modified files back and pair them with their pak-relative
        // paths.
        var modifiedFiles = new List<StripAppliedFile>();
        foreach (var (pakPath, tempPath) in pakToTempMap)
        {
            var leaf = Path.GetFileName(pakPath);
            byte[] bytes;
            if (leaf.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase))
            {
                bytes = newUasset.ToArray();
            }
            else if (leaf.EndsWith(".uexp", StringComparison.OrdinalIgnoreCase))
            {
                bytes = (newUexp ?? new MemoryStream()).ToArray();
            }
            else if (leaf.EndsWith(".ubulk", StringComparison.OrdinalIgnoreCase))
            {
                if (newUbulkBytes is null)
                    continue; // shouldn't happen if we got here
                bytes = newUbulkBytes;
            }
            else
            {
                continue; // ignore extras (.uptnl etc) — pass-through in pak rewrite
            }
            modifiedFiles.Add(new StripAppliedFile(pakPath, Convert.ToBase64String(bytes)));
        }

        int originalTop = Math.Max(parsed.SizeX, parsed.SizeY);
        int keptTop = Math.Max(parsed.Mips[firstKept].MipWidth, parsed.Mips[firstKept].MipHeight);

        return (new StripAppliedTexture(
            AssetPath: target.AssetPath,
            ExportName: tex.ObjectName.ToString(),
            DropMipCount: firstKept,
            KeptMipCount: parsed.Mips.Count - firstKept,
            OriginalTopDim: originalTop,
            KeptTopDim: keptTop,
            SavedBytes: savedBytes,
            PixelFormat: parsed.PixelFormat,
            CompressionSettings: ExtractCompressionSettings(tex),
            Files: modifiedFiles,
            OriginalFiles: originalFiles), null);
    }

    /// <summary>
    /// Pulls the `CompressionSettings` UPROPERTY off a Texture2D export when
    /// the cook serialized it. UE writes it as either BytePropertyData (older
    /// FName-encoded byte enum) or EnumPropertyData (newer FName-keyed enum).
    /// Returns null when the property isn't present (uncommon — most cooks
    /// emit it even when the value is TC_Default).
    /// </summary>
    private static string? ExtractCompressionSettings(UAssetAPI.ExportTypes.Export tex)
    {
        // Only NormalExport carries tagged properties as a typed list. Other
        // export shapes (RawExport / FunctionExport) don't get classified.
        if (tex is not UAssetAPI.ExportTypes.NormalExport normal) return null;
        foreach (var prop in normal.Data)
        {
            var propName = prop?.Name?.Value?.Value;
            if (propName != "CompressionSettings") continue;
            switch (prop)
            {
                case UAssetAPI.PropertyTypes.Objects.BytePropertyData bp:
                    return bp.EnumValue?.Value?.Value
                        ?? (bp.Value != 0 ? bp.Value.ToString() : null);
                case UAssetAPI.PropertyTypes.Objects.EnumPropertyData ep:
                    return ep.Value?.Value?.Value;
                case UAssetAPI.PropertyTypes.Objects.NamePropertyData np:
                    return np.Value?.Value?.Value;
                default:
                    return prop?.PropertyType?.Value;
            }
        }
        // Cook didn't emit it — default TC_Default by UE convention.
        return null;
    }

    /// <summary>
    /// Try to locate the FTexturePlatformData header within an Extras byte
    /// buffer. Returns the parsed structure including per-mip byte ranges.
    /// Mirrors the heuristic from <see cref="TextureBytesProbe"/> for the
    /// initial header sniff, then walks the mip array deterministically.
    /// </summary>
    internal static bool TryParsePlatformData(byte[] extras, out ParsedPlatformData parsed, out string error)
    {
        parsed = default!;
        error = "";
        int? headerOffset = null;
        // Scan first 128 bytes for a plausible SizeX-prefixed header. UE4.13-era
        // cooks land at offset 13 (1 byte bCooked + 12 byte 3xint32 preamble).
        // Newer UE5 cooks vary, so we scan a wider window.
        for (int probe = 0; probe + 32 < extras.Length && probe <= 128; probe += 1)
        {
            if (LooksLikeHeader(extras, probe))
            {
                headerOffset = probe;
                break;
            }
        }
        if (headerOffset is null) { error = "no FTexturePlatformData header found in first 128 bytes"; return false; }

        try
        {
            int off = headerOffset.Value;

            int sizeX = ReadI32(extras, ref off, out error); if (error != "") return false;
            int sizeY = ReadI32(extras, ref off, out error); if (error != "") return false;
            int numSlices = ReadI32(extras, ref off, out error); if (error != "") return false;

            // PixelFormat FString — length prefix, then bytes (length includes null
            // terminator for positive-length ASCII strings).
            int fsLen = ReadI32(extras, ref off, out error); if (error != "") return false;
            if (fsLen < 0 || fsLen > 64) { error = $"FString len out of range: {fsLen}"; return false; }
            string pixelFormat = "";
            if (fsLen > 0)
            {
                if (off + fsLen > extras.Length) { error = $"FString overruns extras: off={off} len={fsLen} extras={extras.Length}"; return false; }
                pixelFormat = System.Text.Encoding.ASCII.GetString(extras, off, fsLen).TrimEnd('\0');
                off += fsLen;
            }

            int firstMipToSerialize = ReadI32(extras, ref off, out error); if (error != "") return false;
            int numMipsOffset = off;
            int numMips = ReadI32(extras, ref off, out error); if (error != "") return false;
            if (numMips < 1 || numMips > 16) { error = $"NumMips out of range: {numMips}"; return false; }

            // One-time `bCooked` sentinel between NumMips and the mip array.
            // Empirically observed on Pamali (UE4.22): a single int32 (value 1)
            // is emitted ONCE, not per-mip. CUE4Parse master reads it per-mip
            // and throws on mip 1+ where the 4-byte slot is actually the next
            // mip's BulkDataFlags (0x0501 for ubulk-stored mips, 0x48 for
            // inline mips) — neither of which is a valid bool. We preserve
            // this byte on write-back so the cook stays bit-identical for
            // the non-stripped mips.
            int cookedSentinelOffset = off;
            int cookedSentinel = ReadI32(extras, ref off, out error); if (error != "") return false;

            var mips = new List<ParsedMip>(numMips);
            int mipsArrayStart = off;
            for (int i = 0; i < numMips; i++)
            {
                int mipEntryStart = off;
                // Per-mip layout (32 bytes + optional inline payload):
                //   1. FByteBulkDataHeader: int32 flags + int32 elementCount
                //      + uint32 sizeOnDisk + int64 offsetInFile = 20 bytes
                //      (BULKDATA_Size64Bit assumed unset, which is the common case).
                //   2. If BULKDATA_ForceInlinePayload is set, SizeOnDisk bytes
                //      of inline payload follow directly (small mips go in-band
                //      in the .uexp, not in .ubulk).
                //   3. int32 SizeX, SizeY, SizeZ (always for UE4.20+).
                //
                // No per-mip cooked byte — that lives in the one-time sentinel
                // above. The legacy comment about cooked-per-mip was based on
                // CUE4Parse master's interpretation, which doesn't match the
                // empirical UE4.22 Pamali layout.
                if (off + 20 > extras.Length) { error = $"FByteBulkData header truncated at mip {i} (off={off}, extras={extras.Length})"; return false; }
                int bulkFlags = ReadI32(extras, ref off, out error); if (error != "") return false;

                // Defensive: BULKDATA_Size64Bit (1<<13) flips ElementCount /
                // SizeOnDisk to int64. We bail rather than try to handle 64-bit
                // sizes for v0.6 — none of Pamali's textures hit this.
                const int BULKDATA_Size64Bit = 1 << 13;
                if ((bulkFlags & BULKDATA_Size64Bit) != 0)
                {
                    error = $"mip {i} uses BULKDATA_Size64Bit, deferred to a later v0.6 patch";
                    return false;
                }

                int elementCount = ReadI32(extras, ref off, out error); if (error != "") return false;
                int sizeOnDisk = (int)ReadU32(extras, ref off, out error); if (error != "") return false;
                long offsetInFile = ReadI64(extras, ref off, out error); if (error != "") return false;

                const int BULKDATA_ForceInlinePayload = 0x40;
                const int BULKDATA_PayloadInSeperateFile = 0x100;
                bool inline = (bulkFlags & BULKDATA_ForceInlinePayload) != 0;
                bool inUbulk = (bulkFlags & BULKDATA_PayloadInSeperateFile) != 0;

                int inlinePayloadOffset = -1;
                if (inline)
                {
                    if (sizeOnDisk < 0 || off + sizeOnDisk > extras.Length)
                    {
                        error = $"inline payload truncated at mip {i} (sizeOnDisk={sizeOnDisk}, off={off}, extras={extras.Length})";
                        return false;
                    }
                    inlinePayloadOffset = off;
                    off += sizeOnDisk;
                }

                if (off + 12 > extras.Length) { error = $"mip dims truncated at mip {i} (off={off}, extras={extras.Length})"; return false; }
                int mipW = ReadI32(extras, ref off, out error); if (error != "") return false;
                int mipH = ReadI32(extras, ref off, out error); if (error != "") return false;
                int mipZ = ReadI32(extras, ref off, out error); if (error != "") return false;
                int mipEntryEnd = off;

                mips.Add(new ParsedMip(
                    BulkDataFlags: bulkFlags,
                    ElementCount: elementCount,
                    SizeOnDisk: sizeOnDisk,
                    OffsetInFile: offsetInFile,
                    IsInlinePayload: inline,
                    IsInSeparateFile: inUbulk,
                    InlinePayloadOffset: inlinePayloadOffset,
                    MipWidth: mipW,
                    MipHeight: mipH,
                    MipDepth: mipZ,
                    EntryStart: mipEntryStart,
                    EntryEnd: mipEntryEnd));
            }

            parsed = new ParsedPlatformData
            {
                HeaderOffset = headerOffset.Value,
                SizeX = sizeX,
                SizeY = sizeY,
                NumSlices = numSlices,
                PixelFormat = pixelFormat,
                FirstMipToSerialize = firstMipToSerialize,
                NumMipsOffset = numMipsOffset,
                NumMips = numMips,
                CookedSentinelOffset = cookedSentinelOffset,
                CookedSentinel = cookedSentinel,
                MipsArrayStart = mipsArrayStart,
                MipsArrayEnd = off,
                Mips = mips,
            };
            return true;
        }
        catch (Exception ex)
        {
            error = $"unhandled exception during parse: {ex.GetType().Name}: {ex.Message}";
            return false;
        }
    }

    private static int ReadI32(byte[] buf, ref int off, out string error)
    {
        if (off + 4 > buf.Length) { error = $"int32 read overruns at off={off}/{buf.Length}"; return 0; }
        var v = BitConverter.ToInt32(buf, off);
        off += 4;
        error = "";
        return v;
    }

    private static long ReadI64(byte[] buf, ref int off, out string error)
    {
        if (off + 8 > buf.Length) { error = $"int64 read overruns at off={off}/{buf.Length}"; return 0; }
        var v = BitConverter.ToInt64(buf, off);
        off += 8;
        error = "";
        return v;
    }

    private static uint ReadU32(byte[] buf, ref int off, out string error)
    {
        if (off + 4 > buf.Length) { error = $"uint32 read overruns at off={off}/{buf.Length}"; return 0; }
        var v = BitConverter.ToUInt32(buf, off);
        off += 4;
        error = "";
        return v;
    }

    private static bool LooksLikeHeader(byte[] extras, int probe)
    {
        if (probe + 32 > extras.Length) return false;
        int sx = BitConverter.ToInt32(extras, probe);
        int sy = BitConverter.ToInt32(extras, probe + 4);
        int slices = BitConverter.ToInt32(extras, probe + 8);
        if (sx < 1 || sx > 16384) return false;
        if (sy < 1 || sy > 16384) return false;
        if (slices < 1 || slices > 32) return false;
        // Power-of-two is a strong signal but not strictly required.
        int fsLen = BitConverter.ToInt32(extras, probe + 12);
        if (fsLen < 4 || fsLen > 64) return false;
        if (probe + 16 + fsLen > extras.Length) return false;
        var pf = System.Text.Encoding.ASCII.GetString(extras, probe + 16, fsLen).TrimEnd('\0');
        if (!pf.StartsWith("PF_")) return false;
        return true;
    }

    /// <summary>
    /// Build a new Extras buffer with: the original bytes up to the start of
    /// the mip array, the surviving mip entries (with their OffsetInFile patched
    /// to point into the new .ubulk), and any tail bytes after the mip array.
    /// Also updates SizeX / SizeY (shifted right by drop count) and NumMips
    /// in the header in-place via a small copy.
    /// </summary>
    private static byte[] SpliceMipsAndPatchHeader(
        byte[] extras,
        ParsedPlatformData pd,
        int firstKept,
        out List<long> newOffsetsInUbulk)
    {
        // 1. Copy everything before the mip array, then patch SizeX, SizeY,
        //    NumMips fields in the header.
        // 2. Compute new offsets — each surviving mip in the new .ubulk starts
        //    at the cumulative size of preceding surviving non-inline mips.
        // 3. For each surviving mip, write its FByteBulkData with patched
        //    offsets, plus its mip dims.
        // 4. Copy any bytes after the original mip array (Texture2D often has
        //    a few trailing fields like virtual textures / streaming pool, but
        //    those are post-mip-array and don't move).

        var out_ = new MemoryStream(extras.Length);

        // Step 1: pre-mip-array prefix.
        out_.Write(extras, 0, pd.MipsArrayStart);

        // Patch SizeX, SizeY, NumMips in the just-written prefix.
        var buf = out_.GetBuffer(); // mutable view of MemoryStream's backing buffer
        int keptCount = pd.Mips.Count - firstKept;
        int newSizeX = pd.Mips[firstKept].MipWidth;
        int newSizeY = pd.Mips[firstKept].MipHeight;
        // SizeX is the int32 starting at pd.HeaderOffset.
        WriteInt32(buf, pd.HeaderOffset + 0, newSizeX);
        WriteInt32(buf, pd.HeaderOffset + 4, newSizeY);
        WriteInt32(buf, pd.NumMipsOffset, keptCount);

        // Step 2 + 3: surviving mips with patched offsets.
        // Mirrors the parser layout exactly (no per-mip cooked byte — that
        // lives once in the prefix's CookedSentinel which we've already copied):
        //   FByteBulkData header (20 bytes) | optional inline payload | mip W | mip H | mip Z
        newOffsetsInUbulk = new List<long>(keptCount);
        long cursor = 0;
        for (int i = firstKept; i < pd.Mips.Count; i++)
        {
            var m = pd.Mips[i];
            long newOffset = m.IsInlinePayload ? m.OffsetInFile : cursor;
            newOffsetsInUbulk.Add(newOffset);
            if (!m.IsInlinePayload) cursor += m.SizeOnDisk;

            WriteInt32Stream(out_, m.BulkDataFlags);
            WriteInt32Stream(out_, m.ElementCount);
            WriteInt32Stream(out_, m.SizeOnDisk);
            WriteInt64Stream(out_, newOffset);
            if (m.IsInlinePayload && m.InlinePayloadOffset >= 0)
            {
                out_.Write(extras, m.InlinePayloadOffset, m.SizeOnDisk);
            }
            WriteInt32Stream(out_, m.MipWidth);
            WriteInt32Stream(out_, m.MipHeight);
            WriteInt32Stream(out_, m.MipDepth);
        }

        // Step 4: post-mip-array tail.
        int tailLen = extras.Length - pd.MipsArrayEnd;
        if (tailLen > 0)
        {
            out_.Write(extras, pd.MipsArrayEnd, tailLen);
        }

        return out_.ToArray();
    }

    private static void WriteInt32(byte[] buf, int offset, int v)
    {
        buf[offset + 0] = (byte)(v & 0xFF);
        buf[offset + 1] = (byte)((v >> 8) & 0xFF);
        buf[offset + 2] = (byte)((v >> 16) & 0xFF);
        buf[offset + 3] = (byte)((v >> 24) & 0xFF);
    }
    private static void WriteInt32Stream(MemoryStream s, int v)
    {
        s.WriteByte((byte)(v & 0xFF));
        s.WriteByte((byte)((v >> 8) & 0xFF));
        s.WriteByte((byte)((v >> 16) & 0xFF));
        s.WriteByte((byte)((v >> 24) & 0xFF));
    }
    private static void WriteInt64Stream(MemoryStream s, long v)
    {
        for (int i = 0; i < 8; i++)
        {
            s.WriteByte((byte)((v >> (i * 8)) & 0xFF));
        }
    }

    internal sealed class ParsedPlatformData
    {
        public int HeaderOffset;
        public int SizeX;
        public int SizeY;
        public int NumSlices;
        public string PixelFormat = "";
        public int FirstMipToSerialize;
        public int NumMipsOffset;
        public int NumMips;
        public int CookedSentinelOffset;
        public int CookedSentinel;
        public int MipsArrayStart;
        public int MipsArrayEnd;
        public List<ParsedMip> Mips = new();
    }

    internal sealed record ParsedMip(
        int BulkDataFlags,
        int ElementCount,
        int SizeOnDisk,
        long OffsetInFile,
        bool IsInlinePayload,
        bool IsInSeparateFile,
        int InlinePayloadOffset,
        int MipWidth,
        int MipHeight,
        int MipDepth,
        int EntryStart,
        int EntryEnd);
}
