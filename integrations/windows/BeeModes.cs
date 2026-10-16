using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;

namespace Cloudroom.Desktop;

public partial class MainWindow
{
    private readonly Dictionary<string, string> modes = [], groupIds = [], workStates = [];
    private JsonObject? pendingSwitch;
    private bool councilView;
    private string switchMode = "chat";
    private readonly Dictionary<string, Transcript> councilHistories = [];
    private readonly Dictionary<string, string> councilModes = [];

    private string OutboxPath() => Path.Combine(Path.GetDirectoryName(Path.GetFullPath(ConfigPathBox.Text))!, "bee-desktop-" + client!.Identity + ".json");
    private void SaveOutbox()
    {
        SaveLibraryDraft();
        if (client == null) return;
        if (sessionId != null) drafts[sessionId] = Composer.Text;
        var starts = new JsonArray();
        foreach (var start in pendingStarts ?? []) starts.Add(start.DeepClone());
        var prompts = new JsonObject();
        foreach (var pair in pendingPrompts) prompts[pair.Key] = pair.Value.DeepClone();
        var savedDrafts = new JsonObject();
        foreach (var pair in drafts) savedDrafts[pair.Key] = pair.Value;
        var saved = new JsonObject { ["starts"] = starts, ["prompts"] = prompts, ["drafts"] = savedDrafts,
            ["draft_id"] = draftId, ["draft_text"] = draftText, ["selected"] = sessionId };
        string path = OutboxPath();
        File.WriteAllText(path + ".pending", saved.ToJsonString());
        File.Move(path + ".pending", path, true);
    }
    private void RestoreOutbox()
    {
        if (client == null || !File.Exists(OutboxPath())) return;
        var saved = JsonNode.Parse(File.ReadAllText(OutboxPath()));
        pendingStarts = Json.Array(saved, "starts").OfType<JsonObject>().Select(s => (JsonObject)s.DeepClone()).ToList();
        if (pendingStarts.Count == 0) pendingStarts = null;
        pendingPrompts.Clear();
        if (saved?["prompts"] is JsonObject prompts)
            foreach (var pair in prompts) if (pair.Value is JsonObject prompt) pendingPrompts[pair.Key] = (JsonObject)prompt.DeepClone();
        if (saved?["drafts"] is JsonObject savedDrafts)
            foreach (var pair in savedDrafts) drafts[pair.Key] = pair.Value?.GetValue<string>() ?? "";
        draftId = Json.Text(saved, "draft_id") is string id && id.Length > 0 ? id : null;
        draftText = Json.Text(saved, "draft_text");
        string selected = Json.Text(saved, "selected");
        if (draftId != null && pendingPrompts.Count == 0) { draftId = null; draftText = null; drafts.Remove(selected); }
        if (SessionList.Items.OfType<SessionItem>().Any(item => item.Id == selected)) { sessionId = null; SelectSession(selected); }
    }
    private static bool IsBee(string? id) => id?.StartsWith("bee_", StringComparison.Ordinal) == true;
    private static string SessionPath(string id) => (IsBee(id) ? "/v1/bee/sessions/" : "/v1/sessions/") + Uri.EscapeDataString(id);
    private bool ChatAvailable(string harness, string model) => Json.Array(capabilities, "harnesses").Any(h => Json.Text(h, "id") == harness && Json.Array(h, "chat_models").Any(m => m?.GetValue<string>() == model));
    private bool WorkAvailable(string harness, string model) => Json.Array(capabilities, "harnesses").Any(h => Json.Text(h, "id") == harness && Json.Flag(h, "work") && Json.Array(h, "work_models").Any(m => m?.GetValue<string>() == model));

