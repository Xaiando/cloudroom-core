using System.Globalization;
using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;

namespace Cloudroom.Desktop;

public sealed record AutomationItem(string Id, string Name, string Description, bool Enabled, long Revision)
{
    public string PauseLabel => Enabled ? "Pause" : "Enable";
}
public sealed record PluginItem(string Id, string Name, string Description, bool Installed, bool Enabled, string Status)
{
    public string ActionLabel => !Installed ? "Install" : Enabled ? "Disable" : "Enable";
}
public sealed record AutomationRunItem(string Id, string Label, string Detail, JsonNode State);
public sealed record AutomationWorkspaceItem(string Id, string Label);

public partial class MainWindow
{
    private bool refreshingExtensions;
    private string? selectedPlugin, editingAutomation;
    private long editingRevision;
    private string editorRequestId = "";
    private DateTime nextExtensions = DateTime.MinValue;
    private readonly Dictionary<string, JsonObject> extensionOutbox = [];
    private List<PluginItem> pluginItems = [];
    private JsonNode? pluginState;

    private static string TimeLabel(JsonNode? value, string key)
    {
        long time = Json.Number(value, key);
        return time > 0 ? DateTimeOffset.FromUnixTimeMilliseconds(time).ToLocalTime().ToString("g") : "—";
    }
    private async Task<JsonNode> ExtensionPost(string path, JsonObject body)
    {
        if (client == null) throw new IOException("Connect to Core first.");
        if (!extensionOutbox.TryGetValue(path, out var pending))
        {
            pending = (JsonObject)body.DeepClone();
            if (!pending.ContainsKey("request_id")) pending["request_id"] = Json.Id();
            extensionOutbox[path] = pending;
        }
        try
        {
            var result = await client.Post(path, pending, lifetime.Token);
            extensionOutbox.Remove(path);
            nextExtensions = DateTime.MinValue;
            return result;
        }
        catch (CoreApiException) { extensionOutbox.Remove(path); throw; }
    }
    private async Task OpenExtensions(bool plugins)
    {
        if (client == null || !connected) throw new IOException("Connect to Core first.");
        ExtensionsPane.Visibility = Visibility.Visible;
        ConversationPane.Visibility = Visibility.Collapsed;
        AutomationsView.Visibility = plugins ? Visibility.Collapsed : Visibility.Visible;
        PluginsView.Visibility = plugins ? Visibility.Visible : Visibility.Collapsed;
        LibraryView.Visibility = Visibility.Collapsed;
        ExtensionTitle.Text = plugins ? "Plugins" : "Automations";
        ExtensionSubtitle.Text = plugins ? "Original Cloudroom features, adapted for native Windows." : "Schedule scripts and authorized Work tasks. Core must be running and Windows awake.";
        ExtensionsError.Text = "";
        nextExtensions = DateTime.MinValue;
        await RefreshExtensions();
    }
    private async void ShowAutomations(object sender, RoutedEventArgs e) => await Act(() => OpenExtensions(false));
    private async void ShowPlugins(object sender, RoutedEventArgs e) => await Act(() => OpenExtensions(true));
    private void BackToConversation(object sender, RoutedEventArgs e)
    {
        ExtensionsPane.Visibility = Visibility.Collapsed;
        ConversationPane.Visibility = Visibility.Visible;
    }
    private async void RefreshExtensionsClicked(object sender, RoutedEventArgs e) => await Act(async () => { nextExtensions = DateTime.MinValue; await RefreshExtensions(); });

