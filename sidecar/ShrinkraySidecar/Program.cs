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
//
// v0.7.2: a handler MAY also emit 0..N intermediate progress lines before the
// final response. These look like:
//   { "id": "2", "progress": { "current": 5, "total": 200, ... } }
// and are written via ProgressEmitter.Emit(payload) from inside the handler.
// The terminal response (with the `ok` field) is still required and arrives
// after all progress messages — clients distinguish by presence of `progress`
// vs `ok` per line. Backward compatible: clients that ignore `progress` lines
// keep working with no protocol break.
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
            string? capturedId = null;
            try
            {
                var request = JsonSerializer.Deserialize<Request>(line, JsonOpts.Reader)
                    ?? throw new InvalidOperationException("null request");
                capturedId = request.Id;
                // Bind the per-request progress emitter for the duration of
                // Dispatch. Sidecar is single-threaded (one request at a
                // time), so a static-field context is enough — no AsyncLocal.
                ProgressEmitter.Bind(stdout, request.Id);
                try
                {
                    responseJson = await Dispatch(request);
                }
                finally
                {
                    ProgressEmitter.Unbind();
                }
            }
            catch (Exception ex)
            {
                ProgressEmitter.Unbind();
                var err = new Response { Id = capturedId, Ok = false, Error = $"protocol error: {ex.Message}" };
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
                "probe_texture_bytes" => HandleProbeTextureBytes(req.Args),
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

    private static object HandleProbeTextureBytes(JsonElement? args)
    {
        if (args is null) throw new ArgumentException("probe_texture_bytes requires args.asset_path");
        var assetPath = args.Value.TryGetProperty("asset_path", out var p)
            ? p.GetString() ?? throw new ArgumentException("asset_path must be string")
            : throw new ArgumentException("missing asset_path");
        var engineVer = UAssetAPI.UnrealTypes.EngineVersion.VER_UE4_AUTOMATIC_VERSION;
        if (args.Value.TryGetProperty("engine_version", out var ev) && ev.ValueKind == JsonValueKind.String)
        {
            var name = ev.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<UAssetAPI.UnrealTypes.EngineVersion>(name, true, out var parsed))
                engineVer = parsed;
        }
        bool walkMips = args.Value.TryGetProperty("walk_mips", out var wm)
            && wm.ValueKind == JsonValueKind.True;
        string? pakPath = args.Value.TryGetProperty("pak_path", out var pp) && pp.ValueKind == JsonValueKind.String
            ? pp.GetString()
            : null;
        var game = CUE4Parse.UE4.Versions.EGame.GAME_UE5_LATEST;
        if (args.Value.TryGetProperty("game", out var g) && g.ValueKind == JsonValueKind.String)
        {
            var name = g.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<CUE4Parse.UE4.Versions.EGame>(name, true, out var parsed))
                game = parsed;
        }
        return TextureBytesProbe.Probe(assetPath, engineVer, walkMips, pakPath, game);
    }

    /// <summary>
    /// v0.6 write-side: for each (asset_path, max_dim) target inside a pak,
    /// rewrite the .uasset Extras byte buffer to drop top mips above max_dim
    /// and regenerate the .ubulk with surviving mip payloads only. Returns
    /// modified bytes (base64) keyed by pak-relative path so the Rust side
    /// can substitute them in a repak rewrite. Also returns the original
    /// bytes so backup.rs can record them for restore.
    /// </summary>
    private static object HandleApplyStripMips(JsonElement? args)
    {
        if (args is null) throw new ArgumentException("apply_strip_mips requires args.pak_path + args.targets");
        var pakPath = args.Value.TryGetProperty("pak_path", out var p)
            ? p.GetString() ?? throw new ArgumentException("pak_path must be string")
            : throw new ArgumentException("missing pak_path");
        if (!args.Value.TryGetProperty("targets", out var t) || t.ValueKind != JsonValueKind.Array)
            throw new ArgumentException("missing targets array");

        var targets = new List<StripTarget>();
        foreach (var item in t.EnumerateArray())
        {
            var assetPath = item.TryGetProperty("asset_path", out var ap)
                ? ap.GetString() ?? throw new ArgumentException("target.asset_path must be string")
                : throw new ArgumentException("missing target.asset_path");
            int maxDim = item.TryGetProperty("max_dim", out var md) && md.ValueKind == JsonValueKind.Number
                ? md.GetInt32()
                : throw new ArgumentException("missing target.max_dim");
            targets.Add(new StripTarget(assetPath, maxDim));
        }

        var game = CUE4Parse.UE4.Versions.EGame.GAME_UE5_LATEST;
        if (args.Value.TryGetProperty("game", out var g) && g.ValueKind == JsonValueKind.String)
        {
            var name = g.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<CUE4Parse.UE4.Versions.EGame>(name, true, out var parsed))
                game = parsed;
        }
        var engineVer = UAssetAPI.UnrealTypes.EngineVersion.VER_UE4_AUTOMATIC_VERSION;
        if (args.Value.TryGetProperty("engine_version", out var ev) && ev.ValueKind == JsonValueKind.String)
        {
            var name = ev.GetString();
            if (!string.IsNullOrEmpty(name) && Enum.TryParse<UAssetAPI.UnrealTypes.EngineVersion>(name, true, out var parsed))
                engineVer = parsed;
        }

        return StripMipsApplier.Apply(pakPath, targets, game, engineVer);
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

public sealed class ProgressMessage
{
    [JsonPropertyName("id")] public string? Id { get; set; }
    [JsonPropertyName("progress")] public object? Progress { get; set; }
}

/// <summary>
/// Per-request progress emitter. <see cref="Program.Main"/> binds the current
/// stdout + request id before dispatching a handler; handlers call
/// <see cref="Emit"/> any number of times to write intermediate progress lines
/// alongside the eventual response. The Rust client parses progress lines
/// separately from the terminal response by checking for the `progress` field
/// vs the `ok` field (see <c>shrinkray-sidecar::Sidecar::call_with_progress</c>).
///
/// Thread-safe: a handler may emit progress from worker threads via
/// <see cref="Parallel.ForEach"/>; the internal lock keeps WriteLine + Flush
/// from interleaving bytes between threads. Without the lock concurrent calls
/// can corrupt the JSON stream and the Rust client's line-based parser fails.
/// </summary>
public static class ProgressEmitter
{
    private static TextWriter? _stdout;
    private static string? _requestId;
    private static readonly object _writeLock = new();

    internal static void Bind(TextWriter stdout, string? requestId)
    {
        _stdout = stdout;
        _requestId = requestId;
    }

    internal static void Unbind()
    {
        _stdout = null;
        _requestId = null;
    }

    public static void Emit(object payload)
    {
        var stdout = _stdout;
        if (stdout is null) return;
        var msg = new ProgressMessage { Id = _requestId, Progress = payload };
        var json = JsonSerializer.Serialize(msg, JsonOpts.Writer);
        // WriteLine + Flush keep the client's read loop unblocked; without
        // the flush stdout buffering can hold the message until the response.
        // The lock prevents concurrent writers from interleaving bytes.
        lock (_writeLock)
        {
            stdout.WriteLine(json);
            stdout.Flush();
        }
    }
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
