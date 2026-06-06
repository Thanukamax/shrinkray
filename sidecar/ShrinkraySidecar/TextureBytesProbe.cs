using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Versions;
using UAssetAPI;
using UAssetAPI.ExportTypes;
using UAssetAPI.UnrealTypes;

namespace Shrinkray.Sidecar;

// Recon command for the v0.6 write-side: open a single .uasset via UAssetAPI
// (which handles the .uexp + .ubulk siblings automatically), find the
// Texture2D export, and dump the byte boundary UAssetAPI hands us via
// Export.Extras. We need to know whether Extras starts at:
//   (a) right after the tagged-properties "None" sentinel (i.e. includes
//       FStripDataFlags + bCooked bool before FTexturePlatformData), or
//   (b) right after bCooked (matches the CUE4Parse byte dump from v0.5)
// The answer changes how the FTexturePlatformData parser must position
// itself within Extras.

public sealed record TextureBytesProbeMipPreview(
    [property: JsonPropertyName("flags_hex")] string FlagsHex,
    [property: JsonPropertyName("element_count")] long ElementCount,
    [property: JsonPropertyName("size_on_disk")] long SizeOnDisk,
    [property: JsonPropertyName("offset_in_file")] long OffsetInFile);

// One mip walked with the v0.6.0-final candidate layout (32 bytes per mip,
// no per-mip cooked flag; the cooked flag is a single int32 sentinel between
// NumMips and the mip array). The first 16 bytes are echoed back as hex so
// the caller can sanity-check the stride visually.
public sealed record MipWalkEntry(
    [property: JsonPropertyName("index")] int Index,
    [property: JsonPropertyName("offset_in_extras")] int OffsetInExtras,
    [property: JsonPropertyName("bulk_flags_hex")] string BulkFlagsHex,
    [property: JsonPropertyName("element_count")] long ElementCount,
    [property: JsonPropertyName("size_on_disk")] long SizeOnDisk,
    [property: JsonPropertyName("offset_in_file")] long OffsetInFile,
    [property: JsonPropertyName("size_x")] int SizeX,
    [property: JsonPropertyName("size_y")] int SizeY,
    [property: JsonPropertyName("size_z")] int SizeZ,
    [property: JsonPropertyName("first_bytes_hex")] string FirstBytesHex,
    [property: JsonPropertyName("dims_sane")] bool DimsSane);

public sealed record TextureBytesProbeResult(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("engine_version")] string EngineVersion,
    [property: JsonPropertyName("export_count")] int ExportCount,
    [property: JsonPropertyName("texture_export_index")] int? TextureExportIndex,
    [property: JsonPropertyName("texture_export_class")] string? TextureExportClass,
    [property: JsonPropertyName("extras_length")] int ExtrasLength,
    [property: JsonPropertyName("extras_first_64_hex")] string ExtrasFirst64Hex,
    [property: JsonPropertyName("extras_first_64_ascii")] string ExtrasFirst64Ascii,
    [property: JsonPropertyName("parsed_size_x")] int? ParsedSizeX,
    [property: JsonPropertyName("parsed_size_y")] int? ParsedSizeY,
    [property: JsonPropertyName("parsed_num_slices")] int? ParsedNumSlices,
    [property: JsonPropertyName("parsed_pixel_format")] string? ParsedPixelFormat,
    [property: JsonPropertyName("parsed_first_mip")] int? ParsedFirstMip,
    [property: JsonPropertyName("parsed_num_mips")] int? ParsedNumMips,
    [property: JsonPropertyName("parsed_extras_offset_to_size_x")] int? ParsedExtrasOffsetToSizeX,
    [property: JsonPropertyName("one_time_cooked_sentinel")] int? OneTimeCookedSentinel,
    [property: JsonPropertyName("mip_walk")] IReadOnlyList<MipWalkEntry>? MipWalk,
    [property: JsonPropertyName("mip_walk_end_offset")] int? MipWalkEndOffset,
    [property: JsonPropertyName("trailing_bytes_hex")] string? TrailingBytesHex,
    [property: JsonPropertyName("mip_walk_error")] string? MipWalkError);

