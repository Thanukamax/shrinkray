using System.Linq;
using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Pak;
using CUE4Parse.UE4.Versions;

namespace Shrinkray.Sidecar;

// plan_strip_mips: walk every readable asset in a pak, find UTexture-derived
// exports, project the savings if we cap the top mip dimension to max_dim.
// Read-only. Apply path lands in a follow-up command once we wire UAssetAPI
// for write-side serialization.

public sealed record StripMipsItem(
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("export_name")] string ExportName,
    [property: JsonPropertyName("class_name")] string ClassName,
    [property: JsonPropertyName("pixel_format")] string PixelFormat,
    [property: JsonPropertyName("current_mip0_dim")] int CurrentMip0Dim,
    [property: JsonPropertyName("kept_mip0_dim")] int KeptMip0Dim,
    [property: JsonPropertyName("drop_mip_count")] int DropMipCount,
    [property: JsonPropertyName("kept_mip_count")] int KeptMipCount,
    [property: JsonPropertyName("save_bytes")] long SaveBytes,
    [property: JsonPropertyName("original_bytes")] long OriginalBytes,
    [property: JsonPropertyName("compression_settings")] string? CompressionSettings);

public sealed record ClassCount(
    [property: JsonPropertyName("class_name")] string ClassName,
    [property: JsonPropertyName("count")] int Count);

public sealed record PlanStripMipsResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("max_dim")] int MaxDim,
    [property: JsonPropertyName("scanned_assets")] int ScannedAssets,
    [property: JsonPropertyName("texture_count")] int TextureCount,
    [property: JsonPropertyName("items")] IReadOnlyList<StripMipsItem> Items,
    [property: JsonPropertyName("total_save_bytes")] long TotalSaveBytes,
    [property: JsonPropertyName("total_texture_bytes")] long TotalTextureBytes,
    [property: JsonPropertyName("truncated")] bool Truncated,
    [property: JsonPropertyName("class_histogram")] IReadOnlyList<ClassCount> ClassHistogram);

public static class StripMipsPlannerImpl
{
    /// <summary>
    /// Walk every readable package in a pak and project mip-strip savings
    /// for every UTexture-derived export whose top mip exceeds maxDim.
    /// </summary>
    public static PlanStripMipsResult Plan(string pakPath, int maxDim, EGame game, int assetLimit = 500)
    {
        if (!File.Exists(pakPath))
            throw new FileNotFoundException($"pak not found: {pakPath}");
        if (maxDim < 1 || maxDim > 16384)
            throw new ArgumentException("max_dim must be 1..16384");

        var pakDir = Path.GetDirectoryName(Path.GetFullPath(pakPath))
            ?? throw new InvalidOperationException("pak has no parent directory");
        var versions = new VersionContainer(game);

        // Enumerate the pak via PakFileReader directly — same pattern as
        // AssetLister. DefaultFileProvider.Files turned out to be unreliable
        // for post-Initialize enumeration of just-mounted pak entries.
        using var reader = new PakFileReader(pakPath, versions);
        if (reader.IsEncrypted)
        {
            // Can't read the index without an AES key.
            return new PlanStripMipsResult(
                PakPath: pakPath,
                MaxDim: maxDim,
                ScannedAssets: 0,
                TextureCount: 0,
                Items: Array.Empty<StripMipsItem>(),
                TotalSaveBytes: 0,
                TotalTextureBytes: 0,
                Truncated: false,
                ClassHistogram: Array.Empty<ClassCount>());
        }
        reader.Mount(StringComparer.OrdinalIgnoreCase);

        // Provider is used only for TryLoadPackage, which transparently
        // resolves the triple (.uasset + .uexp + .ubulk) once the pak is
        // mounted. Initialize the provider so it sees the pak too.
        using var provider = new DefaultFileProvider(
            new DirectoryInfo(pakDir),
            SearchOption.TopDirectoryOnly,
            versions);
        provider.Initialize();
        // Initialize() only enumerates paks. MountAsync() actually reads
        // pak indexes into the provider; PostMount + LoadVirtualPaths
        // finalize the /Game/ → real-path resolution.
        provider.MountAsync().GetAwaiter().GetResult();
        provider.PostMount();
        provider.LoadVirtualPaths();

        var items = new List<StripMipsItem>();
        var classHisto = new Dictionary<string, int>(StringComparer.Ordinal);
        long totalSave = 0;
        long totalTex = 0;
        int textureCount = 0;
        int scannedAssets = 0;
        bool truncated = false;

        foreach (var kv in reader.Files)
        {
            var file = kv.Value;
            // Only inspect package containers — IsUePackage flags .uasset /
            // .umap. The .uexp / .ubulk sidecars are loaded transitively by
            // TryLoadPackage.
            if (!file.IsUePackage) continue;

            if (scannedAssets >= assetLimit)
            {
                truncated = true;
                break;
            }
            scannedAssets++;

            try
            {
                CUE4Parse.UE4.Assets.IPackage? pkg = null;
                if (provider.Files.TryGetValue(file.Path, out var direct))
                {
                    try { pkg = provider.LoadPackage(direct); }
                    catch { /* per-package load failures are normal */ }
                }
                if (pkg is null) continue;

                // GetExports() materializes typed exports — OfType<UTexture>
                // filters by runtime type, which is the only reliable way to
                // catch UTexture2D + UTextureCube without missing subclasses.
                foreach (var obj in pkg.GetExports())
                {
                    if (obj is null) continue;
                    // Histogram by RUNTIME type (GetType().Name), so we can see
                    // whether CUE4Parse is constructing UTexture2D or just bare
                    // UObject with ExportType=Texture2D.
                    var exportType = obj.ExportType ?? "<null>";
                    var runtimeType = obj.GetType().FullName ?? obj.GetType().Name;
                    var className = $"{exportType} → {runtimeType}";
                    if (classHisto.Count < 80 || classHisto.ContainsKey(className))
                        classHisto[className] = classHisto.GetValueOrDefault(className) + 1;

                    // Match by GetType().Name — covers UTexture2D, UTextureCube,
                    // UVirtualTexture2D and any future subclass without
                    // requiring a typed cast (which CUE4Parse 1.x sometimes
                    // doesn't honor reliably for cooked content).
                    var typeName = obj.GetType().Name;
                    if (!typeName.StartsWith("UTexture")) continue;
                    try
                    {
                        var item = ProjectStripFromUObject(file.Path, obj, typeName, maxDim);
                        if (item is null) continue;
                        textureCount++;
                        totalTex += item.OriginalBytes;
                        if (item.SaveBytes > 0)
                        {
                            items.Add(item);
                            totalSave += item.SaveBytes;
                        }
                    }
                    catch
                    {
                        // Per-texture extraction failure — skip silently.
                    }
                }
            }
            catch
            {
                // Per-package load failures are normal — skip.
            }
        }

        var histoSorted = classHisto
            .OrderByDescending(kv => kv.Value)
            .Take(20)
            .Select(kv => new ClassCount(kv.Key, kv.Value))
            .ToList();

        // Largest savings first.
        items.Sort((a, b) => b.SaveBytes.CompareTo(a.SaveBytes));

        return new PlanStripMipsResult(
            PakPath: pakPath,
            MaxDim: maxDim,
            ScannedAssets: scannedAssets,
            TextureCount: textureCount,
            Items: items,
            TotalSaveBytes: totalSave,
            TotalTextureBytes: totalTex,
            Truncated: truncated,
            ClassHistogram: histoSorted);
    }

