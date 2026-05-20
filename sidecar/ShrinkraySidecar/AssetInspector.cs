using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Versions;

namespace Shrinkray.Sidecar;

// Single-package inspection: open one .uasset (or .umap) inside a pak and return
// a summary the UI can render — class, dependency count, custom versions,
// export list. This is the Phase 2 read-side foundation: once we can resolve
// references like this, the next step is rewriting them.

public sealed record ExportInfo(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("class_name")] string ClassName,
    [property: JsonPropertyName("serial_size")] long SerialSize);

public sealed record ImportInfo(
    [property: JsonPropertyName("object_name")] string ObjectName,
    [property: JsonPropertyName("class_name")] string ClassName,
    [property: JsonPropertyName("outer_package")] string OuterPackage);

public sealed record CustomVersionEntry(
    [property: JsonPropertyName("key")] string Key,
    [property: JsonPropertyName("version")] int Version);

public sealed record MipDescriptor(
    [property: JsonPropertyName("index")] int Index,
    [property: JsonPropertyName("width")] int Width,
    [property: JsonPropertyName("height")] int Height,
    [property: JsonPropertyName("byte_size")] long ByteSize);

public sealed record TextureInfo(
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("class_name")] string ClassName,
    [property: JsonPropertyName("pixel_format")] string PixelFormat,
    [property: JsonPropertyName("mip_count")] int MipCount,
    [property: JsonPropertyName("mips")] IReadOnlyList<MipDescriptor> Mips,
    [property: JsonPropertyName("total_bytes")] long TotalBytes);

public sealed record InspectAssetResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("name_count")] int NameCount,
    [property: JsonPropertyName("import_count")] int ImportCount,
    [property: JsonPropertyName("export_count")] int ExportCount,
    [property: JsonPropertyName("file_version_ue")] string FileVersionUe,
    [property: JsonPropertyName("custom_versions")] IReadOnlyList<CustomVersionEntry> CustomVersions,
    [property: JsonPropertyName("exports")] IReadOnlyList<ExportInfo> Exports,
    [property: JsonPropertyName("imports")] IReadOnlyList<ImportInfo> Imports,
    [property: JsonPropertyName("textures")] IReadOnlyList<TextureInfo> Textures);

