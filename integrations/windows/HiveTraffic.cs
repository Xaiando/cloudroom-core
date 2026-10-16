using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;

namespace Cloudroom.Desktop;

public sealed record HiveTaskView(string Id, string Model, string Path, string State, string Traffic, Brush Color, string Activity, bool Busy, Brush TaskColor);
public partial class MainWindow
{
    private JsonObject? pendingHiveStart, pendingHiveAction;
    private string? pendingHiveRoute;
    private string hiveRendered = "";
    private async Task RefreshHive(CoreClient source, string? selected)
    {
        var state = await source.Get("/v1/bee/traffic", lifetime.Token);
        if (!ReferenceEquals(source, client) || councilId != selected) return;
        var rows = Json.Array(state, "tasks").Where(t => selected == null || Json.Text(t, "council_id") == selected).ToList();
        string rendered = string.Join("|", rows.Select(t => t?.ToJsonString()));
        if (rendered != hiveRendered)
        {
            HiveTasks.ItemsSource = rows.Select(t =>
            {
                string light = Json.Text(t, "traffic"), phase = Json.Text(t, "state");
                string label = light switch { "red" => "RED · Modifying", "yellow" => "YELLOW · Reserved / waiting", "blue" => "BLUE · Verification / recovery", _ => "GREEN · Available" };
                Brush color = light switch { "red" => Brushes.Salmon, "yellow" => Brushes.Gold, "blue" => Brushes.DeepSkyBlue, _ => Brushes.LightGreen };
                string owner = Json.Text(t?["lease"]?["owner"], "agent_id");
                string ownerTask = Json.Text(t?["lease"]?["owner"], "task_id");
                string queue = Json.Number(t, "queue_position") > 0 ? " · queue " + Json.Number(t, "queue_position") : "";
                return new HiveTaskView(Json.Text(t, "id"), "Codex · " + Json.Text(t, "model") + " · " + HiveWorker(Json.Text(t, "id")), Json.Text(t, "path"), "Task: " + phase.Replace('_', ' ') + queue, label, color, Json.Text(t, "activity") + (owner.Length > 0 ? "\nResource owner: " + owner + " · " + HiveWorker(ownerTask) : ""), phase is "preparing" or "executing" or "stopping" or "integrating", phase == "waiting" ? Brushes.Gold : Brushes.WhiteSmoke);
            }).ToList();
            hiveRendered = rendered;
        }
        HiveStatus.Text = rows.Count == 0 ? "No isolated Work tasks. Choose isolated Work access to run a research microtask." : rows.Count + " tasks · " + rows.Count(t => Json.Text(t, "state") == "verification") + " awaiting review · " + rows.Count(t => Json.Text(t, "state") == "waiting") + " waiting";
        HiveRetry.Visibility = pendingHiveStart != null || pendingHiveAction != null ? Visibility.Visible : Visibility.Collapsed;
    }
    private async void StartHiveClicked(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (client == null || councilId == null || sender is not FrameworkElement { DataContext: CouncilTaskView task }) return;
        if (pendingHiveStart != null || pendingHiveAction != null) throw new IOException("Retry the unconfirmed HTC request first.");
        var catalog = await client.Get("/v1/bee/traffic/catalog", lifetime.Token);
        var models = Json.Array(catalog, "models").Select(m => Json.Text(m, "model")).ToList();
        if (models.Count == 0) throw new IOException("Install and sign in to native Codex before starting isolated Work.");
        var dialog = HiveWindow("Authorize isolated Work", 680, 520);
        var stack = new StackPanel { Margin = new Thickness(24) };
        stack.Children.Add(HiveText(task.Title, 22));
        stack.Children.Add(HiveText("Research owner: " + task.Owner + "\nCoding worker: native Codex. API Chat participants cannot modify files.", 13));
        stack.Children.Add(HiveText("Native Work model", 13));
        var model = new ComboBox { ItemsSource = models, SelectedIndex = 0, Margin = new Thickness(0, 8, 0, 12) }; stack.Children.Add(model);
        stack.Children.Add(HiveText("One tracked UTF-8 file to review and integrate", 13));
        var path = new TextBox { Margin = new Thickness(0, 8, 0, 12), ToolTip = "Project-relative path, for example src/provider.rs" }; stack.Children.Add(path);
        stack.Children.Add(HiveText("Start from a clean Git project. Each task gets a detached worktree. HTC queues shared integration and checks the baseline file revision. Compilation/tests are not automatic. Other worktree changes remain private.", 13));
        var authorize = new CheckBox { Margin = new Thickness(0, 12, 0, 12), Content = HiveText("I authorize native Work in this isolated worktree. Workers run under my Windows account; this is not account isolation.", 13) }; stack.Children.Add(authorize);
        var error = HiveText("", 12); error.Foreground = Brushes.Salmon; stack.Children.Add(error);
        bool accepted = false;
        var start = HiveButton("Start isolated Work", () =>
        {
            if (authorize.IsChecked != true || path.Text.Trim().Length == 0) { error.Text = "Choose a target file and authorize Work."; return; }
            accepted = true; dialog.Close();
        }); stack.Children.Add(start);
        dialog.Content = new ScrollViewer { Content = stack, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }; dialog.ShowDialog();
        if (!accepted) return;
        pendingHiveStart = new JsonObject { ["request_id"] = Json.Id(), ["council_id"] = councilId, ["round_id"] = task.RoundId, ["microtask"] = task.Index, ["harness"] = "codex", ["model"] = (string)model.SelectedItem, ["path"] = path.Text.Trim().Replace('\\', '/'), ["authorize_work"] = true };
        SaveCouncilDraft(); await RetryHive();
    });
    private async void RetryHiveClicked(object sender, RoutedEventArgs e) => await Act(RetryHive);
    private async Task RetryHive()
    {
        if (client == null) return;
        try
        {
            if (pendingHiveStart != null) { await client.Post("/v1/bee/traffic/tasks", (JsonObject)pendingHiveStart.DeepClone(), lifetime.Token); pendingHiveStart = null; }
            if (pendingHiveAction != null && pendingHiveRoute != null) { await client.Post(pendingHiveRoute, (JsonObject)pendingHiveAction.DeepClone(), lifetime.Token); pendingHiveAction = null; pendingHiveRoute = null; }
        }
        catch (CoreApiException error) when (error.StatusCode is >= 400 and < 500)
        { pendingHiveStart = pendingHiveAction = null; pendingHiveRoute = null; SaveCouncilDraft(); throw; }
        finally { SaveCouncilDraft(); HiveRetry.Visibility = pendingHiveStart != null || pendingHiveAction != null ? Visibility.Visible : Visibility.Collapsed; }
        await RefreshCouncilWorkspace();
    }
    private async void InspectHiveClicked(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (sender is FrameworkElement { DataContext: HiveTaskView task }) await InspectHive(task.Id);
    });
    private Window HiveWindow(string title, double width, double height) => new() { Owner = this, Title = title, Width = width, Height = height, MinWidth = Math.Min(width, 600), MinHeight = 400, WindowStartupLocation = WindowStartupLocation.CenterOwner, Background = new SolidColorBrush(Color.FromRgb(23, 30, 41)), Foreground = Brushes.WhiteSmoke, Resources = Resources };
    private static TextBlock HiveText(string text, double size) => new() { Text = text, TextWrapping = TextWrapping.Wrap, FontSize = size, Margin = new Thickness(0, 0, 0, 8) };
    private Button HiveButton(string label, Action click)
    { var b = new Button { Content = label, Style = (Style)FindResource("Quiet"), Margin = new Thickness(0, 0, 8, 8) }; b.Click += (_, _) => click(); return b; }
    private async Task InspectHive(string id, string? smokeOutput = null)
    {
        if (client == null) return;
        var dialog = HiveWindow("Hive task · inspect and control", 1020, 760);
        var grid = new Grid { Margin = new Thickness(22) };
        grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto }); grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) }); grid.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
        var heading = HiveText("Loading…", 13); grid.Children.Add(heading);
        var tabs = new TabControl(); Grid.SetRow(tabs, 1); grid.Children.Add(tabs);
        var bottom = new StackPanel { Margin = new Thickness(0, 12, 0, 0) }; Grid.SetRow(bottom, 2); grid.Children.Add(bottom);
        var authorization = new CheckBox { Content = "I reviewed the exact proposal and required checks, or the current recovery state.", Margin = new Thickness(0, 0, 0, 10) }; bottom.Children.Add(authorization);
        var controls = new WrapPanel(); bottom.Children.Add(controls);
        var error = HiveText("", 12); error.Foreground = Brushes.Salmon; bottom.Children.Add(error);
        JsonNode? detail = null, currentDisk = null;
        bool busy = false;
        async Task Load()
        {
            detail = await client.Get("/v1/bee/traffic/tasks/" + id, lifetime.Token); var t = detail["task"];
            string phase = Json.Text(t, "state");
            heading.Text = "Codex · " + Json.Text(t, "model") + " · " + Json.Text(t, "path") + "\nTask: " + phase + " · Resource: " + Json.Text(t, "traffic").ToUpperInvariant() + " · " + Json.Text(t?["lease"], "state") + " · generation " + Json.Number(t?["lease"], "generation") + "\n" + Json.Text(t, "activity") + "\n" + OrdinaryPath(Json.Text(t, "worktree")) + "\nNative session: " + Json.Text(t, "session_id") + " · attempt " + Json.Number(t, "attempt") + "/3 · queue " + Json.Number(t, "queue_position") + "\nLease acquired: " + HiveTime(Json.Number(t?["lease"], "acquired_at")) + " · expires: " + HiveTime(Json.Number(t?["lease"], "expires_at")) + "\n" + Json.Text(t, "error");
            tabs.Items.Clear();
            void Add(string name, string text) => tabs.Items.Add(new TabItem { Header = name, Content = new TextBox { Text = text, IsReadOnly = true, AcceptsReturn = true, FontFamily = new FontFamily("Consolas"), TextWrapping = TextWrapping.NoWrap, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto } });
            Add("Baseline", Json.Text(detail["base"], "text")); Add("Proposed file", Json.Text(detail["proposed"], "text"));
            Add("Checks & objective", string.Join("\n", Json.Array(detail, "checks").Select(v => v?.GetValue<string>())) + "\n\n" + Json.Text(detail, "objective"));
            currentDisk = null;
            if (phase == "recovery")
            {
                currentDisk = await client.Get("/v1/bee/files/" + Json.Text(t, "council_id") + "/preview?path=" + Uri.EscapeDataString(Json.Text(t, "path")), lifetime.Token);
                Add("Current shared file", Json.Text(currentDisk, "text"));
            }
            controls.Children.Clear();
            foreach (string operation in new[] { "pause", "cancel", "retry", "approve", "reject", "recover" })
            {
                string label = operation switch { "approve" => "Approve integration", "recover" => "Keep current disk / release recovery", "retry" => "Retry preserved Work", _ => char.ToUpperInvariant(operation[0]) + operation[1..] };
                var button = HiveButton(label, async () =>
                {
                    if (busy) return; busy = true;
                    try
                    {
                        if (pendingHiveAction != null || pendingHiveStart != null) throw new IOException("Retry the unconfirmed HTC request first.");
                        if (operation is "approve" or "retry" or "recover" && authorization.IsChecked != true) throw new IOException("Inspect the proposal or recovery state, then check the review box.");
                        pendingHiveRoute = "/v1/bee/traffic/tasks/" + id + "/" + operation;
                        pendingHiveAction = new JsonObject { ["request_id"] = Json.Id(), ["expected_revision"] = Json.Number(detail?["task"], "revision"), ["proposed_revision"] = Json.Text(detail?["proposed"], "revision"), ["current_revision"] = Json.Text(currentDisk, "revision"), ["authorize"] = authorization.IsChecked == true };
                        SaveCouncilDraft(); await RetryHive(); authorization.IsChecked = false; await Load(); error.Text = "Request accepted. Refresh to observe execution settling.";
                    }
                    catch (Exception failure) { error.Text = SafeError(failure); }
                    finally { busy = false; }
                });
                button.IsEnabled = operation switch { "approve" => phase == "verification", "recover" => phase == "recovery", "retry" => phase is "failed" or "paused" or "cancelled", _ => phase is "preparing" or "executing" or "stopping" or "waiting" or "verification" or "failed" or "paused" or "cancelled" };
                controls.Children.Add(button);
            }
            controls.Children.Add(HiveButton("Refresh", async () => { try { await Load(); } catch (Exception failure) { error.Text = SafeError(failure); } }));
            controls.Children.Add(HiveButton("Close", () => dialog.Close()));
        }
        await Load(); dialog.Content = grid;
        if (smokeOutput == null) { dialog.ShowDialog(); return; }
        dialog.Show(); dialog.UpdateLayout(); await Task.Delay(80);
        if (tabs.Items.Count < 3 || !controls.Children.OfType<Button>().Any(b => Equals(b.Content, "Approve integration") && b.IsEnabled)) throw new IOException("HTC inspection did not expose the persisted proposal and review controls.");
        var bitmap = new System.Windows.Media.Imaging.RenderTargetBitmap((int)dialog.ActualWidth, (int)dialog.ActualHeight, 96, 96, PixelFormats.Pbgra32); bitmap.Render(dialog);
        var encoder = new System.Windows.Media.Imaging.PngBitmapEncoder(); encoder.Frames.Add(System.Windows.Media.Imaging.BitmapFrame.Create(bitmap));
        using (var file = File.Create(System.IO.Path.Combine(smokeOutput, "cloud-bee-hive-review.png"))) encoder.Save(file);
        dialog.Close();
    }
    private static string HiveWorker(string id) => "worker " + (id.Length > 12 ? id[^8..] : id);
    private static string HiveTime(long ms) => ms > 0 ? DateTimeOffset.FromUnixTimeMilliseconds(ms).ToLocalTime().ToString("T") : "—";

    private async Task HiveSmokeTest(string output)
    {
        if (client == null || smokeDirectory == null) throw new IOException("HTC smoke tests require a disposable protocol fixture.");
        string root = System.IO.Path.GetDirectoryName(System.IO.Path.GetFullPath(ConfigPathBox.Text))!;
        string workspace = System.IO.Path.Combine(root, "hive-project"), gate = System.IO.Path.Combine(root, "gui-hive-release");
        await OpenCouncilWorkspace();
        councilId = null; councilRendered = ""; CouncilFolder.Text = workspace; SelectTag(CouncilAccess, "edit"); CouncilCoordinate.IsChecked = true;
        CouncilGptOn.IsChecked = CouncilQwenOn.IsChecked = true; CouncilGrokOn.IsChecked = CouncilClaudeOn.IsChecked = false;
        CouncilComposer.Text = "HTC_FIXTURE_APPLY HTC_FIXTURE_GATE=" + gate; await SendCouncilMessage();
        var deadline = DateTime.UtcNow.AddSeconds(30);
        do { await RefreshCouncilWorkspace(); if (!councilBusy && CouncilTasks.Items.Count > 0) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        if (councilBusy || CouncilTasks.Items.Count == 0 || CouncilTasks.Items.OfType<CouncilTaskView>().Any(t => !t.WorkAllowed)) throw new IOException("HTC research plan did not enable separate native Work.");
        JsonObject Start(string key) => new() { ["request_id"] = key, ["council_id"] = councilId, ["round_id"] = "plan_gui_hive", ["microtask"] = 0, ["harness"] = "codex", ["model"] = "fixture", ["path"] = "note.txt", ["authorize_work"] = true };
        // Use the actual accepted Council message identity rather than a guessed fixture ID.
        string round = Json.Text((await client.Get("/v1/bee/councils/" + councilId))["latest"], "request_id");
        pendingHiveStart = Start("gui_hive_one"); pendingHiveStart["round_id"] = round; SaveCouncilDraft();
        try { await RetryHive(); throw new IOException("HTC fixture did not drop start acceptance."); }
        catch (Exception failure) when (failure is System.Net.Http.HttpRequestException or IOException && failure is not CoreApiException) { if (pendingHiveStart == null) throw; }
        string original = pendingHiveStart!.ToJsonString(); councilIdentity = null; RestoreCouncilDraft();
        if (pendingHiveStart?.ToJsonString() != original) throw new IOException("HTC lost-acceptance retry changed after reconnect.");
        await RetryHive();
        pendingHiveStart = Start("gui_hive_two"); pendingHiveStart["round_id"] = round; SaveCouncilDraft(); await RetryHive();
        deadline = DateTime.UtcNow.AddSeconds(30);
        do { await RefreshCouncilWorkspace(); if (HiveTasks.Items.OfType<HiveTaskView>().Count(t => t.Traffic.StartsWith("RED", StringComparison.Ordinal)) == 2) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        if (HiveTasks.Items.OfType<HiveTaskView>().Count(t => t.Busy && t.Traffic.StartsWith("RED", StringComparison.Ordinal)) != 2) throw new IOException("HTC did not render two parallel private modifications.");
        await File.WriteAllTextAsync(gate, "release");
        deadline = DateTime.UtcNow.AddSeconds(30);
        do { await RefreshCouncilWorkspace(); if (HiveTasks.Items.OfType<HiveTaskView>().Any(t => t.State.Contains("verification")) && HiveTasks.Items.OfType<HiveTaskView>().Any(t => t.State.Contains("waiting"))) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        var reviewed = HiveTasks.Items.OfType<HiveTaskView>().FirstOrDefault(t => t.State.Contains("verification")) ?? throw new IOException("HTC proposal review did not render.");
        var queued = HiveTasks.Items.OfType<HiveTaskView>().FirstOrDefault(t => t.State.Contains("waiting")) ?? throw new IOException("HTC shared integration queue did not render.");
        if (!reviewed.Traffic.StartsWith("BLUE", StringComparison.Ordinal) || !queued.State.Contains("queue 1")) throw new IOException("HTC GUI did not match persisted backend ownership/queue state.");
        string actual = CouncilFolder.Text; CouncilFolder.Text = "Disposable Git project · isolated workers";
        var screenMessages = CouncilConversation.Items.OfType<ChatMessage>().Select(m => (Message: m, Original: m.Text)).ToList();
        foreach (var message in screenMessages) message.Message.Text = System.Text.RegularExpressions.Regex.Replace(message.Original, @"HTC_FIXTURE_APPLY HTC_FIXTURE_GATE=[^\r\n]+", "Implement the reviewed change in parallel worktrees");
        CouncilConversation.Items.Refresh();
        await Capture(System.IO.Path.Combine(output, "cloud-bee-hive-traffic.png")); CouncilFolder.Text = actual;
        foreach (var message in screenMessages) message.Message.Text = message.Original;
        CouncilConversation.Items.Refresh();
        await InspectHive(reviewed.Id, output);
        var proposal = await client.Get("/v1/bee/traffic/tasks/" + reviewed.Id);
        pendingHiveRoute = "/v1/bee/traffic/tasks/" + reviewed.Id + "/approve";
        pendingHiveAction = new JsonObject { ["request_id"] = Json.Id(), ["expected_revision"] = Json.Number(proposal["task"], "revision"), ["proposed_revision"] = Json.Text(proposal["proposed"], "revision"), ["authorize"] = true }; SaveCouncilDraft();
        try { await RetryHive(); throw new IOException("HTC fixture did not drop integration acceptance."); }
        catch (Exception failure) when (failure is System.Net.Http.HttpRequestException or IOException && failure is not CoreApiException) { if (pendingHiveAction == null) throw; }
        await RetryHive();
        deadline = DateTime.UtcNow.AddSeconds(20);
        do { await RefreshCouncilWorkspace(); if (HiveTasks.Items.OfType<HiveTaskView>().Any(t => t.Id == queued.Id && t.State.Contains("verification"))) break; await Task.Delay(50); } while (DateTime.UtcNow < deadline);
        var stale = await client.Get("/v1/bee/traffic/tasks/" + queued.Id);
        pendingHiveRoute = "/v1/bee/traffic/tasks/" + queued.Id + "/reject";
        pendingHiveAction = new JsonObject { ["request_id"] = Json.Id(), ["expected_revision"] = Json.Number(stale["task"], "revision") }; SaveCouncilDraft(); await RetryHive();
        if (HiveTasks.Items.OfType<HiveTaskView>().Any(t => !t.Traffic.StartsWith("GREEN", StringComparison.Ordinal))) throw new IOException("HTC accepted/rejected tasks did not release resource ownership.");
        if (await File.ReadAllTextAsync(System.IO.Path.Combine(workspace, "note.txt")) != "Native isolated proposal\n") throw new IOException("HTC GUI approved a different file proposal.");
        DismissCouncilWorkspace(this, new RoutedEventArgs());
        await File.WriteAllTextAsync(System.IO.Path.Combine(output, "hive-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "HTC persisted red/blue/green ownership, two native workers, conflicting integration queue, proposal/controls window, immutable start/integration retries and reviewed file save" }.ToJsonString());
    }
}