    /// <summary>
    /// Project the strip for any texture-named UObject. Reads SizeX / SizeY /
    /// NumMips / Format via property dictionary (works regardless of whether
    /// CUE4Parse's typed deserializer populated PlatformData), then computes
    /// per-mip byte sizes via the pixel-format formula. Matches the on-disk
    /// layout UE uses for cooked textures.
    /// </summary>
    private static StripMipsItem? ProjectStripFromUObject(
        string assetPath,
        CUE4Parse.UE4.Assets.Exports.UObject obj,
        string typeName,
        int maxDim)
    {
        // Reflection: PlatformData on UTexture is a PROPERTY (with private
        // setter); SizeX / SizeY / PixelFormat / Mips on FTexturePlatformData
        // are public FIELDS — GetProperty wouldn't find them.
        int sizeX = 0, sizeY = 0, numMips = 0;
        string formatName = "PF_Unknown";
        var t = obj.GetType();
        var pdProp = t.GetProperty("PlatformData");
        var pd = pdProp?.GetValue(obj);
        if (pd is not null)
        {
            var pdType = pd.GetType();
            sizeX = (int)(pdType.GetField("SizeX")?.GetValue(pd) ?? 0);
            sizeY = (int)(pdType.GetField("SizeY")?.GetValue(pd) ?? 0);
            var pixFmt = pdType.GetField("PixelFormat")?.GetValue(pd);
            formatName = pixFmt?.ToString() ?? "PF_Unknown";
            if (!formatName.StartsWith("PF_")) formatName = "PF_" + formatName;
            var mipsArr = pdType.GetField("Mips")?.GetValue(pd) as System.Collections.IEnumerable;
            numMips = mipsArr is null ? 0 : mipsArr.Cast<object?>().Count();
        }
        // Property dict fallback for UE versions where PlatformData isn't
        // serialized but the UPROPERTY-tagged fields exist.
        if (sizeX <= 0) sizeX = obj.GetOrDefault<int>("SizeX");
        if (sizeY <= 0) sizeY = obj.GetOrDefault<int>("SizeY");
        if (numMips <= 0) numMips = obj.GetOrDefault<int>("NumMips");
        if (formatName == "PF_Unknown")
        {
            var fn = obj.GetOrDefault<FName>("Format").Text;
            if (!string.IsNullOrEmpty(fn)) formatName = fn;
        }
        if (sizeX <= 0 || sizeY <= 0 || numMips <= 0) return null;

        // Pull CompressionSettings UPROPERTY when the cook serialized it.
        // Many UE4 cooks drop this — the Rust-side classifier has a name +
        // pixel-format fallback for that case, so we just surface whatever
        // we find here and let downstream decide.
        string? compressionSettings = null;
        var csFn = obj.GetOrDefault<FName>("CompressionSettings").Text;
        if (!string.IsNullOrEmpty(csFn) && csFn != "None") compressionSettings = csFn;

        long originalBytes = 0;
        var sizes = new long[numMips];
        for (int i = 0; i < numMips; i++)
        {
            int w = Math.Max(1, sizeX >> i);
            int h = Math.Max(1, sizeY >> i);
            sizes[i] = BytesForMip(formatName, w, h);
            originalBytes += sizes[i];
        }
        int firstKept = numMips - 1;
        for (int i = 0; i < numMips; i++)
        {
            int w = Math.Max(1, sizeX >> i);
            int h = Math.Max(1, sizeY >> i);
            if (Math.Max(w, h) <= maxDim) { firstKept = i; break; }
        }
        if (firstKept <= 0) return null;

        long saveBytes = 0;
        for (int i = 0; i < firstKept; i++) saveBytes += sizes[i];
        int keptMip0 = Math.Max(Math.Max(1, sizeX >> firstKept), Math.Max(1, sizeY >> firstKept));
        int currentMip0 = Math.Max(sizeX, sizeY);

        return new StripMipsItem(
            AssetPath: assetPath,
            ExportName: obj.Name,
            ClassName: obj.ExportType ?? typeName,
            PixelFormat: formatName,
            CurrentMip0Dim: currentMip0,
            KeptMip0Dim: keptMip0,
            DropMipCount: firstKept,
            KeptMipCount: numMips - firstKept,
            SaveBytes: saveBytes,
            OriginalBytes: originalBytes,
            CompressionSettings: compressionSettings);
    }