public static class AssetInspectorImpl
{
    public static InspectAssetResult Inspect(string pakPath, string assetPath, EGame game)
    {
        if (!File.Exists(pakPath))
            throw new FileNotFoundException($"pak not found: {pakPath}");
        var pakDir = Path.GetDirectoryName(Path.GetFullPath(pakPath))
            ?? throw new InvalidOperationException("pak has no parent directory");

        // Provider scans the pak's parent dir — fine even if there are siblings.
        // We're going to load exactly one package by path.
        var versions = new VersionContainer(game);
        using var provider = new DefaultFileProvider(
            new DirectoryInfo(pakDir),
            SearchOption.TopDirectoryOnly,
            versions);
        provider.Initialize();
        // Initialize() only enumerates paks in the directory — it does NOT
        // mount them. MountAsync() reads each pak's index into the provider's
        // Files dict; without it TryLoadPackage returns null for every path.
        // PostMount + LoadVirtualPaths finalize the /Game/ path resolution.
        provider.MountAsync().GetAwaiter().GetResult();
        provider.PostMount();
        provider.LoadVirtualPaths();

        // TryLoadPackage(string) swallows exceptions and returns false on
        // failure. Use LoadPackage(GameFile) directly so the underlying
        // exception (decompression, version, bad header) surfaces.
        IPackage? pkg = null;
        Exception? loadException = null;
        if (provider.Files.TryGetValue(assetPath, out var directHit))
        {
            try
            {
                pkg = provider.LoadPackage(directHit);
            }
            catch (Exception ex)
            {
                loadException = ex;
            }
        }
        if (pkg is null)
        {
            var detail = loadException is null
                ? "GameFile not found in provider.Files dict"
                : $"{loadException.GetType().Name}: {loadException.Message}";
            throw new InvalidOperationException(
                $"could not load package '{assetPath}' from {pakPath}. " +
                $"provider has {provider.Files.Count} files mounted across " +
                $"{provider.MountedVfs.Count} pak(s). underlying: {detail}");
        }

        var summary = pkg.Summary;

        var customVersions = new List<CustomVersionEntry>();
        if (summary?.CustomVersionContainer?.Versions is { } cv)
        {
            foreach (var v in cv)
                customVersions.Add(new CustomVersionEntry(v.Key.ToString(), v.Version));
        }

        var exports = new List<ExportInfo>();
        var textures = new List<TextureInfo>();
        var exportLazies = pkg.ExportsLazy;
        if (exportLazies is not null)
        {
            for (int i = 0; i < exportLazies.Length && i < 200; i++)
            {
                try
                {
                    var obj = exportLazies[i]?.Value;
                    if (obj is null) continue;
                    exports.Add(new ExportInfo(
                        Name: obj.Name,
                        ClassName: obj.ExportType ?? obj.GetType().Name,
                        SerialSize: 0));

                    // Texture inspection: any UTexture-derived class with a
                    // PlatformData.Mips chain. Safe under try/catch because
                    // some cooked formats omit data we expect.
                    if (obj is UTexture tex)
                    {
                        var info = BuildTextureInfo(tex);
                        if (info is not null) textures.Add(info);
                    }
                }
                catch
                {
                    // Some exports fail to deserialize on engine-version mismatches;
                    // we still want the summary, so swallow per-export errors.
                }
            }
        }

        var imports = new List<ImportInfo>();
        // ImportMap isn't exposed on IPackage directly in 1.2.2, but Package4 has
        // it via reflection-friendly access — we resolve via the package index API.
        // For now we just report counts; per-import resolution can come later.

        return new InspectAssetResult(
            PakPath: pakPath,
            AssetPath: assetPath,
            NameCount: summary?.NameCount ?? 0,
            ImportCount: pkg.ImportMapLength,
            ExportCount: pkg.ExportMapLength,
            FileVersionUe: summary?.FileVersionUE.ToString() ?? "unknown",
            CustomVersions: customVersions,
            Exports: exports,
            Imports: imports,
            Textures: textures);
    }

    /// <summary>
    /// Read the mip chain out of any UTexture-derived export. Returns null if
    /// PlatformData / Mips aren't accessible (e.g. RHI-cooked textures that
    /// shed their CPU-side data).
    /// </summary>
    private static TextureInfo? BuildTextureInfo(UTexture texture)
    {
        try
        {
            var platformData = texture switch
            {
                UTexture2D t2d => t2d.PlatformData,
                UTextureCube cube => cube.PlatformData,
                _ => null,
            };
            if (platformData is null || platformData.Mips is null || platformData.Mips.Length == 0)
                return null;

            var mips = new List<MipDescriptor>();
            long total = 0;
            for (int i = 0; i < platformData.Mips.Length; i++)
            {
                var mip = platformData.Mips[i];
                if (mip is null) continue;
                long size = 0;
                try
                {
                    // FByteBulkData.Header.ElementCount = total bytes in this mip.
                    // Header is a struct (value type), so `?.` is invalid there.
                    if (mip.BulkData is { } bulk)
                        size = bulk.Header.ElementCount;
                }
                catch
                {
                    // Bulk header may not be readable for inline / stripped cooks.
                }
                total += size;
                mips.Add(new MipDescriptor(
                    Index: i,
                    Width: mip.SizeX,
                    Height: mip.SizeY,
                    ByteSize: size));
            }

            return new TextureInfo(
                Name: texture.Name,
                ClassName: texture.ExportType ?? texture.GetType().Name,
                PixelFormat: platformData.PixelFormat.ToString(),
                MipCount: mips.Count,
                Mips: mips,
                TotalBytes: total);
        }
        catch
        {
            return null;
        }
    }
}
