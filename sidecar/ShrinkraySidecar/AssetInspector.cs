using System.Text.Json.Serialization;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets;
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

public sealed record InspectAssetResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("asset_path")] string AssetPath,
    [property: JsonPropertyName("name_count")] int NameCount,
    [property: JsonPropertyName("import_count")] int ImportCount,
    [property: JsonPropertyName("export_count")] int ExportCount,
    [property: JsonPropertyName("file_version_ue")] string FileVersionUe,
    [property: JsonPropertyName("custom_versions")] IReadOnlyList<CustomVersionEntry> CustomVersions,
    [property: JsonPropertyName("exports")] IReadOnlyList<ExportInfo> Exports,
    [property: JsonPropertyName("imports")] IReadOnlyList<ImportInfo> Imports);

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

        if (!provider.TryLoadPackage(assetPath, out var pkg) || pkg is null)
            throw new InvalidOperationException(
                $"could not load package '{assetPath}' from {pakPath}");

        var summary = pkg.Summary;

        var customVersions = new List<CustomVersionEntry>();
        if (summary?.CustomVersionContainer?.Versions is { } cv)
        {
            foreach (var v in cv)
                customVersions.Add(new CustomVersionEntry(v.Key.ToString(), v.Version));
        }

        var exports = new List<ExportInfo>();
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
            Imports: imports);
    }
}
