using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Pak;
using CUE4Parse.UE4.Versions;
using UAssetAPI;
using UAssetAPI.UnrealTypes;

namespace Shrinkray.Sidecar;

// apply_restore_mips — the inverse of apply_strip_mips (v0.6.0). v0.7.3 ships
// this primitive standalone; v0.7.4 wires it through the manifest-driven
// restore loop + the "Restore (AI)" UI button.
//
// Input model. For each texture being restored, the caller supplies the new
// top-mip BC-encoded bytes per mip level (already encoded by
// `shrinkray_core::bcn::encode_with_mips` upstream). The applier slots those
// in at the top of the existing FTexturePlatformData mip array, regenerates
// the .ubulk by concatenating new-mip bytes + the existing (post-strip)
// ubulk, patches SizeX/SizeY/NumMips, and returns the modified triple via
// the same StripAppliedFile shape that v0.6.0's strip path uses.
//
// What this is NOT:
//   * It does not run AI inference — Rust does that and hands us bytes.
//   * It does not BC-encode — Rust's bcn module does, and hands us bytes.
//   * It does not write back into the pak — Rust's texture_strip::apply_to_pak
//     does that, with the StripAppliedFile bytes we return here.
//
// What this IS: the .NET-side splice that knows the byte layout from v0.6.0
// (one-time cooked sentinel + 32-byte mips + inline-payload handling) and
// can produce a valid Extras + ubulk with new mips on top.

public sealed record RestoreMipLevel(
    [property: JsonPropertyName("w")] int Width,
    [property: JsonPropertyName("h")] int Height,
    [property: JsonPropertyName("bytes_base64")] string BytesBase64);

public sealed record RestoreTarget(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("new_top_mips")] IReadOnlyList<RestoreMipLevel> NewTopMips);

public sealed record RestoreAppliedTexture(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("export_name")] string ExportName,
    [property: JsonPropertyName("inserted_mip_count")] int InsertedMipCount,
    [property: JsonPropertyName("previous_top_dim")] int PreviousTopDim,
    [property: JsonPropertyName("new_top_dim")] int NewTopDim,
    [property: JsonPropertyName("inserted_bytes")] long InsertedBytes,
    [property: JsonPropertyName("files")] IReadOnlyList<StripAppliedFile> Files,
    [property: JsonPropertyName("original_files")] IReadOnlyList<StripAppliedFile> OriginalFiles);

public sealed record ApplyRestoreMipsResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("engine_version")] string EngineVersion,
    [property: JsonPropertyName("applied")] IReadOnlyList<RestoreAppliedTexture> Applied,
    [property: JsonPropertyName("skipped")] IReadOnlyList<StripSkipped> Skipped,
    [property: JsonPropertyName("total_inserted_bytes")] long TotalInsertedBytes);