    private async Task RefreshExtensions()
    {
        if (LibraryView.Visibility == Visibility.Visible) return;
        if (client == null || DateTime.UtcNow < nextExtensions) return;
        var automations = await client.Get("/v1/bee/automations", lifetime.Token);
        string? selected = (AutomationList.SelectedItem as AutomationItem)?.Id;
        var items = Json.Array(automations, "automations").Select(item =>
        {
            string schedule = ScheduleLabel(item?["schedule"]);
            string state = Json.Text(item?["last_run"], "state");
            string description = Json.Text(item, "execution_type").ToUpperInvariant() + " · " + schedule + " · Next: " + TimeLabel(item, "next_run_at") + (state.Length > 0 ? " · " + state.Replace('_', ' ') : "");
            return new AutomationItem(Json.Text(item, "id"), Json.Text(item, "name"), description, Json.Flag(item, "enabled"), Json.Number(item, "revision"));
        }).ToList();
        refreshingExtensions = true;
        AutomationList.ItemsSource = items;
        AutomationList.SelectedItem = items.FirstOrDefault(item => item.Id == selected) ?? items.FirstOrDefault();
        var workspaces = new List<AutomationWorkspaceItem> { new("", "Default · " + Json.Text(automations, "default_workspace")) };
        workspaces.AddRange(Json.Array(automations, "workspaces").Select(w => new AutomationWorkspaceItem(Json.Text(w, "id"), Json.Text(w, "path"))));
        string workspace = AutomationWorkspace.SelectedValue as string ?? "";
        AutomationWorkspace.ItemsSource = workspaces;
        AutomationWorkspace.SelectedValue = workspaces.Any(w => w.Id == workspace) ? workspace : "";
        refreshingExtensions = false;
        if (!Json.Flag(automations, "plugin_enabled")) ExtensionSubtitle.Text = "Automations plugin is disabled. Enable it in Plugins to run tasks; saved schedules and history are preserved.";
        if (AutomationList.SelectedItem is AutomationItem picked) await LoadAutomationRuns(picked.Id);
        pluginState = await client.Get("/v1/bee/plugins", lifetime.Token);
        pluginItems = Json.Array(pluginState, "plugins").Select(p => new PluginItem(Json.Text(p, "id"), Json.Text(p, "name"), Json.Text(p, "description"), Json.Flag(p, "installed"), Json.Flag(p, "enabled"), Json.Text(p, "runtime_status") + " · " + Json.Text(p, "version"))).ToList();
        ApplyPluginFilter();
        nextExtensions = DateTime.UtcNow.AddSeconds(3);
    }
    private static string ScheduleLabel(JsonNode? schedule) => Json.Text(schedule, "type") switch
    {
        "cron" => Json.Text(schedule, "expression") + " · " + Json.Text(schedule, "timezone"),
        "interval" => "Every " + (Json.Number(schedule, "every_ms") / 60000d).ToString("0.##", CultureInfo.InvariantCulture) + " min",
        "once" => "One time: " + TimeLabel(schedule, "at_ms"),
        _ => "Unknown schedule"
    };
    private async Task LoadAutomationRuns(string id)
    {
        if (client == null) return;
        var detail = await client.Get("/v1/bee/automations/" + id, lifetime.Token);
        string? selected = (AutomationRuns.SelectedItem as AutomationRunItem)?.Id;
        var runs = Json.Array(detail, "runs").Where(r => r != null).Select(r => new AutomationRunItem(Json.Text(r, "id"), Json.Text(r, "state").Replace('_', ' '), TimeLabel(r, "started_at") + " · " + Json.Text(r, "trigger"), r!)).ToList();
        refreshingExtensions = true;
        AutomationRuns.ItemsSource = runs;
        AutomationRuns.SelectedItem = runs.FirstOrDefault(r => r.Id == selected) ?? runs.FirstOrDefault();
        refreshingExtensions = false;
        UpdateRunOutput();
    }
    private async void AutomationSelected(object sender, SelectionChangedEventArgs e)
    {
        if (refreshingExtensions || AutomationList.SelectedItem is not AutomationItem item) return;
        await Act(() => LoadAutomationRuns(item.Id));
    }
    private void AutomationRunSelected(object sender, SelectionChangedEventArgs e) { if (!refreshingExtensions) UpdateRunOutput(); }
    private void UpdateRunOutput()
    {
        var state = (AutomationRuns.SelectedItem as AutomationRunItem)?.State;
        AutomationOutput.Text = state == null ? "Select an automation to inspect its runs." : "Status: " + Json.Text(state, "state") + "\nExit code: " + (state["exit_code"]?.ToJsonString() ?? "—") + "\n\n" + Json.Text(state, "output") + "\n" + Json.Text(state, "error");
    }
    private async Task AutomationAction(string id, string action)
    {
        var item = AutomationList.Items.OfType<AutomationItem>().FirstOrDefault(a => a.Id == id) ?? throw new IOException("Reload the automation list first.");
        var body = new JsonObject { ["expected_revision"] = item.Revision };
        if (action == "enable") body["authorize_execution"] = true;
        var accepted = await ExtensionPost("/v1/bee/automations/" + id + "/" + action, body);
        await RefreshExtensions();
        if (action == "run")
        {
            refreshingExtensions = true;
            AutomationList.SelectedItem = AutomationList.Items.OfType<AutomationItem>().FirstOrDefault(a => a.Id == id);
            refreshingExtensions = false;
            await LoadAutomationRuns(id);
            AutomationRuns.SelectedItem = AutomationRuns.Items.OfType<AutomationRunItem>().FirstOrDefault(r => r.Id == Json.Text(accepted, "run_id"));
        }
    }
    private async void RunAutomation(object sender, RoutedEventArgs e) { e.Handled = true; await Act(() => AutomationAction((string)((Button)sender).Tag, "run")); }
    private async void ToggleAutomation(object sender, RoutedEventArgs e)
    {
        e.Handled = true;
        string id = (string)((Button)sender).Tag;
        var item = AutomationList.Items.OfType<AutomationItem>().First(a => a.Id == id);
        await Act(() => AutomationAction(id, item.Enabled ? "pause" : "enable"));
    }
    private async void DeleteAutomation(object sender, RoutedEventArgs e) { e.Handled = true; await Act(() => AutomationAction((string)((Button)sender).Tag, "delete")); }
    private async void CancelAutomationRun(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (AutomationList.SelectedItem is not AutomationItem item || AutomationRuns.SelectedItem is not AutomationRunItem run) throw new IOException("Select the running task first.");
        await ExtensionPost("/v1/bee/automations/" + item.Id + "/cancel", new JsonObject { ["run_id"] = run.Id });
        await RefreshExtensions();
    });
    private async void OpenAutomationWork(object sender, RoutedEventArgs e) => await Act(OpenSelectedAutomationWork);
    private async Task OpenSelectedAutomationWork()
    {
        if (AutomationRuns.SelectedItem is not AutomationRunItem run || Json.Text(run.State, "native_session") is not string native || native.Length == 0)
            throw new IOException("Select an agent automation run to open its Work transcript.");
        SelectSession(native);
        await Poll();
    }

    private void NewAutomation(object sender, RoutedEventArgs e)
    {
        editingAutomation = null; editingRevision = 0; editorRequestId = Json.Id();
        AutomationEditorTitle.Text = "New automation";
        AutomationName.Text = ""; AutomationScheduleType.SelectedIndex = 0; AutomationScheduleValue.Text = "60";
        AutomationTimezone.Text = "Europe/Oslo"; AutomationExecutionType.SelectedIndex = 0; AutomationTimeout.Text = "60";
        AutomationWorkspace.SelectedValue = ""; AutomationConversation.Text = IsBee(sessionId) ? sessionId : "";
        AutomationSource.Text = "Write-Output 'Hello from Cloud Bee'";
        AutomationEnabled.IsChecked = true; AutomationAuthorization.IsChecked = false; AutomationEditorError.Text = "";
        AutomationEditor.Visibility = Visibility.Visible;
    }
    private async void EditAutomation(object sender, RoutedEventArgs e) => await Act(EditSelectedAutomation);
    private async Task EditSelectedAutomation()
    {
        if (client == null || AutomationList.SelectedItem is not AutomationItem item) throw new IOException("Select an automation first.");
        var saved = (await client.Get("/v1/bee/automations/" + item.Id, lifetime.Token))["automation"];
        var definition = saved?["definition"];
        NewAutomation(this, new RoutedEventArgs());
        editingAutomation = item.Id; editingRevision = Json.Number(saved, "revision");
        AutomationEditorTitle.Text = "Edit automation"; AutomationName.Text = Json.Text(definition, "name");
        string schedule = Json.Text(definition?["schedule"], "type");
        AutomationScheduleType.SelectedIndex = schedule == "cron" ? 1 : schedule == "once" ? 2 : 0;
        AutomationScheduleValue.Text = schedule == "cron" ? Json.Text(definition?["schedule"], "expression") : schedule == "once" ? DateTimeOffset.FromUnixTimeMilliseconds(Json.Number(definition?["schedule"], "at_ms")).ToLocalTime().ToString("O") : (Json.Number(definition?["schedule"], "every_ms") / 60000d).ToString(CultureInfo.InvariantCulture);
        AutomationTimezone.Text = Json.Text(definition?["schedule"], "timezone", "Europe/Oslo");
        string interpreter = Json.Text(definition?["execution"], "interpreter");
        bool agent = Json.Text(definition?["execution"], "type") == "agent";
        AutomationExecutionType.SelectedIndex = agent ? 3 : interpreter == "node" ? 1 : interpreter == "python" ? 2 : 0;
        AutomationSource.Text = Json.Text(definition?["execution"], agent ? "prompt" : "source");
        AutomationConversation.Text = Json.Text(definition?["execution"], "conversation_id");
        AutomationWorkspace.SelectedValue = Json.Text(definition, "workspace");
        AutomationTimeout.Text = Math.Max(1, Json.Number(definition?["execution"], "timeout_ms") / 1000).ToString(CultureInfo.InvariantCulture);
        AutomationEnabled.IsChecked = Json.Flag(definition, "enabled");
    }
    private void CancelAutomationEditor(object sender, RoutedEventArgs e) => AutomationEditor.Visibility = Visibility.Collapsed;
    private async Task SaveAutomation()
    {
        if (client == null) return;
        if (AutomationAuthorization.IsChecked != true) throw new IOException("Review the task and select execution authorization before saving.");
        string type = ((ComboBoxItem)AutomationScheduleType.SelectedItem).Tag as string ?? "interval";
        var schedule = new JsonObject { ["type"] = type };
        if (type == "cron") { schedule["expression"] = AutomationScheduleValue.Text.Trim(); schedule["timezone"] = AutomationTimezone.Text.Trim(); }
        else if (type == "once")
        {
            if (!DateTimeOffset.TryParse(AutomationScheduleValue.Text, CultureInfo.InvariantCulture, DateTimeStyles.None, out var once)) throw new IOException("Use an ISO date and offset, such as 2026-10-17T09:00:00+02:00.");
            schedule["at_ms"] = once.ToUnixTimeMilliseconds();
        }
        else
        {
            if (!double.TryParse(AutomationScheduleValue.Text, NumberStyles.Number, CultureInfo.InvariantCulture, out double minutes) || !double.IsFinite(minutes) || minutes <= 0 || minutes > 525600) throw new IOException("Enter an interval between 1 and 525600 minutes.");
            schedule["every_ms"] = (long)(minutes * 60000);
        }
        string interpreter = ((ComboBoxItem)AutomationExecutionType.SelectedItem).Tag as string ?? "powershell";
        JsonObject execution;
        if (interpreter == "agent") execution = new JsonObject { ["type"] = "agent", ["conversation_id"] = AutomationConversation.Text.Trim(), ["prompt"] = AutomationSource.Text };
        else
        {
            if (!long.TryParse(AutomationTimeout.Text, out long timeout) || timeout < 1 || timeout > 600) throw new IOException("Use a script timeout of 1–600 seconds.");
            execution = new JsonObject { ["type"] = "script", ["interpreter"] = interpreter, ["source"] = AutomationSource.Text, ["timeout_ms"] = timeout * 1000 };
        }
        string workspace = AutomationWorkspace.SelectedValue as string ?? "";
        var definition = new JsonObject { ["name"] = AutomationName.Text.Trim(), ["enabled"] = AutomationEnabled.IsChecked == true,
            ["schedule"] = schedule, ["execution"] = execution, ["workspace"] = workspace.Length > 0 ? workspace : null, ["workspace_name"] = null };
        var body = new JsonObject { ["request_id"] = editorRequestId, ["automation"] = definition, ["authorize_execution"] = true };
        if (editingAutomation != null) body["expected_revision"] = editingRevision;
        var result = await ExtensionPost(editingAutomation == null ? "/v1/bee/automations" : "/v1/bee/automations/" + editingAutomation + "/update", body);
        AutomationEditor.Visibility = Visibility.Collapsed;
        await RefreshExtensions();
        string id = editingAutomation ?? Json.Text(result, "id");
        AutomationList.SelectedItem = AutomationList.Items.OfType<AutomationItem>().FirstOrDefault(a => a.Id == id);
        await LoadAutomationRuns(id);
    }
    private async void SaveAutomationClicked(object sender, RoutedEventArgs e) => await Act(SaveAutomation);

    private void ApplyPluginFilter()
    {
        string? id = (PluginList.SelectedItem as PluginItem)?.Id ?? selectedPlugin;
        var items = pluginItems.Where(p => PluginFilter.SelectedIndex == 1 || p.Installed).ToList();
        refreshingExtensions = true;
        PluginList.ItemsSource = items;
        PluginList.SelectedItem = items.FirstOrDefault(p => p.Id == id) ?? items.FirstOrDefault();
        refreshingExtensions = false;
        ShowPluginSettings();
    }
    private void PluginFilterChanged(object sender, SelectionChangedEventArgs e) { if (PluginList != null) ApplyPluginFilter(); }
    private void PluginSelected(object sender, SelectionChangedEventArgs e) { if (!refreshingExtensions) ShowPluginSettings(); }
    private void ShowPluginSettings()
    {
        var item = PluginList.SelectedItem as PluginItem;
        PluginSettingsPanel.Visibility = item?.Id == "custom-instructions" && item.Installed ? Visibility.Visible : Visibility.Collapsed;
        if (item?.Id != selectedPlugin)
            PluginInstructions.Text = Json.Text(Json.Array(pluginState, "plugins").FirstOrDefault(p => Json.Text(p, "id") == item?.Id)?["settings"], "instructions");
        selectedPlugin = item?.Id;
    }
    private async Task PluginAction(string id, string action, JsonObject? body = null)
    {
        await ExtensionPost("/v1/bee/plugins/" + id + "/" + action, body ?? new JsonObject());
        await RefreshExtensions();
    }
    private async void TogglePlugin(object sender, RoutedEventArgs e)
    {
        e.Handled = true;
        string id = (string)((Button)sender).Tag;
        var item = pluginItems.First(p => p.Id == id);
        await Act(() => PluginAction(id, !item.Installed ? "install" : item.Enabled ? "disable" : "enable"));
    }
    private async void UninstallPlugin(object sender, RoutedEventArgs e) { e.Handled = true; await Act(() => PluginAction((string)((Button)sender).Tag, "uninstall")); }
    private async void SavePluginSettings(object sender, RoutedEventArgs e) => await Act(() => PluginAction("custom-instructions", "settings", new JsonObject { ["settings"] = new JsonObject { ["instructions"] = PluginInstructions.Text } }));

    private async Task ExtensionsSmokeTest(string output)
    {
        if (client == null || !efforts.ContainsKey("fixture")) throw new IOException("Extensions smoke requires a fixture Core.");
        await OpenExtensions(false);
        NewAutomation(this, new RoutedEventArgs());
        AutomationName.Text = "Windows automation check";
        AutomationExecutionType.SelectedIndex = 2;
        AutomationSource.Text = "print('Native automation output — ø')";
        AutomationEnabled.IsChecked = false;
        bool rejected = false;
        try { await SaveAutomation(); }
        catch (IOException error) when (error.Message.Contains("authorization")) { rejected = true; }
        if (!rejected) throw new IOException("Automation saved without execution authorization.");
        AutomationAuthorization.IsChecked = true;
        await SaveAutomation();
        var item = AutomationList.SelectedItem as AutomationItem ?? throw new IOException("Saved automation is missing from the GUI.");
        await AutomationAction(item.Id, "run");
        async Task WaitForRun(int count)
        {
            var deadline = DateTime.UtcNow.AddSeconds(20);
            do
            {
                nextExtensions = DateTime.MinValue;
                await RefreshExtensions();
                var runs = AutomationRuns.Items.OfType<AutomationRunItem>().ToList();
                if (runs.Count >= count && runs.All(run => Json.Text(run.State, "state") != "running")) return;
                await Task.Delay(100);
            } while (DateTime.UtcNow < deadline);
            throw new IOException("Automation run did not finish in the actual window.");
        }
        await WaitForRun(1);
        if (!AutomationOutput.Text.Contains("Native automation output — ø")) throw new IOException("Automation output did not render.");
        await EditSelectedAutomation();
        AutomationName.Text = "Windows automation check · edited";
        AutomationSource.Text = "print('Edited automation output')";
        AutomationAuthorization.IsChecked = true;
        await SaveAutomation();
        await AutomationAction(item.Id, "run");
        await WaitForRun(2);
        if (!AutomationOutput.Text.Contains("Edited automation output")) throw new IOException("Edited script did not run.");
        NewAutomation(this, new RoutedEventArgs());
        AutomationName.Text = "Authorized Work check";
        AutomationExecutionType.SelectedIndex = 3;
        AutomationSource.Text = "Verify the automation fixture";
        AutomationEnabled.IsChecked = false;
        AutomationAuthorization.IsChecked = true;
        await SaveAutomation();
        var agent = AutomationList.SelectedItem as AutomationItem ?? throw new IOException("Agent automation is missing.");
        await AutomationAction(agent.Id, "run");
        await WaitForRun(1);
        await OpenSelectedAutomationWork();
        if (ExtensionsPane.Visibility != Visibility.Collapsed || IsBee(sessionId)) throw new IOException("Agent run did not open its native Work transcript.");
        await OpenExtensions(false);
        AutomationList.SelectedItem = AutomationList.Items.OfType<AutomationItem>().First(a => a.Id == item.Id);
        await LoadAutomationRuns(item.Id);
        await Capture(Path.Combine(output, "cloud-bee-automations.png"));
        await OpenExtensions(true);
        PluginFilter.SelectedIndex = 1;
        await PluginAction("keep-awake", "install");
        await PluginAction("keep-awake", "disable");
        await PluginAction("custom-instructions", "settings", new JsonObject { ["settings"] = new JsonObject { ["instructions"] = "Keep changes focused and verify results." } });
        PluginList.SelectedItem = pluginItems.First(p => p.Id == "custom-instructions");
        selectedPlugin = null;
        ShowPluginSettings();
        if (PluginList.Items.Count != 6 || !PluginInstructions.Text.Contains("Keep changes focused")) throw new IOException("Native plugins or settings did not render.");
        await Capture(Path.Combine(output, "cloud-bee-plugins.png"));
        await PluginAction("custom-instructions", "disable");
        await PluginAction("keep-awake", "uninstall");
        await OpenExtensions(false);
        await AutomationAction(item.Id, "delete");
        await AutomationAction(agent.Id, "delete");
        if (AutomationList.Items.OfType<AutomationItem>().Any(a => a.Id == item.Id)) throw new IOException("Deleted automation stayed visible.");
        BackToConversation(this, new RoutedEventArgs());
        await File.WriteAllTextAsync(Path.Combine(output, "extensions-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "native navigation, automation authorization/create/edit/run/output/history/delete, plugin install/disable/settings/uninstall" }.ToJsonString());
    }
}
