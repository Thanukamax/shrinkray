using System.Text.Json;
using System.Text.Json.Serialization;

namespace Shrinkray.Sidecar;

// Newline-delimited JSON request/response over stdin/stdout.
//   { "id": "1", "cmd": "ping" }
//   { "id": "1", "ok": true, "result": { "version": "0.1.0" } }
//   { "id": "2", "cmd": "list_assets", "args": { "pak_path": "/path/to/foo.pak" } }
//   { "id": "2", "ok": true, "result": { "entries": [...] } }
//   { "id": "2", "ok": false, "error": "..." }
// One JSON object per line. EOF on stdin = exit.
public static class Program
{
    public const string SidecarVersion = "0.1.0";

    public static async Task<int> Main(string[] args)
    {
        // --version short-circuit so the Rust side can health-check the binary.
        if (args.Length == 1 && args[0] == "--version")
        {
            Console.WriteLine(SidecarVersion);
            return 0;
        }

        // CUE4Parse 1.2.2 doesn't auto-initialize its compression layer.
        // Without this, every TryLoadPackage on a Zlib-compressed pak entry
        // throws "Zlib decompression failed: not initialized" and returns
        // null. We pass an explicit path because most distros ship the
        // SONAME-versioned libz-ng.so.2 rather than the bare libz-ng.so.
        InitializeCompression();

        var stdin = Console.In;
        var stdout = Console.Out;

        string? line;
        while ((line = await stdin.ReadLineAsync()) != null)
        {
            if (string.IsNullOrWhiteSpace(line)) continue;
            string responseJson;
            try
            {
                var request = JsonSerializer.Deserialize<Request>(line, JsonOpts.Reader)
                    ?? throw new InvalidOperationException("null request");
                responseJson = await Dispatch(request);
            }
            catch (Exception ex)
            {
                var err = new Response { Id = null, Ok = false, Error = $"protocol error: {ex.Message}" };
                responseJson = JsonSerializer.Serialize(err, JsonOpts.Writer);
            }
            await stdout.WriteLineAsync(responseJson);
            await stdout.FlushAsync();
        }
        return 0;
    }

    /// <summary>
    /// Initialize CUE4Parse's compression layer. Must run before any
    /// TryLoadPackage / LoadPackage call on a compressed pak entry.
    /// </summary>
    private static void InitializeCompression()
    {
        // Common paths for libz-ng on Linux distros. The first one that
        // exists wins; if none do, fall back to letting CUE4Parse search
        // its default name (libz-ng.so) on the loader path.
        string[] candidates =
        {
            "/lib64/libz-ng.so.2",
            "/usr/lib64/libz-ng.so.2",
            "/usr/lib/x86_64-linux-gnu/libz-ng.so.2",
            "/lib/libz-ng.so.2",
        };
        foreach (var path in candidates)
        {
            if (!File.Exists(path)) continue;
            try
            {
                CUE4Parse.Compression.ZlibHelper.Initialize(path);
                return;
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine($"[sidecar] ZlibHelper.Initialize({path}) failed: {ex.Message}");
            }
        }
        // Last resort: pass null so CUE4Parse uses its default name.
        try
        {
            CUE4Parse.Compression.ZlibHelper.Initialize((string?)null);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[sidecar] ZlibHelper default init failed: {ex.Message}");
        }
    }

    private static Task<string> Dispatch(Request req)
    {
        Response resp;
        try
        {
            object? result = req.Cmd switch
            {
                "ping" => new PingResult(SidecarVersion),
                "list_assets" => HandleListAssets(req.Args),
                "inspect_asset" => HandleInspectAsset(req.Args),
                "plan_strip_mips" => HandlePlanStripMips(req.Args),
                "apply_strip_mips" => HandleApplyStripMips(req.Args),
                _ => throw new InvalidOperationException($"unknown command: {req.Cmd}"),
            };
            resp = new Response { Id = req.Id, Ok = true, Result = result };
        }
        catch (Exception ex)
        {
            resp = new Response { Id = req.Id, Ok = false, Error = ex.Message };
        }
        return Task.FromResult(JsonSerializer.Serialize(resp, JsonOpts.Writer));
    }

