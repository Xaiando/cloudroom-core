using System.IO;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json.Nodes;

namespace Cloudroom.Desktop;

public sealed class CoreClient : IDisposable
{
    private readonly HttpClient http;
    private readonly string token;
    public Uri Endpoint { get; }
    public string Identity { get; }

    public CoreClient(string endpoint, string token)
    {
        if (!Uri.TryCreate(endpoint, UriKind.Absolute, out var uri) || !uri.IsLoopback ||
            uri.Scheme is not ("http" or "https") || uri.UserInfo.Length > 0 || uri.Query.Length > 0 || uri.Fragment.Length > 0 || uri.AbsolutePath != "/")
            throw new ArgumentException("Use a local Core address, such as http://127.0.0.1:9840.");
        if (token.Length < 32 || token.Any(c => c < '!' || c > '~')) throw new ArgumentException("The Core token must have at least 32 visible ASCII characters.");
        Endpoint = uri;
        Identity = Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(Encoding.UTF8.GetBytes(uri.AbsoluteUri + "\n" + token)))[..16];
        this.token = token;
        // Never forward the local credential through a system proxy or a redirect.
        http = new HttpClient(new HttpClientHandler { AllowAutoRedirect = false, UseProxy = false }) { BaseAddress = uri, Timeout = TimeSpan.FromSeconds(20) };
        http.DefaultRequestHeaders.Authorization = new AuthenticationHeaderValue("Bearer", token);
    }

    public static async Task<(string Endpoint, string Token)> ReadSettings(string path)
    {
        var root = JsonNode.Parse(await File.ReadAllTextAsync(path)) ?? throw new IOException("Core settings are empty.");
        return ("http://" + Json.Text(root, "CLOUDROOM_LISTEN"), Json.Text(root, "CLOUDROOM_TOKEN"));
    }

    public async Task<JsonNode> Get(string path, CancellationToken ct = default) => await Request(HttpMethod.Get, path, null, ct);
    public async Task<JsonNode> Post(string path, JsonObject body, CancellationToken ct = default) => await Request(HttpMethod.Post, path, body, ct);
    private async Task<JsonNode> Request(HttpMethod method, string path, JsonObject? body, CancellationToken ct)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(ct);
        deadline.CancelAfter(TimeSpan.FromSeconds(20));
        ct = deadline.Token;
        if (!path.StartsWith("/v1/", StringComparison.Ordinal)) throw new ArgumentException("Invalid Core API path.");
        using var request = new HttpRequestMessage(method, path);
        if (body != null) request.Content = new StringContent(body.ToJsonString(), Encoding.UTF8, "application/json");
        using var response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, ct);
        await using var stream = await response.Content.ReadAsStreamAsync(ct);
        using var data = new MemoryStream();
        var buffer = new byte[8192];
        int n;
        while ((n = await stream.ReadAsync(buffer, ct)) > 0)
        {
            if (data.Length + n > 32 * 1024 * 1024) throw new IOException("Core response exceeded the 32 MB limit.");
            data.Write(buffer, 0, n);
        }
        var text = Encoding.UTF8.GetString(data.ToArray());
        JsonNode? json = null;
        try { json = JsonNode.Parse(text); } catch (System.Text.Json.JsonException) { }
        if (!response.IsSuccessStatusCode)
        {
            string detail = response.StatusCode == HttpStatusCode.Unauthorized ? "The Core token was rejected. Check Connection settings." : Json.Text(json, "error", "Core returned HTTP " + (int)response.StatusCode + ".");
            throw new CoreApiException(detail.Replace(token, "[redacted]", StringComparison.Ordinal), (int)response.StatusCode);
        }
        return json ?? throw new IOException("Core returned an invalid JSON response.");
    }
    public void Dispose() => http.Dispose();
}

public sealed class CoreApiException(string message, int statusCode) : IOException(message)
{
    public int StatusCode { get; } = statusCode;
}

public static class Json
{
    public static string Text(JsonNode? value, string key, string fallback = "") => value?[key] is JsonValue field && field.TryGetValue<string>(out var text) ? text : fallback;
    public static bool Flag(JsonNode? value, string key) => value?[key] is JsonValue field && field.TryGetValue<bool>(out var flag) && flag;
    public static long Number(JsonNode? value, string key) => value?[key] is JsonValue field && field.TryGetValue<long>(out var number) ? number : 0;
    public static JsonArray Array(JsonNode? value, string key) => value?[key] as JsonArray ?? [];
    public static string Id() => Guid.NewGuid().ToString("N");
}