public static class TextureBytesProbe
{
    // Probe modes:
    //   - assetPath points to a loose .uasset on disk → load directly.
    //   - assetPath is a pak-relative path AND pakPath is set → extract the
    //     .uasset/.uexp/.ubulk triple from the pak into a tempdir first, then
    //     load from there. Mirrors StripMipsApplier's extraction strategy so
    //     the probe walks the exact bytes the applier would mutate.
    public static TextureBytesProbeResult Probe(
        string assetPath,
        EngineVersion engineVer,
        bool walkMips = false,
        string? pakPath = null,
        EGame game = EGame.GAME_UE5_LATEST)
    {
        string? tempRoot = null;
        try
        {
            string uassetPath;
            if (!string.IsNullOrEmpty(pakPath))
            {
                if (!File.Exists(pakPath))
                    throw new FileNotFoundException($"pak not found: {pakPath}");
                var pakDir = Path.GetDirectoryName(Path.GetFullPath(pakPath))
                    ?? throw new InvalidOperationException("pak has no parent directory");
                using var provider = new DefaultFileProvider(
                    new DirectoryInfo(pakDir), SearchOption.TopDirectoryOnly,
                    new VersionContainer(game));
                provider.Initialize();
                provider.MountAsync().GetAwaiter().GetResult();
                provider.PostMount();
                provider.LoadVirtualPaths();
                if (!provider.Files.TryGetValue(assetPath, out var gameFile))
                    throw new FileNotFoundException($"asset not found in pak: {assetPath}");
                var pkgBytes = provider.SavePackage(gameFile);
                if (pkgBytes.Count == 0)
                    throw new InvalidOperationException("SavePackage returned no bytes");
                tempRoot = Path.Combine(Path.GetTempPath(),
                    "shrinkray-probe-" + Guid.NewGuid().ToString("N").Substring(0, 8));
                Directory.CreateDirectory(tempRoot);
                string? uassetOut = null;
                foreach (var (pkgRelPath, bytes) in pkgBytes)
                {
                    var leaf = Path.GetFileName(pkgRelPath);
                    var outPath = Path.Combine(tempRoot, leaf);
                    File.WriteAllBytes(outPath, bytes);
                    if (leaf.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase))
                        uassetOut = outPath;
                }
                uassetPath = uassetOut
                    ?? throw new InvalidOperationException("no .uasset in extracted package");
            }
            else
            {
                uassetPath = assetPath;
            }
            return ProbeImpl(assetPath, uassetPath, engineVer, walkMips);
        }
        finally
        {
            if (tempRoot is not null)
            {
                try { Directory.Delete(tempRoot, recursive: true); } catch { /* ignore */ }
            }
        }
    }

    private static TextureBytesProbeResult ProbeImpl(string reportedPath, string uassetPath, EngineVersion engineVer, bool walkMips)
    {
        var assetPath = reportedPath;
        var asset = new UAsset(uassetPath, engineVer);

        int? texIdx = null;
        string? texClass = null;
        Export? tex = null;
        for (int i = 0; i < asset.Exports.Count; i++)
        {
            var e = asset.Exports[i];
            var className = e.GetExportClassType().ToString();
            if (className.Contains("Texture2D") || className.Contains("TextureCube"))
            {
                texIdx = i;
                texClass = className;
                tex = e;
                break;
            }
        }

        if (tex is null)
        {
            return new TextureBytesProbeResult(
                AssetPath: assetPath,
                EngineVersion: engineVer.ToString(),
                ExportCount: asset.Exports.Count,
                TextureExportIndex: null,
                TextureExportClass: null,
                ExtrasLength: 0,
                ExtrasFirst64Hex: "",
                ExtrasFirst64Ascii: "",
                ParsedSizeX: null, ParsedSizeY: null, ParsedNumSlices: null,
                ParsedPixelFormat: null, ParsedFirstMip: null, ParsedNumMips: null,
                ParsedExtrasOffsetToSizeX: null,
                OneTimeCookedSentinel: null,
                MipWalk: null, MipWalkEndOffset: null, TrailingBytesHex: null,
                MipWalkError: "no texture export found");
        }

        var extras = tex.Extras ?? Array.Empty<byte>();
        var firstN = extras.Take(64).ToArray();
        var hex = string.Join(" ", firstN.Select(b => b.ToString("x2")));
        var ascii = new string(firstN.Select(b => b >= 32 && b < 127 ? (char)b : '.').ToArray());

        // Heuristic parse: scan Extras for the SizeX=SizeY=NumSlices=1 pattern
        // we observed at CUE4Parse position 168 (4 + 4 + 4 + format-string-len + 4 + 4).
        // CUE4Parse showed bytes starting with `0c 00 00 00 00 00 00 00 89 1a 00 00`
        // (3 × int32 preamble) then SizeX. We scan up to offset 32 for a plausible
        // SizeX (power-of-two between 4 and 16384) followed by an equal-or-similar SizeY.
        int? sizeX = null, sizeY = null, numSlices = null, firstMip = null, numMips = null;
        string? pixelFormat = null;
        int? offsetToSizeX = null;
        for (int probe = 0; probe + 32 < extras.Length && probe <= 32; probe += 4)
        {
            int candSx = BitConverter.ToInt32(extras, probe);
            int candSy = BitConverter.ToInt32(extras, probe + 4);
            int candSlices = BitConverter.ToInt32(extras, probe + 8);
            if (candSx < 1 || candSx > 16384) continue;
            if ((candSx & (candSx - 1)) != 0 && candSx != 1) continue; // not power-of-2
            if (candSy < 1 || candSy > 16384) continue;
            if (candSlices < 1 || candSlices > 32) continue;
            // Read FString length at probe+12 to validate format string follows
            int fsLen = BitConverter.ToInt32(extras, probe + 12);
            if (fsLen < 4 || fsLen > 64) continue;
            // Read the format string itself
            if (probe + 16 + fsLen > extras.Length) continue;
            var pf = System.Text.Encoding.ASCII.GetString(extras, probe + 16, fsLen).TrimEnd('\0');
            if (!pf.StartsWith("PF_")) continue;
            // Read FirstMip + NumMips after the string
            int firstMipPos = probe + 16 + fsLen;
            if (firstMipPos + 8 > extras.Length) continue;
            int candFirst = BitConverter.ToInt32(extras, firstMipPos);
            int candNum = BitConverter.ToInt32(extras, firstMipPos + 4);
            if (candNum < 1 || candNum > 16) continue;
            // All sanity checks passed — this is the FTexturePlatformData header.
            sizeX = candSx;
            sizeY = candSy;
            numSlices = candSlices;
            pixelFormat = pf;
            firstMip = candFirst;
            numMips = candNum;
            offsetToSizeX = probe;
            break;
        }

        // v0.6.0-final candidate layout walk. We learned empirically (Pamali
        // T_hairMask03) that the per-mip `bCooked` int32 that CUE4Parse
        // master reads in FTexture2DMipMap is NOT actually per-mip in older
        // UE4 cooks — it's a single int32 sentinel emitted once between
        // NumMips and the mip array. Each mip is then 32 bytes:
        //   int32 BulkDataFlags
        //   int32 ElementCount
        //   uint32 SizeOnDisk
        //   int64 OffsetInFile
        //   int32 SizeX, SizeY, SizeZ
        // Inline payload bytes (BULKDATA_ForceInlinePayload) are NOT
        // observed on Pamali, so this walker skips inline handling — if
        // we hit one in the wild we'll flag it for v0.6.x.
        int? sentinel = null;
        List<MipWalkEntry>? walk = null;
        int? walkEnd = null;
        string? trailing = null;
        string? walkError = null;
        if (walkMips && offsetToSizeX is int hdrOff && numMips is int mipCount && pixelFormat is string pfStr)
        {
            try
            {
                int pos = hdrOff
                    + 4  // SizeX
                    + 4  // SizeY
                    + 4  // PackedData / NumSlices
                    + 4 + pfStr.Length + 1  // FString (length + ASCII with null terminator)
                    + 4  // FirstMipToSerialize
                    + 4; // NumMips
                if (pos + 4 > extras.Length)
                    throw new InvalidOperationException($"extras too short for cooked sentinel: pos={pos} extras={extras.Length}");
                sentinel = BitConverter.ToInt32(extras, pos);
                pos += 4;

                walk = new List<MipWalkEntry>(mipCount);
                int prevW = 0;
                const int BULKDATA_ForceInlinePayload = 1 << 6;
                for (int i = 0; i < mipCount; i++)
                {
                    int mipStart = pos;
                    if (pos + 20 > extras.Length)
                        throw new InvalidOperationException($"mip {i} header truncated at pos={pos} extras={extras.Length}");
                    int bulkFlags = BitConverter.ToInt32(extras, pos); pos += 4;
                    int elementCount = BitConverter.ToInt32(extras, pos); pos += 4;
                    uint sizeOnDisk = BitConverter.ToUInt32(extras, pos); pos += 4;
                    long offsetInFile = BitConverter.ToInt64(extras, pos); pos += 8;
                    // Inline payload: small mips (typically 6+) get their bytes
                    // stored in-band, between the bulk header and the dim trailer.
                    bool inline = (bulkFlags & BULKDATA_ForceInlinePayload) != 0;
                    if (inline)
                    {
                        if (sizeOnDisk > int.MaxValue || pos + (int)sizeOnDisk > extras.Length)
                            throw new InvalidOperationException($"mip {i} inline payload truncated: size={sizeOnDisk} pos={pos} extras={extras.Length}");
                        pos += (int)sizeOnDisk;
                    }
                    if (pos + 12 > extras.Length)
                        throw new InvalidOperationException($"mip {i} dim trailer truncated at pos={pos} extras={extras.Length}");
                    int mw = BitConverter.ToInt32(extras, pos); pos += 4;
                    int mh = BitConverter.ToInt32(extras, pos); pos += 4;
                    int mz = BitConverter.ToInt32(extras, pos); pos += 4;
                    int firstLen = Math.Min(16, extras.Length - mipStart);
                    var firstHex = string.Join(" ", extras.Skip(mipStart).Take(firstLen).Select(b => b.ToString("x2")));
                    // Sanity: dims should halve from the previous mip (or match top-level dims for mip 0).
                    bool dimsSane = mw > 0 && mh > 0 && mw <= 16384 && mh <= 16384 && mz >= 1 && mz <= 32;
                    if (i > 0 && prevW > 0)
                    {
                        int expected = Math.Max(1, prevW / 2);
                        if (mw != expected) dimsSane = false;
                    }
                    prevW = mw;
                    walk.Add(new MipWalkEntry(
                        Index: i,
                        OffsetInExtras: mipStart,
                        BulkFlagsHex: $"0x{bulkFlags:x8}",
                        ElementCount: elementCount,
                        SizeOnDisk: sizeOnDisk,
                        OffsetInFile: offsetInFile,
                        SizeX: mw,
                        SizeY: mh,
                        SizeZ: mz,
                        FirstBytesHex: firstHex,
                        DimsSane: dimsSane));
                }
                walkEnd = pos;
                // Echo back any trailing bytes (bIsVirtual, end-of-format-chain marker, etc.).
                int trailLen = Math.Min(32, extras.Length - pos);
                if (trailLen > 0)
                    trailing = string.Join(" ", extras.Skip(pos).Take(trailLen).Select(b => b.ToString("x2")));
            }
            catch (Exception ex)
            {
                walkError = $"{ex.GetType().Name}: {ex.Message}";
            }
        }

        return new TextureBytesProbeResult(
            AssetPath: assetPath,
            EngineVersion: engineVer.ToString(),
            ExportCount: asset.Exports.Count,
            TextureExportIndex: texIdx,
            TextureExportClass: texClass,
            ExtrasLength: extras.Length,
            ExtrasFirst64Hex: hex,
            ExtrasFirst64Ascii: ascii,
            ParsedSizeX: sizeX, ParsedSizeY: sizeY, ParsedNumSlices: numSlices,
            ParsedPixelFormat: pixelFormat, ParsedFirstMip: firstMip, ParsedNumMips: numMips,
            ParsedExtrasOffsetToSizeX: offsetToSizeX,
            OneTimeCookedSentinel: sentinel,
            MipWalk: walk,
            MipWalkEndOffset: walkEnd,
            TrailingBytesHex: trailing,
            MipWalkError: walkError);
    }
}
