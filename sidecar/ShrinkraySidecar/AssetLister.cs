using System.Text.Json.Serialization;
using CUE4Parse.Compression;
using CUE4Parse.UE4.Pak;
using CUE4Parse.UE4.Versions;

namespace Shrinkray.Sidecar;

public sealed record AssetEntry(
    [property: JsonPropertyName("path")] string Path,
    [property: JsonPropertyName("size")] long Size,
    [property: JsonPropertyName("extension")] string Extension,
    [property: JsonPropertyName("compression")] string Compression,
    [property: JsonPropertyName("encrypted")] bool Encrypted,
    [property: JsonPropertyName("is_package")] bool IsPackage,
    [property: JsonPropertyName("is_payload")] bool IsPayload);

public sealed record ListAssetsResult(
    [property: JsonPropertyName("pak_path")] string PakPath,
    [property: JsonPropertyName("mount_point")] string MountPoint,
    [property: JsonPropertyName("entry_count")] int EntryCount,
    [property: JsonPropertyName("encrypted")] bool Encrypted,
    [property: JsonPropertyName("game")] string Game,
    [property: JsonPropertyName("entries")] IReadOnlyList<AssetEntry> Entries,
    [property: JsonPropertyName("truncated")] bool Truncated);

public static class AssetLister
{
    public static ListAssetsResult List(string pakPath, int? limit, EGame game)
    {
        if (!File.Exists(pakPath))
            throw new FileNotFoundException($"pak not found: {pakPath}");

        var versions = new VersionContainer(game);
        using var reader = new PakFileReader(pakPath, versions);

        if (reader.IsEncrypted)
        {
            // Without an AES key we can't read the index — surface this as a structured result
            // rather than throwing, so the UI can render the "encrypted, supply key" affordance.
            return new ListAssetsResult(
                PakPath: pakPath,
                MountPoint: reader.MountPoint ?? "",
                EntryCount: 0,
                Encrypted: true,
                Game: game.ToString(),
                Entries: Array.Empty<AssetEntry>(),
                Truncated: false);
        }

        reader.Mount(StringComparer.OrdinalIgnoreCase);

        var all = reader.Files;
        var truncated = false;
        IEnumerable<KeyValuePair<string, CUE4Parse.FileProvider.Objects.GameFile>> view = all;
        if (limit is int l && l > 0 && all.Count > l)
        {
            view = all.Take(l);
            truncated = true;
        }

        var entries = view.Select(kv =>
        {
            var file = kv.Value;
            return new AssetEntry(
                Path: file.Path,
                Size: file.Size,
                Extension: file.Extension,
                Compression: file.CompressionMethod.ToString(),
                Encrypted: file.IsEncrypted,
                IsPackage: file.IsUePackage,
                IsPayload: file.IsUePackagePayload);
        }).ToList();

        return new ListAssetsResult(
            PakPath: pakPath,
            MountPoint: reader.MountPoint ?? "",
            EntryCount: all.Count,
            Encrypted: reader.IsEncrypted,
            Game: game.ToString(),
            Entries: entries,
            Truncated: truncated);
    }
}
