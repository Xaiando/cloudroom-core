using System.IO;
using System.Net.Http;
using System.Text;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using Microsoft.Win32;

namespace Cloudroom.Desktop;

public sealed record LibraryItem(string Id, string Title, string Detail, JsonNode State);
public sealed record LibraryRevision(string Label, JsonNode Content);

public partial class MainWindow
{
    private JsonNode? libraryState;
    private string? libraryId, libraryIdentity;
    private string libraryRequest = Json.Id(), libraryBaseline = "";
    private long libraryRevision;
    private bool libraryLoading, libraryDirty, libraryOpen, librarySaving, libraryRestoring;
    private string LibraryType => (LibraryKind.SelectedItem as ComboBoxItem)?.Tag as string ?? "docs";
    private static string ComboTag(ComboBox combo) => (combo.SelectedItem as ComboBoxItem)?.Tag as string ?? "";
    private static void SelectTag(ComboBox combo, string tag) => combo.SelectedItem = combo.Items.OfType<ComboBoxItem>().FirstOrDefault(i => i.Tag as string == tag);
    private string LibraryPlugin => LibraryType == "docs" ? "simple-notes" : LibraryType;

    private async void ShowLibrary(object sender, RoutedEventArgs e) => await Act(OpenLibrary);
    private async Task OpenLibrary()
    {
        SaveCouncilDraft(); CouncilWorkspacePanel.Visibility = Visibility.Collapsed;
        if (client == null || !connected) throw new IOException("Connect to Core first.");
        ExtensionsPane.Visibility = LibraryView.Visibility = Visibility.Visible;
        ConversationPane.Visibility = AutomationsView.Visibility = PluginsView.Visibility = Visibility.Collapsed;
        ExtensionTitle.Text = "Docs · Memory · Tasks";
        ExtensionSubtitle.Text = "Private content, workspace context and task lists in Cloud Bee.";
        ExtensionsError.Text = "";
        await RefreshLibrary();
        if (libraryIdentity != client.Identity)
        {
            libraryIdentity = client.Identity;
            libraryOpen = false;
            RestoreLibraryDraft();
        }
        if (!libraryOpen) LoadLibraryEntry(null);
    }
    private async Task RefreshLibrary()
    {
        if (client == null) return;
        libraryState = await client.Get("/v1/bee/library", lifetime.Token);
        libraryLoading = true;
        string scope = LibraryWorkspace.SelectedValue as string ?? "";
        var workspaces = new List<AutomationWorkspaceItem> { new("", "Global / unassigned"), new("legacy", "Default workspace") };
        workspaces.AddRange(Json.Array(libraryState, "workspaces").Select(w => new AutomationWorkspaceItem(Json.Text(w, "id"), Json.Text(w, "path"))));
        if (scope.Length > 0 && !workspaces.Any(w => w.Id == scope)) workspaces.Add(new(scope, "Unavailable workspace · " + scope));
        LibraryWorkspace.ItemsSource = workspaces;
        LibraryWorkspace.SelectedValue = workspaces.Any(w => w.Id == scope) ? scope : "";
        libraryLoading = false;
        ApplyLibraryFilter();
    }
    private void ApplyLibraryFilter()
    {
        if (LibraryList == null) return;
        string query = LibrarySearch.Text.Trim();
        var items = Json.Array(libraryState, "entries").Where(e => Json.Text(e?["content"], "kind") == LibraryType && Json.Flag(e, "archived") == (LibraryArchived.IsChecked == true))
            .Where(e => query.Length == 0 || (Json.Text(e?["content"], "title") + "\n" + Json.Text(e?["content"], "body") + "\n" + e?["content"]?["tags"]?.ToJsonString()).Contains(query, StringComparison.OrdinalIgnoreCase))
            .OrderByDescending(e => Json.Number(e, "updated_at"))
            .Select(e => new LibraryItem(Json.Text(e, "id"), Json.Text(e?["content"], "title"),
                (LibraryType == "tasks" ? Json.Text(e?["content"], "status").Replace('_', ' ') + " · " : "") + (Json.Text(e?["content"], "workspace") is { Length: > 0 } w ? w : "Global / unassigned") + " · v" + Json.Number(e, "revision"), e!)).ToList();
        libraryLoading = true;
        LibraryList.ItemsSource = items;
        LibraryList.SelectedItem = items.FirstOrDefault(e => e.Id == libraryId);
        bool enabled = Json.Flag(libraryState?["enabled"], LibraryType);
        LibraryEnable.Visibility = enabled ? Visibility.Collapsed : Visibility.Visible;
        LibraryNotice.Text = !enabled ? "Install or enable this plugin to edit. Existing entries remain readable." : LibraryType switch
        {
            "memory" => "Select a scope and check Include to add this memory to new messages. Saved requests retain their original context. Up to 10 KB of selected memory is included per message.",
            "tasks" => "Track task status. Prepare message copies the selected task into your conversation draft for review; it does not start an agent.",
            _ => "Edit private Markdown documents. Import/export uses files you select. Vault sync and rich document previews are still pending."
        };
        LibraryTaskStatus.Visibility = LibraryType == "tasks" ? Visibility.Visible : Visibility.Collapsed;
        LibraryMemoryKind.Visibility = LibraryContext.Visibility = LibraryType == "memory" ? Visibility.Visible : Visibility.Collapsed;
        libraryLoading = false;
    }
    private void RequireSavedLibrary()
    {
        if (libraryDirty) throw new IOException("Save this library draft or choose Discard draft before selecting another entry or type.");
    }
    private async void LibrarySelected(object sender, SelectionChangedEventArgs e)
    {
        if (libraryLoading || LibraryList.SelectedItem is not LibraryItem entry || entry.Id == libraryId) return;
        await Act(async () =>
        {
            try { RequireSavedLibrary(); }
            catch { ApplyLibraryFilter(); throw; }
            if (client == null) return;
            var detail = await client.Get("/v1/bee/library/" + entry.Id, lifetime.Token);
            LoadLibraryEntry(detail["entry"]);
        });
    }
    private void LoadLibraryEntry(JsonNode? entry)
    {
        libraryLoading = true;
        libraryId = entry == null ? null : Json.Text(entry, "id");
        libraryRevision = Json.Number(entry, "revision");
        var content = entry?["content"];
        if (content != null) SelectTag(LibraryKind, Json.Text(content, "kind"));
        LibraryTitle.Text = Json.Text(content, "title"); LibraryBody.Text = Json.Text(content, "body");
        LibraryTags.Text = string.Join(", ", Json.Array(content, "tags").Select(t => t?.GetValue<string>() ?? ""));
        string workspace = Json.Text(content, "workspace");
        if (workspace.Length > 0 && LibraryWorkspace.ItemsSource is List<AutomationWorkspaceItem> workspaces && !workspaces.Any(w => w.Id == workspace))
        {
            LibraryWorkspace.ItemsSource = workspaces.Append(new AutomationWorkspaceItem(workspace, "Unavailable workspace · " + workspace)).ToList();
        }
        LibraryWorkspace.SelectedValue = workspace;
        SelectTag(LibraryTaskStatus, Json.Text(content, "status", "todo")); SelectTag(LibraryMemoryKind, Json.Text(content, "memory_kind", "fact"));
        LibraryContext.IsChecked = Json.Flag(content, "use_in_context");
        LibraryHistory.ItemsSource = Json.Array(entry, "history").Select(h => new LibraryRevision("Revision " + Json.Number(h, "revision"), h!["content"]!.DeepClone())).Reverse().ToList();
        LibraryHistory.SelectedIndex = -1;
        libraryLoading = false; libraryOpen = true; libraryDirty = false; libraryRequest = Json.Id();
        libraryBaseline = LibraryContent().ToJsonString();
        LibraryDraftStatus.Text = entry == null ? "New entry" : Json.Flag(entry, "archived") ? "Archived · restore to edit" : "Saved · revision " + libraryRevision;
        ApplyLibraryFilter(); SaveLibraryDraft();
    }
    private JsonObject LibraryContent() => new()
    {
        ["kind"] = LibraryType, ["title"] = LibraryTitle.Text, ["body"] = LibraryBody.Text,
        ["workspace"] = LibraryWorkspace.SelectedValue is string { Length: > 0 } scope ? scope : null,
        ["tags"] = new JsonArray(LibraryTags.Text.Split(',', StringSplitOptions.TrimEntries | StringSplitOptions.RemoveEmptyEntries).Distinct(StringComparer.OrdinalIgnoreCase).Select(t => (JsonNode?)JsonValue.Create(t)).ToArray()),
        ["status"] = ComboTag(LibraryTaskStatus), ["memory_kind"] = ComboTag(LibraryMemoryKind), ["use_in_context"] = LibraryContext.IsChecked == true
    };
    private void MarkLibraryEdited()
    {
        if (libraryLoading || !libraryOpen || LibraryBody == null) return;
        libraryDirty = LibraryContent().ToJsonString() != libraryBaseline;
        LibraryDraftStatus.Text = libraryDirty ? "Unsaved draft" : "Saved · revision " + libraryRevision;
    }
    private void LibraryEdited(object sender, TextChangedEventArgs e) => MarkLibraryEdited();
    private void LibraryOptionEdited(object sender, SelectionChangedEventArgs e) => MarkLibraryEdited();
    private void LibraryFilterEdited(object sender, RoutedEventArgs e) => MarkLibraryEdited();
    private void LibrarySearchChanged(object sender, TextChangedEventArgs e) { if (LibraryList != null) ApplyLibraryFilter(); }
    private void LibraryFilterChanged(object sender, RoutedEventArgs e) => ApplyLibraryFilter();
    private async void LibraryKindChanged(object sender, SelectionChangedEventArgs e)
    {
        if (libraryLoading || LibraryView == null || !libraryOpen) return;
        await Act(() =>
        {
            if (libraryDirty)
            {
                libraryLoading = true; SelectTag(LibraryKind, Json.Text(JsonNode.Parse(libraryBaseline), "kind")); libraryLoading = false;
                RequireSavedLibrary();
            }
            LoadLibraryEntry(null); return Task.CompletedTask;
        });
    }
    private async void NewLibraryEntry(object sender, RoutedEventArgs e) => await Act(() => { RequireSavedLibrary(); LoadLibraryEntry(null); return Task.CompletedTask; });
    private async void RefreshLibraryClicked(object sender, RoutedEventArgs e) => await Act(RefreshLibrary);
    private async void EnableLibraryPlugin(object sender, RoutedEventArgs e) => await Act(async () => { await ExtensionPost("/v1/bee/plugins/" + LibraryPlugin + "/install", new JsonObject()); await RefreshLibrary(); });
    private async void SaveLibraryClicked(object sender, RoutedEventArgs e) => await Act(SaveLibrary);
    private async Task SaveLibrary()
    {
        if (client == null || librarySaving) return;
        librarySaving = true;
        LibraryView.IsEnabled = false;
        try
        {
            string path = libraryId == null ? "/v1/bee/library" : "/v1/bee/library/" + libraryId + "/update";
            var body = new JsonObject { ["request_id"] = libraryRequest, ["content"] = LibraryContent(), ["authorize_context"] = LibraryContext.IsChecked == true };
            if (libraryId != null) body["expected_revision"] = libraryRevision;
            if (extensionOutbox.TryGetValue(path, out var pending) && pending.ToJsonString() != body.ToJsonString()) throw new IOException("An earlier save has no receipt. Retry it unchanged before editing further; the pending content is retained.");
            extensionOutbox[path] = (JsonObject)body.DeepClone(); SaveLibraryDraft();
            var result = await ExtensionPost(path, body);
            libraryId = Json.Text(result, "id");
            libraryRevision = Json.Number(result, "revision"); libraryDirty = false;
            libraryBaseline = LibraryContent().ToJsonString(); libraryRequest = Json.Id(); SaveLibraryDraft();
            var detail = await client.Get("/v1/bee/library/" + libraryId, lifetime.Token);
            await RefreshLibrary(); LoadLibraryEntry(detail["entry"]);
        }
        catch (CoreApiException) { libraryRequest = Json.Id(); SaveLibraryDraft(); throw; }
        finally { librarySaving = false; LibraryView.IsEnabled = true; }
    }
    private async void DiscardLibraryDraft(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (extensionOutbox.Keys.Any(k => k.StartsWith("/v1/bee/library", StringComparison.Ordinal))) throw new IOException("Retry the pending library save unchanged before discarding; Core may already have accepted it.");
        if (libraryId != null && client != null) LoadLibraryEntry((await client.Get("/v1/bee/library/" + libraryId, lifetime.Token))["entry"]);
        else LoadLibraryEntry(null);
    });
    private async void RecoverLibrarySave(object sender, RoutedEventArgs e) => await Act(() => { RecoverLibraryPending(); return Task.CompletedTask; });
    private void RecoverLibraryPending()
    {
        var pending = extensionOutbox.FirstOrDefault(p => p.Key == "/v1/bee/library" || p.Key.EndsWith("/update", StringComparison.Ordinal) && p.Key.StartsWith("/v1/bee/library/", StringComparison.Ordinal));
        if (pending.Value == null) throw new IOException("There is no pending library save to recover.");
        string? id = pending.Key == "/v1/bee/library" ? null : pending.Key.Split('/')[4];
        libraryRestoring = true;
        try { LoadLibraryEntry(new JsonObject { ["id"] = id, ["revision"] = Json.Number(pending.Value, "expected_revision"), ["content"] = pending.Value["content"]!.DeepClone() }); }
        finally { libraryRestoring = false; }
        libraryId = id; libraryRequest = Json.Text(pending.Value, "request_id"); libraryDirty = true;
        LibraryDraftStatus.Text = "Pending save restored · Save retries the same request"; SaveLibraryDraft();
    }
    private async void ArchiveLibraryEntry(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        RequireSavedLibrary();
        if (libraryId == null || client == null) throw new IOException("Save or select an entry first.");
        var detail = (await client.Get("/v1/bee/library/" + libraryId, lifetime.Token))["entry"];
        await ExtensionPost("/v1/bee/library/" + libraryId + (Json.Flag(detail, "archived") ? "/restore" : "/archive"), new JsonObject { ["expected_revision"] = libraryRevision });
        await RefreshLibrary(); LoadLibraryEntry((await client.Get("/v1/bee/library/" + libraryId, lifetime.Token))["entry"]);
    });
    private async void LoadLibraryRevision(object sender, RoutedEventArgs e) => await Act(() =>
    {
        RequireSavedLibrary();
        if (LibraryHistory.SelectedItem is not LibraryRevision revision) throw new IOException("Select a previous revision first.");
        var saved = revision.Content;
        LibraryTitle.Text = Json.Text(saved, "title"); LibraryBody.Text = Json.Text(saved, "body");
        LibraryTags.Text = string.Join(", ", Json.Array(saved, "tags").Select(t => t?.GetValue<string>() ?? ""));
        LibraryWorkspace.SelectedValue = Json.Text(saved, "workspace");
        SelectTag(LibraryTaskStatus, Json.Text(saved, "status")); SelectTag(LibraryMemoryKind, Json.Text(saved, "memory_kind")); LibraryContext.IsChecked = Json.Flag(saved, "use_in_context"); MarkLibraryEdited();
        return Task.CompletedTask;
    });
    private async void PrepareLibraryMessage(object sender, RoutedEventArgs e) => await Act(() =>
    {
        if (sessionId == null) throw new IOException("Select a conversation first, then reopen the library.");
        string text = LibraryTitle.Text + "\n\n" + LibraryBody.Text;
        if (Encoding.UTF8.GetByteCount(Composer.Text + text) > 24000) throw new IOException("This content is too long for a conversation message; select a shorter excerpt.");
        Composer.Text += (Composer.Text.Length > 0 ? "\n\n" : "") + text;
        SaveLibraryDraft(); BackToConversation(this, new RoutedEventArgs()); SaveOutbox(); Composer.Focus();
        return Task.CompletedTask;
    });
    private async void ImportLibraryText(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        var dialog = new OpenFileDialog { Filter = "Markdown and text|*.md;*.txt|All files|*.*", CheckFileExists = true };
        if (dialog.ShowDialog(this) != true) return;
        if (new FileInfo(dialog.FileName).Length > 49152) throw new IOException("Import is limited to 48 KiB of UTF-8 text.");
        string text = await File.ReadAllTextAsync(dialog.FileName, new UTF8Encoding(false, true));
        LibraryBody.Text = text;
        if (LibraryTitle.Text.Length == 0) LibraryTitle.Text = Path.GetFileNameWithoutExtension(dialog.FileName);
        MarkLibraryEdited(); SaveLibraryDraft();
    });
    private async void ExportLibraryText(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        var dialog = new SaveFileDialog { Filter = "Markdown|*.md|Text|*.txt", FileName = "cloud-bee-note.md" };
        if (dialog.ShowDialog(this) == true) await File.WriteAllTextAsync(dialog.FileName, LibraryBody.Text, new UTF8Encoding(false));
    });
    private string LibraryDraftPath() => Path.ChangeExtension(OutboxPath(), ".library.json");
    private void SaveLibraryDraft()
    {
        if (client == null || !libraryOpen || libraryRestoring || libraryIdentity != client.Identity) return;
        var pending = new JsonObject();
        foreach (var pair in extensionOutbox.Where(p => p.Key.StartsWith("/v1/bee/library", StringComparison.Ordinal))) pending[pair.Key] = pair.Value.DeepClone();
        var draft = new JsonObject { ["id"] = libraryId, ["revision"] = libraryRevision, ["request_id"] = libraryRequest, ["content"] = LibraryContent(), ["baseline"] = libraryBaseline, ["dirty"] = libraryDirty, ["pending"] = pending };
        string path = LibraryDraftPath(); File.WriteAllText(path + ".pending", draft.ToJsonString()); File.Move(path + ".pending", path, true);
    }
    private void RestoreLibraryDraft()
    {
        if (!File.Exists(LibraryDraftPath())) return;
        var draft = JsonNode.Parse(File.ReadAllText(LibraryDraftPath())) ?? throw new IOException("Cannot read the saved library draft.");
        libraryRestoring = true;
        try { LoadLibraryEntry(new JsonObject { ["id"] = draft["id"]?.DeepClone(), ["revision"] = draft["revision"]?.DeepClone(), ["content"] = draft["content"]?.DeepClone() }); }
        finally { libraryRestoring = false; }
        libraryId = Json.Text(draft, "id") is { Length: > 0 } id ? id : null;
        libraryRequest = Json.Text(draft, "request_id"); libraryBaseline = Json.Text(draft, "baseline"); libraryDirty = Json.Flag(draft, "dirty");
        if (draft["pending"] is JsonObject pending) foreach (var pair in pending) if (pair.Value is JsonObject body) extensionOutbox[pair.Key] = (JsonObject)body.DeepClone();
        LibraryDraftStatus.Text = libraryDirty ? "Restored unsaved draft · review before saving" : "Saved · revision " + libraryRevision;
    }
    private async Task LibrarySmokeTest(string output)
    {
        await OpenLibrary();
        foreach (string kind in new[] { "docs", "memory", "tasks" })
        {
            SelectTag(LibraryKind, kind);
            await ExtensionPost("/v1/bee/plugins/" + LibraryPlugin + "/install", new JsonObject()); await RefreshLibrary(); LoadLibraryEntry(null);
            LibraryTitle.Text = kind == "docs" ? "Windows project notes" : kind == "memory" ? "Project preference" : "Verify the Windows release";
            LibraryBody.Text = kind == "docs" ? "# Cloud Bee\n\nKeep our native GUI.\n\n- Check launch errors\n- Preserve history\n\nUnicode: ø — 蜂" : kind == "memory" ? "Use native Windows paths and verify changes." : "Run the disposable acceptance fixtures before publishing.";
            LibraryTags.Text = "windows, cloud-bee";
            try { await SaveLibrary(); }
            catch (Exception error) when (error is HttpRequestException or IOException && error is not CoreApiException)
            {
                if (!extensionOutbox.ContainsKey("/v1/bee/library")) throw;
                SaveLibraryDraft(); RestoreLibraryDraft(); RecoverLibraryPending(); await SaveLibrary();
            }
            string id = libraryId!;
            LibraryBody.Text += "\nReviewed."; await SaveLibrary();
            if (LibraryHistory.Items.Count != 1) throw new IOException("Library revision was not retained.");
            if (kind == "memory")
            {
                var original = (await client!.Get("/v1/bee/library/" + id, lifetime.Token))["entry"]!;
                var unavailable = original.DeepClone(); unavailable["content"]!["workspace"] = "missing-fixture-scope";
                LoadLibraryEntry(unavailable); await RefreshLibrary();
                if (LibraryWorkspace.SelectedValue as string != "missing-fixture-scope") throw new IOException("Missing workspace silently became global memory.");
                LoadLibraryEntry(original);
            }
            LibrarySearch.Text = "NOT PRESENT";
            if (LibraryList.Items.Count != 0) throw new IOException("Library search did not filter.");
            LibrarySearch.Text = "";
            await Capture(Path.Combine(output, "cloud-bee-" + kind + ".png"));
            LibraryBody.Text += "\nUnsaved preserved draft"; SaveLibraryDraft(); RestoreLibraryDraft();
            if (!libraryDirty || !LibraryBody.Text.Contains("Unsaved preserved draft")) throw new IOException("Library draft did not survive restoration.");
            LoadLibraryEntry((await client!.Get("/v1/bee/library/" + id, lifetime.Token))["entry"]);
            await ExtensionPost("/v1/bee/library/" + id + "/archive", new JsonObject { ["expected_revision"] = libraryRevision }); await RefreshLibrary();
            if (LibraryList.Items.OfType<LibraryItem>().Any(i => i.Id == id)) throw new IOException("Archived entry stayed visible.");
            var archived = (await client!.Get("/v1/bee/library/" + id, lifetime.Token))["entry"];
            await ExtensionPost("/v1/bee/library/" + id + "/restore", new JsonObject { ["expected_revision"] = Json.Number(archived, "revision") }); await RefreshLibrary(); LoadLibraryEntry(null);
        }
        await File.WriteAllTextAsync(Path.Combine(output, "library-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "native Docs/Memory/Tasks, create/edit/revisions, search, archive/restore" }.ToJsonString());
        BackToConversation(this, new RoutedEventArgs());
    }
}
