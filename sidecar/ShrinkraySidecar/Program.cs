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