    private void UpdateModes()
    {
        if (ChatModeButton == null) return;
        string harness = sessionId == null ? HarnessPicker.SelectedItem as string ?? "codex" : sessionHarnesses.GetValueOrDefault(sessionId, "codex");
        string model = sessionId == null ? ModelPicker.SelectedItem as string ?? "" : sessionModels.GetValueOrDefault(sessionId, "");
        string mode = sessionId == null ? "chat" : modes.GetValueOrDefault(sessionId, "work");
        bool bee = sessionId == null || IsBee(sessionId);
        LegacyWorkCheck.Visibility = harness is "grok" or "qwen" ? Visibility.Visible : Visibility.Collapsed;
        if (LegacyWorkCheck.Visibility == Visibility.Collapsed) LegacyWorkCheck.IsChecked = false;
        bool chat = bee && ChatAvailable(harness, model), work = bee && WorkAvailable(harness, model);
        ChatModeButton.Content = (mode == "chat" ? "●  " : "") + "Chat" + (chat ? "" : " · unavailable");
        WorkModeButton.Content = (mode == "work" ? "●  " : "") + "Work" + (bee && !work ? " · unavailable" : "");
        ChatModeButton.IsEnabled = connected && !acting && chat;
        WorkModeButton.IsEnabled = connected && !acting && work;
        ModeNote.Text = !bee ? "Existing agent session · Work uses this Windows account. Start a new Cloud Bee conversation for managed modes."
            : mode == "chat" ? chat ? "CHAT · Text only. Tools and execution are disabled." : "CHAT unavailable · Configure a separate API or local model connection."
            : "WORK · Authorized workspace. Unsupported approval requests are denied. Windows account isolation is not provided.";
        BackgroundWork.Text = sessionId != null && mode == "chat" && workStates.TryGetValue(sessionId, out var state) && state.Length > 0 ? "Background Work: " + state.Replace('_', ' ') + " · use Interrupt to cancel an active turn" : "";
        if (sessionId == null || IsBee(sessionId))
            SendButton.IsEnabled = connected && !acting && (mode == "chat" ? chat : Json.Text(current, "state") is "idle" or "running" or "starting_turn" or "sleeping") && Json.Text(current, "state") != "closed";
        if (IsBee(sessionId) && mode == "chat")
        {
            PauseButton.IsEnabled = false;
            InterruptButton.IsEnabled = connected && !acting && workStates.GetValueOrDefault(sessionId!) is "running" or "starting_turn";
        }
    }