    private static object HandleListAssets(JsonElement? args)
    {
        if (args is null) throw new ArgumentException("list_assets requires args.pak_path");
        var pakPath = args.Value.TryGetProperty("pak_path", out var p)
            ? p.GetString() ?? throw new ArgumentException("pak_path must be string")
            : throw new ArgumentException("missing pak_path");
        int? limit = args.Value.TryGetProperty("limit", out var l) && l.ValueKind == JsonValueKind.Number
            ? l.GetInt32()
            : null;
        // game arg is optional; accept an EGame name like "GAME_UE5_LATEST" or "GAME_UE4_27".
        var game = CUE4Parse.UE4.Versions.EGame.GAME_UE5_LATEST;
        if (args.Value.TryGetProperty("game", out var g) && g.ValueKind == JsonValueKind.String)
        {
            var name = g.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<CUE4Parse.UE4.Versions.EGame>(name, true, out var parsed))
                game = parsed;
        }
        return AssetLister.List(pakPath, limit, game);
    }

    private static object HandleInspectAsset(JsonElement? args)
    {
        if (args is null) throw new ArgumentException("inspect_asset requires args.pak_path + args.asset_path");
        var pakPath = args.Value.TryGetProperty("pak_path", out var p)
            ? p.GetString() ?? throw new ArgumentException("pak_path must be string")
            : throw new ArgumentException("missing pak_path");
        var assetPath = args.Value.TryGetProperty("asset_path", out var a)
            ? a.GetString() ?? throw new ArgumentException("asset_path must be string")
            : throw new ArgumentException("missing asset_path");
        var game = CUE4Parse.UE4.Versions.EGame.GAME_UE5_LATEST;
        if (args.Value.TryGetProperty("game", out var g) && g.ValueKind == JsonValueKind.String)
        {
            var name = g.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<CUE4Parse.UE4.Versions.EGame>(name, true, out var parsed))
                game = parsed;
        }
        return AssetInspectorImpl.Inspect(pakPath, assetPath, game);
    }

    /// <summary>
    /// v0.6 stub. The write-side requires either UAssetAPI integration (to
    /// rewrite cooked .uasset/.uexp/.ubulk in place) or a custom binary
    /// surgery pass. Both are non-trivial and land in v0.6. This stub locks
    /// the IPC shape so the frontend can render a proper "what's next" state
    /// without 404ing on the command.
    /// </summary>
    private static object HandleApplyStripMips(JsonElement? args)
    {
        return new
        {
            implemented = false,
            phase = "v0.6",
            message =
                "apply_strip_mips is not implemented yet. The read-side (plan_strip_mips) " +
                "is byte-exact and ready; the write-side needs UAssetAPI integration to " +
                "rewrite cooked .uasset/.uexp/.ubulk triples and repack into the pak. " +
                "Track the v0.6 milestone for write-side mip stripping.",
            backup_required = true,
            requires = new[] { "UAssetAPI .NET dep", "pak rewriter extension", "LoadPackage validation per rewritten asset" }
        };
    }

    private static object HandlePlanStripMips(JsonElement? args)
    {
        if (args is null) throw new ArgumentException("plan_strip_mips requires args.pak_path + args.max_dim");
        var pakPath = args.Value.TryGetProperty("pak_path", out var p)
            ? p.GetString() ?? throw new ArgumentException("pak_path must be string")
            : throw new ArgumentException("missing pak_path");
        int maxDim = args.Value.TryGetProperty("max_dim", out var md) && md.ValueKind == JsonValueKind.Number
            ? md.GetInt32()
            : throw new ArgumentException("missing max_dim");
        int limit = args.Value.TryGetProperty("limit", out var l) && l.ValueKind == JsonValueKind.Number
            ? l.GetInt32()
            : 500;
        var game = CUE4Parse.UE4.Versions.EGame.GAME_UE5_LATEST;
        if (args.Value.TryGetProperty("game", out var g) && g.ValueKind == JsonValueKind.String)
        {
            var name = g.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<CUE4Parse.UE4.Versions.EGame>(name, true, out var parsed))
                game = parsed;
        }
        return StripMipsPlannerImpl.Plan(pakPath, maxDim, game, limit);
    }
}

public sealed class Request
{
    [JsonPropertyName("id")] public string? Id { get; set; }
    [JsonPropertyName("cmd")] public string Cmd { get; set; } = "";
    [JsonPropertyName("args")] public JsonElement? Args { get; set; }
}

public sealed class Response
{
    [JsonPropertyName("id")] public string? Id { get; set; }
    [JsonPropertyName("ok")] public bool Ok { get; set; }
    [JsonPropertyName("result")] public object? Result { get; set; }
    [JsonPropertyName("error")] public string? Error { get; set; }
}

public sealed record PingResult(
    [property: JsonPropertyName("version")] string Version);

internal static class JsonOpts
{
    public static readonly JsonSerializerOptions Reader = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        PropertyNameCaseInsensitive = true,
    };
    public static readonly JsonSerializerOptions Writer = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        DefaultIgnoreCondition = JsonIgnoreCondition.WhenWritingNull,
    };
}
