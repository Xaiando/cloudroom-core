using System.IO;
using System.Text.Json.Nodes;

namespace Cloudroom.Desktop;

public partial class MainWindow
{
    private async Task ConnectionSettingsSmokeTest()
    {
        string root = Path.Combine(Path.GetDirectoryName(Path.GetFullPath(ConfigPathBox.Text))!, "settings-discovery");
        string normal = Path.Combine(root, "CloudroomCore", "core.json");
        string cached = Path.Combine(root, "Packages", "OpenAI.Codex_fixture", "LocalCache", "Local", "CloudroomCore", "core.json");
        Directory.CreateDirectory(Path.GetDirectoryName(normal)!);
        Directory.CreateDirectory(Path.GetDirectoryName(cached)!);
        string original = await File.ReadAllTextAsync(ConfigPathBox.Text);
        var stale = JsonNode.Parse(original)!;
        stale["CLOUDROOM_TOKEN"] = new string('x', 64);
        await File.WriteAllTextAsync(normal, stale.ToJsonString());
        await File.WriteAllTextAsync(cached, original);
        string resolved = await CoreClient.FindDefaultSettings(root, lifetime.Token);
        var physical = await CoreClient.ReadSettings(cached);
        if (resolved != physical.Path || !Path.IsPathFullyQualified(resolved)) throw new IOException("Default settings discovery selected the rejected token instead of the running Core.");
        var explicitSettings = await CoreClient.ReadSettings(normal);
        using var rejected = new CoreClient(explicitSettings.Endpoint, explicitSettings.Token);
        try { await rejected.Get("/v1/ready", lifetime.Token); throw new IOException("Explicit rejected credentials were silently replaced."); }
        catch (CoreApiException error) when (error.StatusCode == 401) { }
        if (await File.ReadAllTextAsync(cached) != original || await File.ReadAllTextAsync(normal) != stale.ToJsonString()) throw new IOException("Settings discovery changed saved credentials.");
    }
}