    private async void ChangeMode(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (client == null) return;
        if (sessionId == null) await CreateSession();
        if (!IsBee(sessionId)) throw new IOException("Start a new Cloud Bee conversation for Chat / Work modes.");
        switchMode = (string)((Button)sender).Tag;
        if (modes.GetValueOrDefault(sessionId!) == switchMode && (switchMode != "work" || Json.Text(current, "execution_session_id").Length > 0)) return;
        PrepareHandoff(switchMode);
    });

    private void PrepareHandoff(string mode)
    {
        switchMode = mode;
        HandoffTitle.Text = switchMode == "work" ? "Authorize Work" : "Return to Chat";
        HandoffWorkspace.Text = folder ?? Json.Text(current?["workspace"], "path");
        var discussion = transcript.Messages.Where(message => message.IsMessage && message.Role != "CORE").TakeLast(4).ToList();
        static string Compact(string text, int limit) => text.Length <= limit ? text : text[..limit] + "…";
        string lastUser = discussion.LastOrDefault(message => message.Role == "YOU")?.Text ?? "";
        string lastAnswer = discussion.LastOrDefault(message => message.Role != "YOU")?.Text ?? "";
        HandoffObjective.Text = Compact(Composer.Text.Trim().Length > 0 ? Composer.Text.Trim() : switchMode == "chat" ? lastAnswer : lastUser, 1200);
        HandoffContext.Text = Compact(string.Join("\n\n", discussion.Select(message => message.Role + ": " + Compact(message.Text, 900))), 4000);
        HandoffRequirements.Text = "";
        HandoffConstraints.Text = "";
        HandoffAcceptance.Text = "";
        HandoffArtifacts.Text = "";
        WorkAuthorization.IsChecked = false;
        WorkAuthorization.Visibility = switchMode == "work" ? Visibility.Visible : Visibility.Collapsed;
        pendingSwitch = null;
        HandoffDialog.Visibility = Visibility.Visible;
    }

    private async Task ApplyMode()
    {
        if (client == null || sessionId == null) return;
        if (switchMode == "work" && WorkAuthorization.IsChecked != true) throw new IOException("Select the Work authorization checkbox after reviewing the workspace and handoff.");
        pendingSwitch ??= new JsonObject
        {
            ["request_id"] = Json.Id(), ["mode"] = switchMode, ["authorize_work"] = switchMode == "work",
            ["handoff"] = new JsonObject { ["objective"] = HandoffObjective.Text, ["context"] = HandoffContext.Text,
                ["requirements"] = HandoffRequirements.Text, ["constraints"] = HandoffConstraints.Text,
                ["acceptance"] = HandoffAcceptance.Text, ["artifacts"] = HandoffArtifacts.Text }
        };
        await client.Post(SessionPath(sessionId) + "/mode", pendingSwitch, lifetime.Token);
        pendingSwitch = null;
        modes[sessionId] = switchMode;
        transcript.Reset();
        selectionVersion++;
        councilView = false;
        CouncilPanel.Visibility = Visibility.Collapsed;
        HandoffDialog.Visibility = Visibility.Collapsed;
        if (switchMode == "work") await WaitUntilReady(client, sessionId);
        await Poll();
    }
    private async void ConfirmMode(object sender, RoutedEventArgs e) => await Act(ApplyMode);
    private void CancelHandoff(object sender, RoutedEventArgs e) { HandoffDialog.Visibility = Visibility.Collapsed; pendingSwitch = null; }

    private void ShowChatSetup(object sender, RoutedEventArgs e)
    {
        ProfileModel.Text = ModelPicker.SelectedItem as string ?? "";
        ChatSetupDialog.Visibility = Visibility.Visible;
    }
    private void CloseChatSetup(object sender, RoutedEventArgs e) { ProfileKey.Clear(); ChatSetupDialog.Visibility = Visibility.Collapsed; }
    private async void SaveChatSetup(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (client == null) return;
        string provider = (ProfileProvider.SelectedItem as ComboBoxItem)?.Tag as string ?? "openai";
        string harness = provider == "openai" ? "codex" : provider == "xai" ? "grok" : "local";
        string endpoint = provider == "openai" ? "https://api.openai.com/v1/chat/completions" : provider == "xai" ? "https://api.x.ai/v1/chat/completions" : ProfileEndpoint.Text.Trim();
        await client.Post("/v1/bee/profiles", new JsonObject { ["provider"] = provider, ["harness"] = harness, ["model"] = ProfileModel.Text.Trim(), ["endpoint"] = endpoint, ["api_key"] = ProfileKey.Password }, lifetime.Token);
        ProfileKey.Clear();
        ChatSetupDialog.Visibility = Visibility.Collapsed;
        // Refresh capabilities without reconnecting, losing drafts, or interrupting Work.
        capabilities = await client.Get("/v1/bee/capabilities", lifetime.Token);
        ApplyCapabilities(capabilities);
        if (sessionId == null) { HarnessPicker.SelectedItem = harness; ModelPicker.SelectedItem = ProfileModel.Text.Trim(); }
        else { SelectSession(sessionId); await Poll(); }
        UpdateModes();
    });

    private async void ShowCouncil(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        councilView = !councilView;
        CouncilPanel.Visibility = councilView ? Visibility.Visible : Visibility.Collapsed;
        if (councilView) await RefreshCouncil();
    });
    private async Task RefreshCouncil()
    {
        if (client == null || !councilView) return;
        var group = ActiveGroup() ?? (sessionId == null ? [] : new List<string> { sessionId });
        var messages = new List<ChatMessage>();
        foreach (string member in group)
        {
            var state = (await client.Get(SessionPath(member), lifetime.Token))["session"];
            string model = sessionModels.GetValueOrDefault(member, member);
            string mode = Json.Text(state, "mode", "work");
            if (!councilHistories.TryGetValue(member, out var history) || councilModes.GetValueOrDefault(member) != mode)
            {
                history = new Transcript(); councilHistories[member] = history; councilModes[member] = mode;
            }
            for (int page = 0; page < 4; page++)
            {
                var records = Json.Array(await client.Get(SessionPath(member) + "/events?after=" + history.Cursor, lifetime.Token), "events");
                foreach (var record in records) if (record != null) history.Apply(record);
                if (records.Count < 256) break;
            }
            messages.Add(new ChatMessage { SourceSession = member, Role = model.ToUpperInvariant(), Detail = mode.ToUpperInvariant() + " · " + Json.Text(state, "state"), Text = history.Messages.LastOrDefault(m => m.Role != "YOU" && !m.IsTool)?.Text ?? "No answer yet. Select this model's tab to set its mode or inspect its full history." });
        }
        CouncilAnswers.ItemsSource = messages;
    }

    private async Task BeeSmokeTest(string output)
    {
        if (client == null || !ChatAvailable("codex", "fixture")) throw new IOException("Cloud Bee smoke requires the configured HTTP fixture.");
        string group = Json.Id();
        var ids = new List<string>();
        foreach (string model in new[] { "fixture", "basic" })
        {
            var start = new JsonObject { ["request_id"] = Json.Id(), ["harness"] = "codex", ["model"] = model,
                ["workspace"] = "bee_" + Json.Id(), ["workspace_name"] = "bee-" + model, ["group"] = group };
            ids.Add(Json.Text(await client.Post("/v1/bee/sessions", start), "session_id"));
        }
        await RefreshSessions();
        SelectSession(ids[0]);
        await Poll();
        if (modes[ids[0]] != "chat" || !ChatModeButton.Content.ToString()!.Contains("●")) throw new IOException("Chat selection is not visible.");
        SendAllCheck.IsChecked = true;
        Composer.Text = "Discuss our design";
        await SendDraft();
        await WaitForState(s => Json.Text(s, "state") == "idle");
        await Poll();
        if (!transcript.Messages.Any(m => m.Text.Contains("bee hello"))) throw new IOException("Chat did not render text-only output.");
        HeaderSubtitle.Text = "Deterministic fixture workspace · native Windows";
        await Capture(Path.Combine(output, "cloud-bee-chat.png"));
        PrepareHandoff("work");
        if (!HandoffContext.Text.Contains("Discuss our design") || WorkAuthorization.IsChecked == true) throw new IOException("The handoff did not prepare discussion context for explicit review.");
        HandoffObjective.Text = "Implement approved design";
        HandoffContext.Text = "Discuss our design";
        HandoffRequirements.Text = "Preserve the design";
        HandoffConstraints.Text = "Use the selected workspace";
        HandoffAcceptance.Text = "Verify fixture output";
        HandoffArtifacts.Text = "design.txt";
        WorkAuthorization.IsChecked = true;
        pendingSwitch = null;
        await ApplyMode();
        await WaitForState(s => Json.Text(s, "state") == "idle");
        SendAllCheck.IsChecked = false;
        Composer.Text = "Implement approved design";
        await SendDraft();
        await WaitForState(s => Json.Text(s, "state") == "idle" && s?["receipts"] is JsonObject receipts && receipts.Any(r => Json.Text(r.Value, "command") == "prompt" && Json.Text(r.Value, "state") == "completed"));
        if (modes[ids[1]] != "chat") throw new IOException("Work selection changed another model's mode.");
        HeaderSubtitle.Text = "Deterministic fixture workspace · native Windows";
        await Capture(Path.Combine(output, "cloud-bee-work.png"));
        Composer.Text = "BEE_FIXTURE_HOLD";
        await SendDraft();
        await WaitForState(s => Json.Text(s, "state") == "running");
        if (!InterruptButton.IsEnabled) throw new IOException("Work cancellation is not available in the GUI.");
        switchMode = "chat";
        HandoffObjective.Text = "Discuss running work";
        pendingSwitch = null;
        await ApplyMode();
        await Poll();
        if (Json.Text(current, "work_state") != "running" || !InterruptButton.IsEnabled || BackgroundWork.Text.Length == 0) throw new IOException("Chat hid or stopped background Work.");
        await SessionCommand("interrupt", new JsonObject());
        await WaitForState(s => Json.Text(s, "work_state") == "idle");
        switchMode = "work";
        pendingSwitch = null;
        await ApplyMode();
        SelectSession(ids[1]); await Poll();
        if (modes[ids[1]] != "chat") throw new IOException("Switching tabs lost independent modes.");
        SelectSession(ids[0]); await Poll();
        if (modes[ids[0]] != "work") throw new IOException("Returning to a tab lost Work mode.");
        councilView = true;
        CouncilPanel.Visibility = Visibility.Visible;
        await RefreshCouncil();
        HeaderSubtitle.Text = "Deterministic fixture workspace · native Windows";
        await Capture(Path.Combine(output, "cloud-bee-council.png"));
        await Connect();
        SelectSession(ids[0]); await Poll();
        if (ActiveGroup()?.Count != 2 || modes[ids[0]] != "work" || modes[ids[1]] != "chat") throw new IOException("Reconnect lost Council membership or modes.");
        await File.WriteAllTextAsync(Path.Combine(output, "bee-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "tool-free Chat, reviewed Work handoff, independent model modes, Council comparison, durable reconnect" }.ToJsonString());
    }
}
