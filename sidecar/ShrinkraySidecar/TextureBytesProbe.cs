using System.Text.Json.Serialization;
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
    [property: JsonPropertyName("parsed_extras_offset_to_size_x")] int? ParsedExtrasOffsetToSizeX);

public static class TextureBytesProbe
{
    public static TextureBytesProbeResult Probe(string assetPath, EngineVersion engineVer)
    {
        var asset = new UAsset(assetPath, engineVer);

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
                ParsedExtrasOffsetToSizeX: null);
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
            ParsedExtrasOffsetToSizeX: offsetToSizeX);
    }
}