public static class RestoreMipsApplier
{
    public static ApplyRestoreMipsResult Apply(
        string pakPath,
        IReadOnlyList<RestoreTarget> targets,
        EGame game,
        EngineVersion engineVer)
    {
        if (!File.Exists(pakPath))
            throw new FileNotFoundException($"pak not found: {pakPath}");
        if (targets.Count == 0)
        {
            return new ApplyRestoreMipsResult(pakPath, engineVer.ToString(),
                Array.Empty<RestoreAppliedTexture>(), Array.Empty<StripSkipped>(), 0);
        }

        var pakDir = Path.GetDirectoryName(Path.GetFullPath(pakPath))
            ?? throw new InvalidOperationException("pak has no parent directory");
        var versions = new VersionContainer(game);

        using var provider = new DefaultFileProvider(
            new DirectoryInfo(pakDir), SearchOption.TopDirectoryOnly, versions);
        provider.Initialize();
        provider.MountAsync().GetAwaiter().GetResult();
        provider.PostMount();
        provider.LoadVirtualPaths();

        var tempRoot = Path.Combine(Path.GetTempPath(),
            "shrinkray-restore-" + Guid.NewGuid().ToString("N").Substring(0, 12));
        Directory.CreateDirectory(tempRoot);

        var applied = new List<RestoreAppliedTexture>();
        var skipped = new List<StripSkipped>();
        long totalInserted = 0;

        try
        {
            int idx = 0;
            foreach (var target in targets)
            {
                idx++;
                string status; string? reason = null; long insertedBytes = 0;
                try
                {
                    var result = ApplyOne(provider, target, engineVer, tempRoot);
                    if (result.applied is not null)
                    {
                        applied.Add(result.applied);
                        totalInserted += result.applied.InsertedBytes;
                        insertedBytes = result.applied.InsertedBytes;
                        status = "applied";
                    }
                    else if (result.skipped is not null)
                    {
                        skipped.Add(result.skipped);
                        status = "skipped";
                        reason = result.skipped.Reason;
                    }
                    else
                    {
                        status = "skipped";
                        reason = "internal: ApplyOne returned no result";
                        skipped.Add(new StripSkipped(target.AssetPath, reason));
                    }
                }
                catch (Exception ex)
                {
                    Console.Error.WriteLine($"[apply_restore_mips] {target.AssetPath} failed: {ex.GetType().Name}: {ex.Message}");
                    Console.Error.WriteLine($"  stack: {ex.StackTrace?.Split('\n').FirstOrDefault()?.Trim()}");
                    status = "skipped";
                    reason = $"{ex.GetType().Name}: {ex.Message}";
                    skipped.Add(new StripSkipped(target.AssetPath, reason));
                }
                ProgressEmitter.Emit(new
                {
                    op = "apply_restore_mips",
                    current = idx,
                    total = targets.Count,
                    asset_path = target.AssetPath,
                    status,
                    inserted_bytes = insertedBytes,
                    reason,
                });
            }
        }
        finally
        {
            try { Directory.Delete(tempRoot, recursive: true); } catch { /* ignore */ }
        }

        return new ApplyRestoreMipsResult(
            PakPath: pakPath,
            EngineVersion: engineVer.ToString(),
            Applied: applied,
            Skipped: skipped,
            TotalInsertedBytes: totalInserted);
    }

