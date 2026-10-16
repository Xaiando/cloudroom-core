using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using Microsoft.Win32;

namespace Cloudroom.Desktop;

public sealed record CouncilModel(string Harness, string Model, string Provider) { public string Label => Model + (Provider == "opencode" ? " · OpenCode" : Provider == "deepseek" ? " · direct" : Harness == "local" ? " · local" : ""); }
public sealed record CouncilSaved(string Id, string Title, string State) { public string Label => Title + " · " + State.Replace('_', ' '); }
public sealed record CouncilTaskView(string Title, string Owner, string State, string Activity, string Result, string Operations, string Timing, bool Busy, double Progress, int Index, string RoundId, bool WorkAllowed);
public partial class MainWindow
{
    private string? councilId, councilIdentity;
    private JsonObject? pendingCouncilStart, pendingCouncilMessage;
    private bool councilLoading, councilBusy, councilRefreshing, councilSelectionDirty;
    private string councilRendered = "";
    private JsonNode? councilCatalog;
    private string? councilCacheId;
    private int councilBefore;
    private readonly SortedDictionary<int, JsonNode> councilRounds = [];
    private readonly Dictionary<string, string> councilDrafts = [];
    private (CheckBox Enable, ComboBox Models, TextBlock Status, string Harness)[] CouncilSlots => [(CouncilGptOn, CouncilGptModel, CouncilGptStatus, "codex"), (CouncilGrokOn, CouncilGrokModel, CouncilGrokStatus, "grok"), (CouncilClaudeOn, CouncilClaudeModel, CouncilClaudeStatus, "claude"), (CouncilQwenOn, CouncilQwenModel, CouncilQwenStatus, "qwen"), (CouncilDeepSeekOn, CouncilDeepSeekModel, CouncilDeepSeekStatus, "deepseek")];
    private async Task OpenCouncilWorkspace()
    {
        if (client == null || !connected) throw new IOException("Connect to Core first.");
        CouncilWorkspacePanel.Visibility = Visibility.Visible;
        councilCatalog = await client.Get("/v1/bee/councils/catalog", lifetime.Token);
        councilLoading = true;
        foreach (var slot in CouncilSlots)
        {
            string selected = (slot.Models.SelectedItem as CouncilModel)?.Model ?? "";
            var models = Json.Array(councilCatalog, "models").Where(m => Json.Text(m, "harness") == slot.Harness || slot.Harness == "qwen" && Json.Text(m, "harness") == "local")
                .Select(m => new CouncilModel(Json.Text(m, "harness"), Json.Text(m, "model"), Json.Text(m, "provider"))).ToList();
            slot.Models.ItemsSource = models; slot.Models.SelectedItem = models.FirstOrDefault(m => m.Model == selected) ?? models.FirstOrDefault();
            if (models.Count == 0) slot.Enable.IsChecked = false;
            slot.Status.Text = models.Count == 0 ? "Add a Chat connection" : "Ready";
        }
        councilLoading = false;
        RestoreCouncilDraft();
        await RefreshCouncilWorkspace(true);
        UpdateCouncilControls();
    }
    private void DismissCouncilWorkspace(object sender, RoutedEventArgs e) { SaveCouncilDraft(); CouncilWorkspacePanel.Visibility = Visibility.Collapsed; }
    private async void CompareCurrentGroup(object sender, RoutedEventArgs e) => await Act(OpenGroupComparison);
    private async Task OpenGroupComparison()
    {
        SaveCouncilDraft(); CouncilWorkspacePanel.Visibility = Visibility.Collapsed;
        councilView = true; CouncilPanel.Visibility = Visibility.Visible; await RefreshCouncil();
    }
    private void DismissGroupComparison(object sender, RoutedEventArgs e) { councilView = false; CouncilPanel.Visibility = Visibility.Collapsed; }
    private async void CouncilConnections(object sender, RoutedEventArgs e) { ShowChatSetup(sender, e); await Task.CompletedTask; }
    private void CouncilBrowseFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "Choose the shared Council workspace", Multiselect = false };
        if (dialog.ShowDialog(this) == true) { CouncilFolder.Text = dialog.FolderName; SaveCouncilDraft(); }
    }
    private async void NewCouncilClicked(object sender, RoutedEventArgs e) => await Act(() =>
    {
        if (pendingCouncilStart != null || pendingCouncilMessage != null) throw new IOException("Retry the unconfirmed Council request before starting another conversation.");
        if (councilId != null) councilDrafts[councilId] = CouncilComposer.Text;
        councilId = null; councilBusy = false; councilRendered = ""; CouncilComposer.Text = "";
        CouncilConversation.ItemsSource = null; CouncilTasks.ItemsSource = null; CouncilOverall.Text = "New Council · choose models, folder and shared access";
        CouncilProgress.IsIndeterminate = false; CouncilProgress.Value = 0; CouncilHistory.SelectedIndex = -1;
        UpdateCouncilControls(); SaveCouncilDraft(); return Task.CompletedTask;
    });
    private async void CouncilHistorySelected(object sender, SelectionChangedEventArgs e)
    {
        if (councilLoading || CouncilHistory.SelectedItem is not CouncilSaved selected || selected.Id == councilId) return;
        await Act(async () =>
        {
            if (pendingCouncilStart != null || pendingCouncilMessage != null) throw new IOException("Retry the unconfirmed Council request before switching history.");
            if (councilId != null) councilDrafts[councilId] = CouncilComposer.Text;
            councilId = selected.Id; councilSelectionDirty = false; CouncilComposer.Text = councilDrafts.GetValueOrDefault(councilId, ""); councilRendered = "";
            await RefreshCouncilWorkspace(); SaveCouncilDraft();
        });
    }
    private void UpdateCouncilControls()
    {
        if (CouncilSend == null) return;
        bool editable = councilId == null && pendingCouncilStart == null && !acting;
        bool membersEditable = !councilBusy && !acting && pendingCouncilStart == null && pendingCouncilMessage == null;
        foreach (var slot in CouncilSlots) { slot.Enable.IsEnabled = slot.Models.IsEnabled = membersEditable && slot.Models.Items.Count > 0; }
        CouncilAccess.IsEnabled = CouncilBrowse.IsEnabled = editable;
        CouncilSend.IsEnabled = connected && !acting && (!councilBusy || pendingCouncilMessage != null);
        CouncilSend.Content = pendingCouncilMessage != null || pendingCouncilStart != null ? "Retry send" : "Send to Council ↗";
        CouncilCancel.IsEnabled = councilId != null && councilBusy && !acting;
        CouncilComposer.IsReadOnly = acting || pendingCouncilMessage != null || pendingCouncilStart != null;
        CouncilCoordinate.IsEnabled = pendingCouncilMessage == null;
    }
    private async void CouncilSendClicked(object sender, RoutedEventArgs e) => await Act(SendCouncilMessage);
    private JsonArray SelectedCouncilMembers()
    {
        var members = CouncilSlots.Where(s => s.Enable.IsChecked == true).Select(s => s.Models.SelectedItem as CouncilModel ?? throw new IOException("Choose a configured model.")).ToList();
        if (members.Count == 0) throw new IOException("Enable at least one configured API model.");
        return new JsonArray(members.Select(m => (JsonNode)new JsonObject { ["harness"] = m.Harness, ["model"] = m.Model }).ToArray());
    }
    private static string CouncilTitle(string text)
    {
        var result = new System.Text.StringBuilder(); int bytes = 0;
        foreach (var rune in text.EnumerateRunes().Take(60)) { if (bytes + rune.Utf8SequenceLength > 160) break; result.Append(rune.ToString()); bytes += rune.Utf8SequenceLength; }
        return result.ToString();
    }
    private async Task SendCouncilMessage()
    {
        if (client == null) return;
        string text = CouncilComposer.Text.Trim(); if (text.Length == 0) return;
        if (System.Text.Encoding.UTF8.GetByteCount(text) > 16000) throw new IOException("Keep a Council message within 16,000 UTF-8 bytes.");
        if (councilId == null)
        {
            pendingCouncilStart ??= new JsonObject { ["request_id"] = Json.Id(), ["title"] = CouncilTitle(text), ["members"] = SelectedCouncilMembers(), ["workspace"] = CouncilFolder.Text.Length == 0 ? null : CouncilFolder.Text, ["access"] = ComboTag(CouncilAccess), ["authorize_access"] = ComboTag(CouncilAccess) != "discuss" };
            SaveCouncilDraft();
            try { councilId = Json.Text(await client.Post("/v1/bee/councils", (JsonObject)pendingCouncilStart.DeepClone(), lifetime.Token), "council_id"); }
            catch (CoreApiException error) when (error.StatusCode is >= 400 and < 500) { pendingCouncilStart = null; SaveCouncilDraft(); throw; }
            pendingCouncilStart = null; SaveCouncilDraft();
        }
        pendingCouncilMessage ??= new JsonObject { ["request_id"] = Json.Id(), ["text"] = text, ["coordinate"] = CouncilCoordinate.IsChecked == true, ["members"] = SelectedCouncilMembers() };
        if (Json.Text(pendingCouncilMessage, "text") != text) throw new IOException("Retry the original unconfirmed message unchanged.");
        SaveCouncilDraft(); UpdateCouncilControls();
        try { await client.Post("/v1/bee/councils/" + councilId + "/messages", (JsonObject)pendingCouncilMessage.DeepClone(), lifetime.Token); }
        catch (CoreApiException error) when (error.StatusCode is >= 400 and < 500) { pendingCouncilMessage = null; SaveCouncilDraft(); throw; }
        pendingCouncilMessage = null; councilSelectionDirty = false; CouncilComposer.Clear(); councilDrafts.Remove(councilId); SaveCouncilDraft();
        await RefreshCouncilWorkspace(true);
    }
    private async void CouncilCancelClicked(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (client != null && councilId != null) { await client.Post("/v1/bee/councils/" + councilId + "/cancel", new JsonObject(), lifetime.Token); await RefreshCouncilWorkspace(); }
    });
    private async void EarlierCouncilMessages(object sender, RoutedEventArgs e) => await Act(() => RefreshCouncilWorkspace(before: councilBefore));
    private async void CouncilComposerKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && (Keyboard.Modifiers & ModifierKeys.Shift) == 0) { e.Handled = true; await Act(SendCouncilMessage); }
    }
    private void ChatProviderChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ProfileEndpoint == null || ProfileModel == null || ProfileNotice == null) return;
        string provider = ComboTag(ProfileProvider);
        ProfileModel.IsReadOnly = provider == "opencode";
        ProfileNotice.Text = provider == "opencode" ? "Limited-time free model via OpenCode. Use an OpenCode Console API key. Account access and provider limits apply. No switch to paid models if unavailable."
            : provider == "deepseek" ? "Direct DeepSeek API is billed per token. Use a DeepSeek API key; this is separate from OpenCode's free offer."
            : "";
        if (provider == "opencode") { ProfileModel.Text = "deepseek-v4.1-flash-free"; ProfileEndpoint.Text = "https://opencode.ai/inference/openai/v1/chat/completions"; }
        if (provider == "deepseek") { ProfileModel.Text = "deepseek-flash"; ProfileEndpoint.Text = "https://api.deepseek.com/chat/completions"; }
        ProfileEndpoint.IsReadOnly = provider is "opencode" or "deepseek";
        ProfileEndpointLabel.Text = provider is "opencode" or "deepseek" ? "Provider endpoint (preset)" : "Endpoint for Local or Qwen API (use your regional Model Studio URL)";
        if (provider == "dashscope" && !ProfileEndpoint.Text.Contains("aliyuncs.com", StringComparison.Ordinal)) ProfileEndpoint.Text = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1/chat/completions";
        if (provider == "local" && !ProfileEndpoint.Text.StartsWith("http://127.", StringComparison.Ordinal)) ProfileEndpoint.Text = "http://127.0.0.1:11434/v1/chat/completions";
    }
    private async Task RefreshCouncilWorkspace(bool history = false, int? before = null)
    {
        if (client == null || councilRefreshing || CouncilWorkspacePanel.Visibility != Visibility.Visible) return;
        councilRefreshing = true; var source = client; string? selectedId = councilId;
        try
        {
            if (history)
            {
                var saved = await source.Get("/v1/bee/councils", lifetime.Token);
                councilLoading = true;
                CouncilHistory.ItemsSource = Json.Array(saved, "councils").Select(c => new CouncilSaved(Json.Text(c, "id"), Json.Text(c, "title"), Json.Text(c, "state"))).ToList();
                CouncilHistory.SelectedItem = CouncilHistory.Items.OfType<CouncilSaved>().FirstOrDefault(c => c.Id == selectedId);
                councilLoading = false;
            }
            await RefreshHive(source, selectedId);
            await RefreshHive(source, selectedId);
            if (selectedId == null) return;
            var envelope = await source.Get("/v1/bee/councils/" + selectedId + (before.HasValue ? "?before=" + before : ""), lifetime.Token);
            var c = envelope["council"]!;
            if (selectedId != councilId || !ReferenceEquals(source, client)) return;
            if (councilCacheId != selectedId) { councilRounds.Clear(); councilCacheId = selectedId; }
            int offset = (int)Json.Number(envelope, "rounds_offset");
            var page = Json.Array(c, "rounds"); for (int i = 0; i < page.Count; i++) if (page[i] != null) councilRounds[offset + i] = page[i]!.DeepClone();
            councilBefore = councilRounds.Count == 0 ? 0 : councilRounds.Keys.First(); CouncilEarlier.IsEnabled = councilBefore > 0;
            var members = Json.Array(c, "members"); var rounds = councilRounds.Values.ToList(); var last = envelope["latest"];
            string state = Json.Text(last, "state", "idle");
            councilBusy = state is "broadcasting" or "planning" or "executing" or "synthesizing" or "cancelling";
            councilLoading = true;
            CouncilFolder.Text = OrdinaryPath(c["workspace"]?.GetValue<string>() ?? ""); SelectTag(CouncilAccess, Json.Text(c, "access"));
            foreach (var slot in CouncilSlots)
            {
                var member = members.FirstOrDefault(m => Json.Text(m, "harness") == slot.Harness || slot.Harness == "qwen" && Json.Text(m, "harness") == "local");
                if (!councilSelectionDirty) { slot.Enable.IsChecked = member != null; if (member != null) slot.Models.SelectedItem = slot.Models.Items.OfType<CouncilModel>().FirstOrDefault(m => m.Model == Json.Text(member, "model") && m.Harness == Json.Text(member, "harness")); }
                if (slot.Enable.IsChecked != true || (slot.Models.SelectedItem as CouncilModel)?.Model != Json.Text(member, "model")) member = null;
                int memberIndex = member == null ? -1 : members.IndexOf(member);
                var reply = Json.Array(last, "replies").FirstOrDefault(r => Json.Number(r, "member") == memberIndex);
                var tasks = Json.Array(last, "tasks").Where(t => Json.Number(t, "owner") == memberIndex).ToList();
                string modelState = tasks.Any(t => Json.Text(t, "state") == "running") ? "working" : tasks.Any(t => Json.Text(t, "state") == "queued") ? "queued" : Json.Text(tasks.LastOrDefault(), "state", state).Replace('_', ' ');
                slot.Status.Text = member == null ? slot.Models.Items.Count == 0 ? "Add a Chat connection" : slot.Enable.IsChecked == true ? "Ready for next message" : "Disabled" : tasks.Count > 0 ? tasks.Count(t => Json.Text(t, "state") == "completed") + "/" + tasks.Count + " tasks · " + modelState : Json.Text(reply, "state", "ready").Replace('_', ' ');
            }
            councilLoading = false;
            var taskRows = Json.Array(last, "tasks"); int done = taskRows.Count(t => Json.Text(t, "state") == "completed");
            CouncilOverall.Text = state.Replace('_', ' ').ToUpperInvariant() + (taskRows.Count > 0 ? " · " + done + "/" + taskRows.Count + " microtasks reported complete" : Json.Flag(last, "coordinate") ? " · first enabled model coordinates" : " · broadcast to enabled models");
            CouncilProgress.IsIndeterminate = councilBusy && taskRows.Count == 0; CouncilProgress.Value = taskRows.Count == 0 ? 0 : 100.0 * done / taskRows.Count;
            string rendered = c.ToJsonString() + ":" + councilRounds.Count;
            if (rendered != councilRendered)
            {
                var messages = new List<ChatMessage>();
                foreach (var round in rounds)
                {
                    var roundMembers = Json.Array(round, "members"); if (roundMembers.Count == 0) roundMembers = members;
                    messages.Add(new ChatMessage { Role = "YOU", Detail = Json.Text(round, "state").Replace('_', ' '), Text = Json.Text(round, "text"), SourceSession = selectedId });
                    foreach (var reply in Json.Array(round, "replies"))
                    {
                        int owner = (int)Json.Number(reply, "member"); string model = owner < roundMembers.Count ? Json.Text(roundMembers[owner], "model") : "Model";
                        messages.Add(new ChatMessage { Role = model, Detail = Json.Text(reply, "state"), Text = Json.Text(reply, "text", "Waiting for response"), SourceSession = selectedId });
                    }
                    if (Json.Text(round, "summary").Length > 0) messages.Add(new ChatMessage { Role = "COUNCIL", Detail = "Shared result · review verification claims", Text = Json.Text(round, "summary"), SourceSession = selectedId });
                }
                CouncilConversation.ItemsSource = messages;
                CouncilTasks.ItemsSource = taskRows.Select((t, index) => {
                    int owner = (int)Json.Number(t, "owner"); string model = owner < members.Count ? Json.Text(members[owner], "model") : "Model";
                    long started = Json.Number(t, "started_at"), finished = Json.Number(t, "finished_at");
                    string timing = started > 0 ? finished > 0 ? "Elapsed " + TimeSpan.FromMilliseconds(Math.Max(0, finished - started)).ToString(@"mm\:ss") : "Started " + DateTimeOffset.FromUnixTimeMilliseconds(started).ToLocalTime().ToString("T") : "Queued";
                    return new CouncilTaskView(Json.Text(t, "title"), model, Json.Text(t, "state").Replace('_', ' '), Json.Text(t, "activity") + " · step " + Json.Number(t, "steps"), Json.Text(t, "result"), string.Join("\n", Json.Array(t, "operations").Select(o => Json.Text(o, "operation") + " " + Json.Text(o, "path") + " · " + Json.Text(o, "state"))), timing, Json.Text(t, "state") == "running", Json.Text(t, "state") == "completed" ? 100 : 0, index, Json.Text(last, "request_id"), Json.Text(c, "access") == "edit");
                }).ToList();
                councilRendered = rendered;
            }
        }
        finally { councilRefreshing = false; councilLoading = false; UpdateCouncilControls(); }
    }
    private void CouncilInputChanged(object sender, RoutedEventArgs e)
    {
        if (councilLoading || CouncilHistory == null) return;
        if (CouncilSlots.Any(s => ReferenceEquals(sender, s.Enable) || ReferenceEquals(sender, s.Models))) councilSelectionDirty = true;
        try { SaveCouncilDraft(); } catch (Exception error) when (error is IOException or UnauthorizedAccessException) { CouncilNotice.Text = SafeError(error); }
    }
    private string CouncilDraftPath() => Path.ChangeExtension(OutboxPath(), ".council.json");
    private void SaveCouncilDraft()
    {
        if (client == null || councilLoading || councilIdentity != client.Identity) return;
        if (councilId != null) councilDrafts[councilId] = CouncilComposer.Text;
        var drafts = new JsonObject(); foreach (var pair in councilDrafts) drafts[pair.Key] = pair.Value;
        var slots = new JsonArray(CouncilSlots.Select(s => (JsonNode)new JsonObject { ["harness"] = (s.Models.SelectedItem as CouncilModel)?.Harness, ["model"] = (s.Models.SelectedItem as CouncilModel)?.Model, ["enabled"] = s.Enable.IsChecked == true }).ToArray());
        var saved = new JsonObject { ["selected"] = councilId, ["text"] = CouncilComposer.Text, ["drafts"] = drafts, ["slots"] = slots, ["selection_dirty"] = councilSelectionDirty, ["workspace"] = CouncilFolder.Text, ["access"] = ComboTag(CouncilAccess), ["coordinate"] = CouncilCoordinate.IsChecked == true, ["start"] = pendingCouncilStart?.DeepClone(), ["message"] = pendingCouncilMessage?.DeepClone(), ["hive_start"] = pendingHiveStart?.DeepClone(), ["hive_action"] = pendingHiveAction?.DeepClone(), ["hive_route"] = pendingHiveRoute, ["hive_start"] = pendingHiveStart?.DeepClone(), ["hive_action"] = pendingHiveAction?.DeepClone(), ["hive_route"] = pendingHiveRoute };
        string path = CouncilDraftPath(); File.WriteAllText(path + ".pending", saved.ToJsonString()); File.Move(path + ".pending", path, true);
    }
    private void RestoreCouncilDraft()
    {
        if (client == null || councilIdentity == client.Identity) return;
        councilLoading = true;
        try
        {
            councilIdentity = client.Identity; councilId = null; pendingCouncilStart = pendingCouncilMessage = null; pendingHiveStart = pendingHiveAction = null; pendingHiveRoute = null; pendingHiveStart = pendingHiveAction = null; pendingHiveRoute = null; councilDrafts.Clear(); councilRendered = "";
            if (!File.Exists(CouncilDraftPath())) return;
            var saved = JsonNode.Parse(File.ReadAllText(CouncilDraftPath()));
            councilId = Json.Text(saved, "selected") is { Length: > 0 } id ? id : null;
            pendingCouncilStart = saved?["start"]?.DeepClone() as JsonObject; pendingCouncilMessage = saved?["message"]?.DeepClone() as JsonObject;
            pendingHiveStart = saved?["hive_start"]?.DeepClone() as JsonObject; pendingHiveAction = saved?["hive_action"]?.DeepClone() as JsonObject; pendingHiveRoute = Json.Text(saved, "hive_route");
            pendingHiveStart = saved?["hive_start"]?.DeepClone() as JsonObject; pendingHiveAction = saved?["hive_action"]?.DeepClone() as JsonObject; pendingHiveRoute = Json.Text(saved, "hive_route");
            councilSelectionDirty = Json.Flag(saved, "selection_dirty");
            CouncilComposer.Text = Json.Text(saved, "text"); CouncilFolder.Text = Json.Text(saved, "workspace"); SelectTag(CouncilAccess, Json.Text(saved, "access", "discuss")); CouncilCoordinate.IsChecked = Json.Flag(saved, "coordinate");
            if (saved?["drafts"] is JsonObject drafts) foreach (var pair in drafts) councilDrafts[pair.Key] = pair.Value?.GetValue<string>() ?? "";
            var slots = Json.Array(saved, "slots");
            for (int i = 0; i < Math.Min(slots.Count, CouncilSlots.Length); i++) { var slot = CouncilSlots[i]; slot.Models.SelectedItem = slot.Models.Items.OfType<CouncilModel>().FirstOrDefault(m => m.Harness == Json.Text(slots[i], "harness") && m.Model == Json.Text(slots[i], "model")); slot.Enable.IsChecked = Json.Flag(slots[i], "enabled") && slot.Models.SelectedItem != null; }
        }
        finally { councilLoading = false; }
    }
    private async Task CouncilWorkspaceSmokeTest(string output)
    {
        ShowChatSetup(this, new RoutedEventArgs());
        SelectTag(ProfileProvider, "opencode");
        if (ProfileModel.Text != "deepseek-v4.1-flash-free" || !ProfileModel.IsReadOnly || ProfileEndpoint.Text != "https://opencode.ai/inference/openai/v1/chat/completions" || !ProfileNotice.Text.Contains("OpenCode Console API key")) throw new IOException("Free DeepSeek connection preset is incorrect.");
        await Capture(Path.Combine(output, "cloud-bee-deepseek-free.png"));
        SelectTag(ProfileProvider, "deepseek");
        if (ProfileModel.Text != "deepseek-flash" || ProfileModel.IsReadOnly || ProfileEndpoint.Text != "https://api.deepseek.com/chat/completions" || !ProfileNotice.Text.Contains("billed per token")) throw new IOException("Direct DeepSeek connection preset is incorrect.");
        CloseChatSetup(this, new RoutedEventArgs());
        await OpenCouncilWorkspace();
        foreach (var slot in CouncilSlots)
        {
            string model = slot.Harness switch { "codex" => "gpt-fixture", "grok" => "grok-fixture", "claude" => "claude-fixture", "deepseek" => "deepseek-v4.1-flash-free", _ => "qwen-fixture" };
            slot.Models.SelectedItem = slot.Models.Items.OfType<CouncilModel>().FirstOrDefault(m => m.Model == model) ?? throw new IOException("Council fixture model missing: " + model);
            slot.Enable.IsChecked = true;
        }
        CouncilFolder.Text = Path.Combine(Path.GetDirectoryName(Path.GetFullPath(ConfigPathBox.Text))!, "repository");
        SelectTag(CouncilAccess, "read"); CouncilCoordinate.IsChecked = true;
        CouncilComposer.Text = "Inspect the shared project and coordinate microtasks";
        try { await SendCouncilMessage(); throw new IOException("Council fixture did not drop acceptance."); }
        catch (Exception error) when (error is System.Net.Http.HttpRequestException or IOException && error is not CoreApiException) { if (pendingCouncilMessage == null) throw; }
        string pending = pendingCouncilMessage!.ToJsonString(), identity = councilId!;
        councilIdentity = null; RestoreCouncilDraft();
        if (councilId != identity || pendingCouncilMessage?.ToJsonString() != pending) throw new IOException("Council retry changed during reconnect recovery.");
        await SendCouncilMessage();
        var deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (CouncilTasks.Items.OfType<CouncilTaskView>().Any(t => t.Busy)) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        if (!CouncilTasks.Items.OfType<CouncilTaskView>().Any(t => t.Busy) || !councilBusy || CouncilConversation.Items.OfType<ChatMessage>().Count(m => m.Role.EndsWith("-fixture", StringComparison.Ordinal) || m.Role == "deepseek-v4.1-flash-free") != 5) throw new IOException("Council did not show all model replies and live microtask progress.");
        string actualFolder = CouncilFolder.Text;
        CouncilFolder.Text = "Disposable fixture workspace · shared by all models";
        await Capture(Path.Combine(output, "cloud-bee-council-coordinated.png"));
        CouncilFolder.Text = actualFolder;
        await client!.Post("/v1/fixture/council/release", new JsonObject());
        deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (!councilBusy) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        if (councilBusy || CouncilTasks.Items.Count != 5 || CouncilTasks.Items.OfType<CouncilTaskView>().Any(t => t.State != "completed") || CouncilProgress.Value != 100 || !CouncilConversation.Items.OfType<ChatMessage>().Any(m => m.Role == "COUNCIL" && m.Text.Contains("shared result"))) throw new IOException("Council dependency results or shared synthesis did not render.");
        CouncilComposer.Text = "Retained Council draft — 蜂"; SaveCouncilDraft(); councilIdentity = null; RestoreCouncilDraft(); await RefreshCouncilWorkspace(true);
        if (CouncilComposer.Text != "Retained Council draft — 蜂" || councilId != identity || ComboTag(CouncilAccess) != "read" || CouncilSlots.Any(s => s.Enable.IsChecked != true)) throw new IOException("Council reconnect lost selections, shared access or drafts.");
        await client.Post("/v1/fixture/council/hold", new JsonObject());
        CouncilComposer.Text = "Another coordinated request"; await SendCouncilMessage();
        deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (CouncilTasks.Items.OfType<CouncilTaskView>().Any(t => t.Busy)) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        await client.Post("/v1/bee/councils/" + councilId + "/cancel", new JsonObject());
        deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (!councilBusy) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        if (!CouncilOverall.Text.StartsWith("CANCELLED", StringComparison.Ordinal)) throw new IOException("Council cancellation did not settle its owned workers.");
        await client.Post("/v1/fixture/council/release", new JsonObject());
        CouncilGrokOn.IsChecked = CouncilClaudeOn.IsChecked = CouncilDeepSeekOn.IsChecked = false; CouncilCoordinate.IsChecked = false;
        await RefreshCouncilWorkspace();
        if (CouncilGrokOn.IsChecked == true || CouncilClaudeOn.IsChecked == true) throw new IOException("Polling overwrote the next message's model selection.");
        CouncilComposer.Text = "Only GPT and Qwen this time"; await SendCouncilMessage();
        deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (!councilBusy) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        var latest = CouncilConversation.Items.OfType<ChatMessage>().TakeLast(3).ToList();
        if (latest.Count != 3 || latest[0].Role != "YOU" || latest[1].Role != "gpt-fixture" || latest[2].Role != "qwen-fixture" || CouncilConversation.Items.OfType<ChatMessage>().Count(m => m.Role == "claude-fixture") != 2) throw new IOException("Changed Council membership lost prior replies or sent to a disabled model.");
        DismissCouncilWorkspace(this, new RoutedEventArgs());
        await File.WriteAllTextAsync(Path.Combine(output, "council-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "five provider selectors including DeepSeek Flash Free, shared broadcast, immutable lost-acceptance retry, live microtask progress and dependencies, synthesis, folder/access, cancellation, saved history and drafts" }.ToJsonString());
    }
}
