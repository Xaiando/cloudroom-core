using System.Diagnostics;
using System.IO;
using System.Net.Http;
using System.Text;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;

namespace Cloudroom.Desktop;

public partial class MainWindow : Window
{
    private readonly Transcript transcript = new();
    private readonly CancellationTokenSource lifetime = new();
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromSeconds(1) };
    private readonly Dictionary<string, string> titles = [], drafts = [];
    private readonly Dictionary<string, string[]> efforts = [];
    private readonly string? smokeDirectory;
    private CoreClient? client;
    private JsonNode? capabilities, current;
    private string? sessionId, folder, draftId, draftSession, draftText;
    private JsonObject? pendingStart, pendingPrompt;
    private bool acting, polling, refreshingList, connected;
    private long selectionVersion;
    private DateTime nextList = DateTime.MinValue;

    public MainWindow(string configPath, string? smokeDirectory)
    {
        InitializeComponent();
        ConfigPathBox.Text = configPath;
        this.smokeDirectory = smokeDirectory;
        ConversationList.ItemsSource = transcript.Messages;
        transcript.Messages.CollectionChanged += (_, _) => UpdateWelcome();
        timer.Tick += async (_, _) => await Poll();
        UpdateWelcome();
    }

    private async void WindowLoaded(object sender, RoutedEventArgs e)
    {
        try
        {
            await LoadLocalSettings();
            await Connect();
            if (smokeDirectory != null) { await SmokeTest(smokeDirectory); Application.Current.Shutdown(0); return; }
        }
        catch (Exception error)
        {
            if (smokeDirectory != null)
            {
                Directory.CreateDirectory(smokeDirectory);
                await File.WriteAllTextAsync(Path.Combine(smokeDirectory, "failure.txt"), SafeError(error));
                Application.Current.Shutdown(1);
                return;
            }
            ConnectionError.Text = SafeError(error);
            ConnectionDialog.Visibility = Visibility.Visible;
        }
        timer.Start();
        Composer.Focus();
    }

    private async Task LoadLocalSettings()
    {
        var settings = await CoreClient.ReadSettings(ConfigPathBox.Text);
        EndpointBox.Text = settings.Endpoint;
        TokenBox.Password = settings.Token;
    }

    private async Task Connect()
    {
        var candidate = new CoreClient(EndpointBox.Text.Trim(), TokenBox.Password.Trim());
        try
        {
            if (!Json.Flag(await candidate.Get("/v1/ready", lifetime.Token), "ready")) throw new IOException("Core is not ready for work.");
            var discovered = await candidate.Get("/v1/capabilities", lifetime.Token);
            var codex = Json.Array(discovered, "harnesses").FirstOrDefault(h => Json.Text(h, "id") == "codex") ?? throw new IOException("This Core does not have Codex configured.");
            var models = Json.Array(codex, "models");
            if (models.Count == 0) throw new IOException("Codex model discovery is unavailable. Check the Codex login and retry.");
            client?.Dispose();
            client = candidate;
            sessionId = null;
            current = null;
            folder = null;
            selectionVersion++;
            transcript.Reset();
            drafts.Clear();
            titles.Clear();
            draftId = null;
            pendingStart = null;
            pendingPrompt = null;
            capabilities = discovered;
            efforts.Clear();
            foreach (var model in models)
            {
                string name = Json.Text(model, "model");
                if (name.Length > 0) efforts[name] = Json.Array(model, "reasoning_levels").Select(n => n?.GetValue<string>() ?? "").Where(s => s.Length > 0).ToArray();
            }
            ModelPicker.ItemsSource = efforts.Keys.ToArray();
            ModelPicker.SelectedItem = efforts.ContainsKey(Json.Text(codex, "model")) ? Json.Text(codex, "model") : efforts.Keys.First();
            connected = true;
            SetConnection(true);
            ConnectionDialog.Visibility = Visibility.Collapsed;
            ConnectionError.Text = "";
            ClearError();
            nextList = DateTime.MinValue;
            await RefreshSessions();
            try
            {
                var auth = await client.Get("/v1/accounts/codex", lifetime.Token);
                AccountLabel.Text = Json.Text(auth, "state") == "connected" ? "Codex connected · Local runtime" : "Codex · " + Json.Text(auth, "state", "Check login");
            }
            catch { AccountLabel.Text = "Codex · Check login status"; }
            SetControls();
        }
        catch { if (!ReferenceEquals(client, candidate)) candidate.Dispose(); throw; }
    }

    private void SetConnection(bool ready)
    {
        connected = ready;
        ConnectionLabel.Text = ready ? "Core connected" : "Reconnecting to Core…";
        ConnectionDot.Fill = new SolidColorBrush(ready ? Color.FromRgb(117, 211, 173) : Color.FromRgb(224, 170, 99));
    }

    private async Task RefreshSessions()
    {
        if (client == null) return;
        var source = client;
        var result = await source.Get("/v1/sessions", lifetime.Token);
        if (!ReferenceEquals(client, source)) return;
        var items = Json.Array(result, "sessions").Select(item =>
        {
            string id = Json.Text(item, "session_id");
            string path = Json.Text(item?["workspace"], "path");
            return new SessionItem(id, titles.GetValueOrDefault(id, "Codex · " + id.Replace("cr_", "")[..Math.Min(10, id.Replace("cr_", "").Length)]), Json.Text(item, "state"), Json.Text(item, "model"), Path.GetFileName(path.TrimEnd('\\', '/')));
        }).ToList();
        refreshingList = true;
        SessionList.ItemsSource = items;
        SessionList.SelectedItem = items.FirstOrDefault(i => i.Id == sessionId);
        SessionCount.Text = Json.Number(result, "total").ToString();
        refreshingList = false;
        if (sessionId == null && items.Count > 0) SessionList.SelectedItem = items[0];
        nextList = DateTime.UtcNow.AddSeconds(5);
    }

    private async Task Poll()
    {
        if (polling || client == null || lifetime.IsCancellationRequested) return;
        polling = true;
        var source = client;
        long version = selectionVersion;
        string? id = sessionId;
        try
        {
            if (DateTime.UtcNow >= nextList) await RefreshSessions();
            if (id != null)
            {
                var state = (await source.Get("/v1/sessions/" + Uri.EscapeDataString(id), lifetime.Token))["session"];
                if (version != selectionVersion || !ReferenceEquals(source, client)) return;
                current = state;
                folder = Json.Text(state?["workspace"], "path");
                string status = Json.Text(state, "state");
                StateLabel.Text = status.Replace('_', ' ').ToUpperInvariant();
                HeaderTitle.Text = titles.GetValueOrDefault(id, "Codex session");
                HeaderSubtitle.Text = folder.Length > 0 ? folder : id;
                QueueLabel.Text = Json.Array(state, "queue").Count > 0 ? Json.Array(state, "queue").Count + " message(s) queued" : status is "running" or "starting_turn" ? "Codex is working…" : "";
                for (int page = 0; page < 4; page++)
                {
                    var records = Json.Array(await source.Get("/v1/sessions/" + Uri.EscapeDataString(id) + "/events?after=" + transcript.Cursor, lifetime.Token), "events");
                    if (version != selectionVersion || !ReferenceEquals(source, client)) return;
                    var scroll = FindScroll(ConversationList);
                    bool follow = scroll == null || scroll.ScrollableHeight - scroll.VerticalOffset < 60;
                    foreach (var record in records)
                    {
                        if (record == null) continue;
                        var data = record["data"];
                        if (Json.Text(record, "kind") == "receipt" && Json.Text(data, "command") == "prompt" && !titles.ContainsKey(id))
                        {
                            string title = Json.Text(data?["input"], "text").Split('\n')[0].Trim();
                            titles[id] = title.Length > 60 ? title[..60] + "…" : title;
                            HeaderTitle.Text = titles[id];
                        }
                        transcript.Apply(record);
                    }
                    if (follow && transcript.Messages.Count > 0) ConversationList.ScrollIntoView(transcript.Messages[^1]);
                    if (records.Count < 256) break;
                }
            }
            else await source.Get("/v1/ready", lifetime.Token);
            if (ReferenceEquals(source, client)) { if (!connected) ClearError(); SetConnection(true); }
        }
        catch (Exception error)
        {
            if (!lifetime.IsCancellationRequested && ReferenceEquals(source, client)) { SetConnection(false); ShowError(SafeError(error)); }
        }
        finally { polling = false; SetControls(); }
    }

    private void SelectSession(string id)
    {
        if (sessionId != null) drafts[sessionId] = Composer.Text;
        sessionId = id;
        selectionVersion++;
        transcript.Reset();
        current = null;
        folder = null;
        Composer.Text = drafts.GetValueOrDefault(id, "");
        HeaderTitle.Text = titles.GetValueOrDefault(id, "Codex session");
        HeaderSubtitle.Text = "Loading saved conversation…";
        StateLabel.Text = "LOADING";
        QueueLabel.Text = "";
        if (SessionList.SelectedItem is SessionItem item && efforts.ContainsKey(item.Model)) ModelPicker.SelectedItem = item.Model;
        SetControls();
    }

    private async Task<string> CreateSession()
    {
        if (client == null) throw new IOException("Connect to Core first.");
        if (pendingStart == null)
        {
            string model = ModelPicker.SelectedItem as string ?? throw new IOException("Select a model first.");
            pendingStart = new JsonObject { ["request_id"] = Json.Id(), ["harness"] = "codex", ["model"] = model };
            if (EffortPicker.SelectedItem is string effort && effort.Length > 0) pendingStart["reasoning"] = effort;
            if (NewWorkspaceCheck.IsChecked == true)
            {
                string name = ProjectBox.Text.Trim();
                if (!Regex.IsMatch(name, "^[A-Za-z0-9][A-Za-z0-9_.-]{0,79}$")) { pendingStart = null; throw new IOException("Use 1–80 letters, numbers, dots, hyphens or underscores for the folder name."); }
                pendingStart["workspace"] = "gui_" + Json.Id();
                pendingStart["workspace_name"] = name;
            }
        }
        var accepted = await client.Post("/v1/sessions", pendingStart, lifetime.Token);
        pendingStart = null;
        string id = Json.Text(accepted, "session_id");
        SelectSession(id);
        NewDialog.Visibility = Visibility.Collapsed;
        await RefreshSessions();
        var deadline = DateTime.UtcNow.AddSeconds(90);
        do
        {
            var state = (await client.Get("/v1/sessions/" + id, lifetime.Token))["session"];
            if (Json.Text(state, "state") is "idle" or "sleeping") { current = state; return id; }
            if (Json.Text(state, "state") is "failed" or "process_lost" or "closed") throw new IOException(Json.Text(state, "startup_reason", "The session could not start. Check your Codex login."));
            await Task.Delay(200, lifetime.Token);
        } while (DateTime.UtcNow < deadline);
        throw new IOException("Session startup is still pending. It remains in the session list; check its status before sending again.");
    }

    private async Task SendDraft()
    {
        string text = Composer.Text.Trim();
        if (text.Length == 0) return;
        if (Encoding.UTF8.GetByteCount(text) > 32768) throw new IOException("Keep a message within 32,768 UTF-8 bytes.");
        if (client == null) throw new IOException("Connect to Core first.");
        string id = sessionId ?? await CreateSession();
        // A lost acceptance is retried with the same ID and payload, never silently duplicated.
        if (draftId == null || draftText != text || draftSession != id)
        {
            draftId = Json.Id(); draftText = text; draftSession = id;
            pendingPrompt = new JsonObject { ["request_id"] = draftId, ["text"] = text };
            if (EffortPicker.SelectedItem is string effort && effort.Length > 0) pendingPrompt["reasoning"] = effort;
        }
        await client.Post("/v1/sessions/" + id + "/prompts", pendingPrompt!, lifetime.Token);
        draftId = null;
        pendingPrompt = null;
        drafts.Remove(id);
        Composer.Clear();
        await Poll();
        Composer.Focus();
    }

    private async Task Act(Func<Task> operation)
    {
        if (acting) return;
        acting = true;
        SetControls();
        ClearError();
        try { await operation(); }
        catch (Exception error)
        {
            if (!lifetime.IsCancellationRequested)
            {
                string message = SafeError(error);
                ShowError(message);
                if (NewDialog.Visibility == Visibility.Visible) NewSessionError.Text = message;
            }
        }
        finally { acting = false; nextList = DateTime.MinValue; SetControls(); }
    }

    private async Task SessionCommand(string command, JsonObject? body = null)
    {
        if (client == null || sessionId == null) return;
        body ??= new JsonObject();
        body["request_id"] = Json.Id();
        await client.Post("/v1/sessions/" + sessionId + "/" + command, body, lifetime.Token);
        await Poll();
    }

    private void SetControls()
    {
        bool ready = connected && !acting;
        bool hasSession = sessionId != null && current != null;
        bool closed = Json.Text(current, "state") == "closed";
        NewButton.IsEnabled = ready;
        CreateButton.IsEnabled = ready;
        CreateButton.Content = acting ? "Starting…" : pendingStart != null ? "Retry session" : "Create session";
        SendButton.IsEnabled = ready && !closed && ModelPicker.SelectedItem != null;
        SessionList.IsEnabled = !acting;
        Composer.IsReadOnly = acting;
        ModelPicker.IsEnabled = ready && sessionId == null;
        EffortPicker.IsEnabled = ready && !closed;
        PauseButton.IsEnabled = ready && hasSession && !closed;
        PauseButton.Content = Json.Flag(current, "queue_paused") || Json.Text(current, "state") is "suspended" or "process_lost" or "failed" ? "Resume queue" : "Pause queue";
        InterruptButton.IsEnabled = ready && hasSession && Json.Text(current, "current_request").Length > 0 && Json.Flag(capabilities, "stop");
        CloseButton.IsEnabled = ready && hasSession && !closed;
        FolderButton.IsEnabled = !string.IsNullOrEmpty(folder) && Directory.Exists(folder);
        SendButton.Content = acting ? "Working…" : draftId != null ? "Retry send ↗" : "Send  ↗";
    }

    private void ModelChanged(object sender, SelectionChangedEventArgs e)
    {
        if (EffortPicker == null) return;
        string? previous = EffortPicker.SelectedItem as string;
        string[] levels = ModelPicker.SelectedItem is string model ? efforts.GetValueOrDefault(model, []) : [];
        EffortPicker.ItemsSource = levels;
        EffortPicker.SelectedItem = levels.Contains(previous) ? previous : levels.Contains("medium") ? "medium" : levels.FirstOrDefault();
    }
    private async void SessionSelected(object sender, SelectionChangedEventArgs e)
    {
        if (refreshingList || SessionList.SelectedItem is not SessionItem item || item.Id == sessionId) return;
        SelectSession(item.Id);
        await Poll();
    }
    private async void SendMessage(object sender, RoutedEventArgs e) => await Act(SendDraft);
    private async void CreateSessionClicked(object sender, RoutedEventArgs e) => await Act(async () => { await CreateSession(); await Poll(); });
    private async void PauseQueue(object sender, RoutedEventArgs e) => await Act(() => SessionCommand((string)PauseButton.Content == "Resume queue" ? "resume" : "stop"));
    private async void Interrupt(object sender, RoutedEventArgs e) => await Act(() => SessionCommand("interrupt", new JsonObject { ["target_request_id"] = Json.Text(current, "current_request") }));
    private async void CloseSession(object sender, RoutedEventArgs e) => await Act(() => SessionCommand("close"));
    private void ShowNewSession(object sender, RoutedEventArgs e) { NewSessionError.Text = ""; NewDialog.Visibility = Visibility.Visible; NewWorkspaceCheck.IsChecked = false; }
    private void ShowConnection(object sender, RoutedEventArgs e) { ConnectionDialog.Visibility = Visibility.Visible; ConnectionError.Text = ""; }
    private void DismissDialog(object sender, RoutedEventArgs e) { ConnectionDialog.Visibility = Visibility.Collapsed; NewDialog.Visibility = Visibility.Collapsed; }
    private async void LoadSettings(object sender, RoutedEventArgs e)
    {
        try { await LoadLocalSettings(); ConnectionError.Text = "Settings loaded. Select Connect to continue."; }
        catch (Exception error) { ConnectionError.Text = SafeError(error); }
    }
    private async void ConnectClicked(object sender, RoutedEventArgs e)
    {
        await Act(async () => { try { await Connect(); } catch (Exception error) { ConnectionError.Text = SafeError(error); throw; } });
    }
    private void ChooseSuggestion(object sender, RoutedEventArgs e) { Composer.Text = ((Button)sender).Tag as string ?? ""; Composer.Focus(); Composer.CaretIndex = Composer.Text.Length; }
    private async void ComposerKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key == Key.Enter && Keyboard.Modifiers == ModifierKeys.None) { e.Handled = true; if (SendButton.IsEnabled) await Act(SendDraft); }
    }
    private void OpenFolder(object sender, RoutedEventArgs e)
    {
        if (folder == null || !Directory.Exists(folder)) return;
        try { var info = new ProcessStartInfo("explorer.exe") { UseShellExecute = false }; info.ArgumentList.Add(folder); Process.Start(info); }
        catch (Exception error) { ShowError(SafeError(error)); }
    }
    private void UpdateWelcome() { Welcome.Visibility = transcript.Messages.Count == 0 ? Visibility.Visible : Visibility.Collapsed; ConversationList.Visibility = transcript.Messages.Count > 0 ? Visibility.Visible : Visibility.Collapsed; }
    private void ShowError(string text) { ErrorText.Text = text; ErrorBanner.Visibility = Visibility.Visible; }
    private void ClearError() { ErrorBanner.Visibility = Visibility.Collapsed; ErrorText.Text = ""; }
    private static string SafeError(Exception error) => error is TaskCanceledException ? "Core did not respond in time. Your draft is kept; retry it unchanged to reuse the request ID." : error is HttpRequestException ? "Cannot reach Core. Start the local runtime and check Connection settings. Your draft is kept." : error.Message;
    private static ScrollViewer? FindScroll(DependencyObject parent)
    {
        if (parent is ScrollViewer viewer) return viewer;
        for (int i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++) { var found = FindScroll(VisualTreeHelper.GetChild(parent, i)); if (found != null) return found; }
        return null;
    }
    private void MinimizeWindow(object sender, RoutedEventArgs e) => WindowState = WindowState.Minimized;
    private void MaximizeWindow(object sender, RoutedEventArgs e) => WindowState = WindowState == WindowState.Maximized ? WindowState.Normal : WindowState.Maximized;
    private void CloseWindow(object sender, RoutedEventArgs e) => Close();
    private void WindowClosed(object? sender, EventArgs e) { timer.Stop(); lifetime.Cancel(); client?.Dispose(); lifetime.Dispose(); }

    private async Task SmokeTest(string output)
    {
        if (client == null || !efforts.ContainsKey("fixture")) throw new IOException("Smoke tests require the deterministic fixture backend, never a real model.");
        Directory.CreateDirectory(output);
        foreach (string host in new[] { "https://example.com", "http://127.0.0.1:9840/path", "http://127.0.0.1:9840/?token=test" })
        {
            try { using var invalid = new CoreClient(host, new string('x', 32)); throw new IOException("Unsafe endpoint accepted."); }
            catch (ArgumentException) { }
        }
        await Capture(Path.Combine(output, "welcome.png"));
        ModelPicker.SelectedItem = "fixture";
        NewWorkspaceCheck.IsChecked = true;
        ProjectBox.Text = "gui-project";
        string id = await CreateSession();
        Composer.Text = "Hello from the Windows GUI — ø";
        bool retried = false;
        try { await SendDraft(); }
        catch (Exception error) when (error is HttpRequestException or IOException)
        {
            if (pendingPrompt == null || Composer.Text.Length == 0) throw new IOException("A lost response discarded the pending draft.");
            if (efforts["fixture"].Contains("high")) EffortPicker.SelectedItem = "high";
            await SendDraft();
            retried = true;
        }
        var deadline = DateTime.UtcNow.AddSeconds(30);
        do { await Poll(); if (transcript.Messages.Any(m => m.Role == "CODEX" && m.Text.Contains("hello"))) break; await Task.Delay(100); } while (DateTime.UtcNow < deadline);
        if (!transcript.Messages.Any(m => m.Role == "YOU" && m.Text.Contains("Windows GUI")) || !transcript.Messages.Any(m => m.Role == "CODEX" && m.Text.Contains("hello"))) throw new IOException("Conversation did not render the fixture response.");
        await WaitForState(s => s?["receipts"] is JsonObject receipts && receipts.Any(r => Json.Text(r.Value, "command") == "prompt" && Json.Text(r.Value, "state") == "completed"));
        long cursor = transcript.Cursor;
        int count = transcript.Messages.Count;
        await Poll();
        if (count != transcript.Messages.Count || cursor != transcript.Cursor) throw new IOException("Polling duplicated the transcript.");
        await RefreshSessions();
        await Capture(Path.Combine(output, "conversation.png"));
        await SessionCommand("stop");
        await WaitForState(s => Json.Flag(s, "queue_paused"));
        await SessionCommand("resume");
        await WaitForState(s => !Json.Flag(s, "queue_paused"));
        await SessionCommand("close");
        await WaitForState(s => Json.Text(s, "state") == "closed");
        // A fresh reducer proves that history replay reconstructs the visible conversation.
        var replay = new Transcript();
        foreach (var record in Json.Array(await client.Get("/v1/sessions/" + id + "/events"), "events")) if (record != null) replay.Apply(record);
        if (!replay.Messages.Any(m => m.Role == "CODEX" && m.Text.Contains("hello"))) throw new IOException("Transcript replay lost the assistant response.");
        await File.WriteAllTextAsync(Path.Combine(output, "result.json"), new JsonObject { ["passed"] = true, ["retried_lost_acceptance"] = retried, ["session"] = id, ["messages"] = replay.Messages.Count, ["checks"] = "connection validation, project creation, prompt, live output, duplicate-free polling, pause/resume/close, history replay, lost-response retry" }.ToJsonString());
    }
    private async Task WaitForState(Func<JsonNode?, bool> predicate)
    {
        var deadline = DateTime.UtcNow.AddSeconds(20);
        do { await Poll(); if (predicate(current)) return; await Task.Delay(100); } while (DateTime.UtcNow < deadline);
        throw new IOException("The fixture command did not complete.");
    }
    private async Task Capture(string path)
    {
        await Dispatcher.InvokeAsync(() => { }, DispatcherPriority.ApplicationIdle);
        UpdateLayout();
        var bitmap = new RenderTargetBitmap((int)ActualWidth, (int)ActualHeight, 96, 96, PixelFormats.Pbgra32);
        bitmap.Render(this);
        var encoder = new PngBitmapEncoder();
        encoder.Frames.Add(BitmapFrame.Create(bitmap));
        using var file = File.Create(path);
        encoder.Save(file);
    }
}