    /// <summary>Per-mip byte size in bytes for a given pixel format.</summary>
    private static long BytesForMip(string format, int w, int h)
    {
        // Block-compressed formats: pixels packed in 4×4 blocks.
        // BC1/DXT1/BC4: 8 bytes/block. BC2/BC3/DXT5/BC5/BC6H/BC7: 16 bytes/block.
        long bx = Math.Max(1, (w + 3) / 4);
        long by = Math.Max(1, (h + 3) / 4);
        switch (format)
        {
            case "PF_DXT1":
            case "PF_BC1":
            case "PF_BC4":
                return bx * by * 8;
            case "PF_DXT3":
            case "PF_DXT5":
            case "PF_BC2":
            case "PF_BC3":
            case "PF_BC5":
            case "PF_BC6H":
            case "PF_BC7":
                return bx * by * 16;
            // ASTC blocks (UE5+ mobile cooks). Sizes are per-block bytes.
            case "PF_ASTC_4x4":
            case "PF_ASTC_4x4_HDR":
                return ((long)w * h);  // 1 byte/pixel
            case "PF_ASTC_6x6":
            case "PF_ASTC_6x6_HDR":
                return ((long)((w + 5) / 6) * ((h + 5) / 6)) * 16;
            case "PF_ASTC_8x8":
            case "PF_ASTC_8x8_HDR":
                return ((long)((w + 7) / 8) * ((h + 7) / 8)) * 16;
            // Uncompressed.
            case "PF_A8":
            case "PF_G8":
            case "PF_L8":
            case "PF_R8":
                return (long)w * h;
            case "PF_G16":
            case "PF_R16F":
            case "PF_R16_UINT":
            case "PF_R16_SINT":
                return (long)w * h * 2;
            case "PF_B8G8R8A8":
            case "PF_R8G8B8A8":
            case "PF_R8G8B8A8_UINT":
            case "PF_R8G8B8A8_SNORM":
            case "PF_A2B10G10R10":
            case "PF_A2R10G10B10":
                return (long)w * h * 4;
            case "PF_FloatRGB":
            case "PF_FloatRGBA":
            case "PF_R32_FLOAT":
            case "PF_R16G16_FLOAT":
                return (long)w * h * 8;
            case "PF_A32B32G32R32F":
                return (long)w * h * 16;
            default:
                // Conservative guess for unknown formats — assume BC3-like
                // 16 bytes/block. Rather over- than under-report.
                return bx * by * 16;
        }
    }

}