    private static (RestoreAppliedTexture? applied, StripSkipped? skipped) ApplyOne(
        DefaultFileProvider provider,
        RestoreTarget target,
        EngineVersion engineVer,
        string tempRoot)
    {
        if (target.NewTopMips.Count == 0)
            return (null, new StripSkipped(target.AssetPath, "no new mips supplied"));

        if (!provider.Files.TryGetValue(target.AssetPath, out var gameFile))
            return (null, new StripSkipped(target.AssetPath, "asset not found in pak"));

        // Extract the triple from the (already-stripped) pak.
        var pkgBytes = provider.SavePackage(gameFile);
        if (pkgBytes.Count == 0)
            return (null, new StripSkipped(target.AssetPath, "no package bytes"));

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

        UAssetAPI.ExportTypes.Export? tex = null;
        for (int i = 0; i < asset.Exports.Count; i++)
        {
            var e = asset.Exports[i];
            if (e.GetExportClassType().ToString().Contains("Texture2D")) { tex = e; break; }
        }
        if (tex is null) return (null, new StripSkipped(target.AssetPath, "no Texture2D export"));

        var extras = tex.Extras;
        if (extras is null || extras.Length < 32)
            return (null, new StripSkipped(target.AssetPath, "Extras too short"));

        if (!StripMipsApplier.TryParsePlatformData(extras, out var parsed, out var parseError))
        {
            return (null, new StripSkipped(target.AssetPath,
                $"FTexturePlatformData parse failed: {parseError} (extras.Length={extras.Length})"));
        }

        // Validate new mips: they must be strictly larger than the existing
        // top mip (we're inserting at the top), each smaller than the next,
        // and ending one halving above the existing top mip dim.
        int existingTopW = parsed.Mips[0].MipWidth;
        int existingTopH = parsed.Mips[0].MipHeight;
        var newMips = target.NewTopMips;
        // Bottom-most new mip should be 2× the existing top mip — i.e. the new
        // chain should slot above without gaps.
        var bottomNew = newMips[^1];
        if (bottomNew.Width != existingTopW * 2 || bottomNew.Height != existingTopH * 2)
        {
            return (null, new StripSkipped(target.AssetPath,
                $"new mip chain doesn't slot above existing top: bottom new = {bottomNew.Width}x{bottomNew.Height}, existing top = {existingTopW}x{existingTopH} (expected bottom new = {existingTopW * 2}x{existingTopH * 2})"));
        }
        // Each new mip should be 2× the next (top → bottom halving).
        for (int i = 0; i < newMips.Count - 1; i++)
        {
            if (newMips[i].Width != newMips[i + 1].Width * 2 || newMips[i].Height != newMips[i + 1].Height * 2)
            {
                return (null, new StripSkipped(target.AssetPath,
                    $"new mip chain not strictly-halving: mip {i} = {newMips[i].Width}x{newMips[i].Height}, mip {i + 1} = {newMips[i + 1].Width}x{newMips[i + 1].Height}"));
            }
        }

        // Decode each new mip's base64 bytes upfront so byte-count validation
        // hits before we mutate anything.
        var newMipBytes = new byte[newMips.Count][];
        long totalNewBytes = 0;
        for (int i = 0; i < newMips.Count; i++)
        {
            try { newMipBytes[i] = Convert.FromBase64String(newMips[i].BytesBase64); }
            catch (Exception ex)
            {
                return (null, new StripSkipped(target.AssetPath,
                    $"new mip {i} base64 decode failed: {ex.Message}"));
            }
            totalNewBytes += newMipBytes[i].Length;
        }

        // Take the BulkDataFlags / template from the existing top mip — for
        // standard ubulk-stored cooks this is 0x0501 (PayloadAtEndOfFile |
        // PayloadInSeperateFile | Force_NOT_InlinePayload). Inserted mips
        // mirror that pattern so CUE4Parse reads them the same way.
        int templateBulkFlags = parsed.Mips[0].BulkDataFlags;
        if ((templateBulkFlags & (1 << 6)) != 0) // ForceInlinePayload
        {
            // Existing top mip is inline — that's unusual (means the whole
            // texture was tiny enough to inline). The new larger mips can't
            // be inline. We bail rather than mix layouts.
            return (null, new StripSkipped(target.AssetPath,
                $"existing top mip is inline (bulk flags 0x{templateBulkFlags:x4}); reverse splice for inline → ubulk transition not supported"));
        }

        var newExtras = SpliceRestoredMipsAndPatchHeader(
            extras, parsed, newMips, newMipBytes, templateBulkFlags,
            totalNewBytes, out var newMipUbulkOffsets);

        // Build the new .ubulk: concat new mip bytes + existing ubulk
        // (the existing ubulk contains the surviving lower mips, unmodified).
        byte[] newUbulkBytes;
        {
            using var ms = new MemoryStream();
            foreach (var bytes in newMipBytes)
            {
                ms.Write(bytes, 0, bytes.Length);
            }
            if (ubulkTempPath is not null && File.Exists(ubulkTempPath))
            {
                var existingUbulk = File.ReadAllBytes(ubulkTempPath);
                ms.Write(existingUbulk, 0, existingUbulk.Length);
            }
            newUbulkBytes = ms.ToArray();
        }

        tex.Extras = newExtras;
        asset.Write(out var newUasset, out var newUexp);

        var modifiedFiles = new List<StripAppliedFile>();
        bool sawUbulk = false;
        foreach (var (pakPath, _) in pakToTempMap)
        {
            var leaf = Path.GetFileName(pakPath);
            byte[] bytes;
            if (leaf.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase))
                bytes = newUasset.ToArray();
            else if (leaf.EndsWith(".uexp", StringComparison.OrdinalIgnoreCase))
                bytes = (newUexp ?? new MemoryStream()).ToArray();
            else if (leaf.EndsWith(".ubulk", StringComparison.OrdinalIgnoreCase))
            {
                bytes = newUbulkBytes;
                sawUbulk = true;
            }
            else
                continue;
            modifiedFiles.Add(new StripAppliedFile(pakPath, Convert.ToBase64String(bytes)));
        }
        // Edge case: the stripped pak might have no .ubulk entry (if every
        // surviving mip was inline). After restore we need one — synthesize
        // its pak-relative path from the .uasset.
        if (!sawUbulk)
        {
            var uassetPak = pakToTempMap.Keys.FirstOrDefault(p => p.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase));
            if (uassetPak is not null)
            {
                var ubulkPak = Path.ChangeExtension(uassetPak, ".ubulk");
                modifiedFiles.Add(new StripAppliedFile(ubulkPak, Convert.ToBase64String(newUbulkBytes)));
            }
        }

        int newTop = Math.Max(newMips[0].Width, newMips[0].Height);
        int prevTop = Math.Max(parsed.SizeX, parsed.SizeY);

        return (new RestoreAppliedTexture(
            AssetPath: target.AssetPath,
            ExportName: tex.ObjectName.ToString(),
            InsertedMipCount: newMips.Count,
            PreviousTopDim: prevTop,
            NewTopDim: newTop,
            InsertedBytes: totalNewBytes,
            Files: modifiedFiles,
            OriginalFiles: originalFiles), null);
    }

    /// <summary>
    /// Build a new Extras buffer with new top mips prepended. Layout mirrors
    /// the v0.6.0 strip splice exactly, just with the mip list reordered.
    /// Returns the per-new-mip OffsetInFile values used for diagnostics.
    /// </summary>
    private static byte[] SpliceRestoredMipsAndPatchHeader(
        byte[] extras,
        StripMipsApplier.ParsedPlatformData pd,
        IReadOnlyList<RestoreMipLevel> newMips,
        byte[][] newMipBytes,
        int templateBulkFlags,
        long totalNewBytes,
        out List<long> newMipUbulkOffsets)
    {
        var out_ = new MemoryStream(extras.Length + (int)totalNewBytes + newMips.Count * 32);

        // Step 1: pre-mip-array prefix. Patch SizeX/SizeY/NumMips to reflect
        // the restored top dims + the larger mip count.
        out_.Write(extras, 0, pd.MipsArrayStart);
        var buf = out_.GetBuffer();
        int newSizeX = newMips[0].Width;
        int newSizeY = newMips[0].Height;
        int keptCount = pd.Mips.Count + newMips.Count;
        WriteInt32(buf, pd.HeaderOffset + 0, newSizeX);
        WriteInt32(buf, pd.HeaderOffset + 4, newSizeY);
        WriteInt32(buf, pd.NumMipsOffset, keptCount);

        // Step 2: write the new top mips' entries first. Their OffsetInFile
        // values walk from 0 up through totalNewBytes in the new ubulk.
        newMipUbulkOffsets = new List<long>(newMips.Count);
        long cursor = 0;
        for (int i = 0; i < newMips.Count; i++)
        {
            long offset = cursor;
            newMipUbulkOffsets.Add(offset);
            cursor += newMipBytes[i].Length;

            WriteInt32Stream(out_, templateBulkFlags);   // BulkDataFlags
            WriteInt32Stream(out_, newMipBytes[i].Length); // ElementCount (ByteBulkData: bytes)
            WriteInt32Stream(out_, newMipBytes[i].Length); // SizeOnDisk
            WriteInt64Stream(out_, offset);                // OffsetInFile
            WriteInt32Stream(out_, newMips[i].Width);      // SizeX
            WriteInt32Stream(out_, newMips[i].Height);     // SizeY
            WriteInt32Stream(out_, 1);                     // SizeZ (2D texture)
        }

        // Step 3: write the existing mips with OffsetInFile shifted by
        // totalNewBytes (so they reference their new position in the
        // concatenated ubulk). Inline-payload mips keep their original
        // offset (offsets are informational for inline) and we copy the
        // inline payload bytes verbatim.
        foreach (var m in pd.Mips)
        {
            long newOffset = m.IsInlinePayload ? m.OffsetInFile : m.OffsetInFile + totalNewBytes;

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

        // Step 4: post-mip-array tail. The strip splice preserved this; we
        // preserve it byte-exact too.
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
}
