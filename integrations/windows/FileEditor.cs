using System.IO;
using System.Net.Http;
using System.Text;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;

namespace Cloudroom.Desktop;

public sealed record FileRecovery(string Label, string Text, bool Bom);
public partial class MainWindow
{
    private string fileRevision = "", fileBaseline = "", fileNewline = "lf", fileLanguage = "text";
    private string? fileIdentity;
    private JsonObject? pendingFileSave;
    private bool fileLoading, fileSaving, fileMarkdown, fileBom, fileBaselineBom;
    private bool FileDirty => EditorText() != fileBaseline || fileBom != fileBaselineBom;
    private static string Lf(string value) => value.Replace("\r\n", "\n").Replace('\r', '\n');
    private string EditorText() => fileNewline == "crlf" ? Lf(FileEditor.Text).Replace("\n", "\r\n") : Lf(FileEditor.Text);
    private void RequireSavedFile()
    {
        if (pendingFileSave != null || FileDirty) throw new IOException("Save or discard the file draft before opening another file. An unconfirmed save must be retried or checked with Reload disk.");
    }
    private void LoadEditor(JsonNode result, bool persist = true)
    {
        fileLoading = true;
        try
        {
            fileIdentity = client!.Identity;
            previewPath = Json.Text(result, "path"); previewText = Json.Text(result, "text");
            fileRevision = Json.Text(result, "revision"); fileBaseline = previewText; fileNewline = Json.Text(result, "newline", "lf");
            fileLanguage = Json.Text(result, "language", "text"); fileMarkdown = Json.Flag(result, "markdown");
            fileBom = fileBaselineBom = Json.Flag(result, "bom");
            FileEditor.Text = previewText; FileDiskText.Text = previewText;
            FilePreview.CodeLanguage = fileMarkdown ? "" : fileLanguage; FilePreview.SourceText = previewText; FilePreview.RenderNow();
            FilePreviewTitle.Text = previewPath + " · " + Json.Number(result, "size") + " bytes";
        }
        finally { fileLoading = false; }
        UpdateFileControls(); if (persist) SaveFileDraft();
    }
    private void UpdateFileControls()
    {
        if (FileEditor == null) return;
        bool loaded = previewPath.Length > 0 && fileRevision.Length > 0;
        FileEditButton.IsEnabled = FileReviewButton.IsEnabled = FileReloadButton.IsEnabled = loaded && !fileSaving;
        FileSaveButton.IsEnabled = loaded && !fileSaving && (FileDirty || pendingFileSave != null);
        FileDiscardButton.IsEnabled = loaded && !fileSaving && pendingFileSave == null;
        FileEditor.IsReadOnly = fileSaving || pendingFileSave != null;
        FileDraftStatus.Text = pendingFileSave != null ? "Unconfirmed save · Retry save or Reload disk to check" : FileDirty ? "Unsaved draft · Save writes this workspace file" : loaded ? "Saved · " + fileNewline.ToUpperInvariant() + " · UTF-8" + (fileBom ? " BOM" : "") : "Select a text file";
        FileSaveButton.Content = pendingFileSave != null ? "Retry save" : "Save file";
    }
    private void FileTextEdited(object sender, TextChangedEventArgs e)
    {
        if (fileLoading || FileDraftStatus == null || fileRevision.Length == 0) return;
        try { UpdateFileControls(); SaveFileDraft(); }
        catch (Exception error) when (error is IOException or UnauthorizedAccessException) { FilePreviewError.Text = SafeError(error); }
    }
    private void EditFileClicked(object sender, RoutedEventArgs e)
    {
        FileReviewPane.Visibility = Visibility.Collapsed;
        FileEditor.Visibility = FileEditor.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible;
        FilePreview.Visibility = FileEditor.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible;
        if (FilePreview.Visibility == Visibility.Visible) { FilePreview.SourceText = EditorText(); FilePreview.RenderNow(); }
    }
    private void ReviewFileClicked(object sender, RoutedEventArgs e) => ShowFileReview();
    private void ShowFileReview()
    {
        FileDiskText.Text = fileBaseline; FileDraftText.Text = EditorText();
        FileEditor.Visibility = FilePreview.Visibility = Visibility.Collapsed; FileReviewPane.Visibility = Visibility.Visible;
    }
    private async void SaveFileClicked(object sender, RoutedEventArgs e) => await Act(SaveWorkspaceFile);
    private async Task SaveWorkspaceFile()
    {
        if (client == null || fileSession == null || fileRevision.Length == 0 || fileSaving) return;
        pendingFileSave ??= new JsonObject { ["request_id"] = Json.Id(), ["path"] = previewPath, ["expected_revision"] = fileRevision, ["text"] = EditorText(), ["bom"] = fileBom, ["authorize_write"] = true };
        fileSaving = true; UpdateFileControls();
        try
        {
            SaveFileDraft(); // A failure to persist the immutable request prevents the project write.
            var result = await client.Post("/v1/bee/files/" + fileSession + "/save", (JsonObject)pendingFileSave.DeepClone(), lifetime.Token);
            fileRevision = Json.Text(result, "revision"); fileBaseline = Json.Text(pendingFileSave, "text"); previewText = fileBaseline;
            fileBaselineBom = fileBom;
            pendingFileSave = null; FilePreviewError.Text = "Saved. Core retained a private recovery copy of the previous contents.";
            FilePreviewTitle.Text = previewPath + " · " + (Encoding.UTF8.GetByteCount(fileBaseline) + (fileBom ? 3 : 0)) + " bytes";
            FilePreview.SourceText = fileBaseline;
            if (FileReviewPane.Visibility == Visibility.Visible) ShowFileReview();
            SaveFileDraft(); await RefreshFileHistory();
        }
        catch (CoreApiException error) when (error.StatusCode is >= 400 and < 500)
        {
            pendingFileSave = null; SaveFileDraft(); throw;
        }
        finally { fileSaving = false; UpdateFileControls(); }
    }
    private async void ReloadFileClicked(object sender, RoutedEventArgs e) => await Act(ReloadFileDisk);
    private async Task ReloadFileDisk()
    {
        if (client == null || fileSession == null || fileSaving) return;
        // Explicit reconciliation only; opening the app never retries a write.
        if (pendingFileSave != null)
        {
            try { await client.Get("/v1/bee/files/" + fileSession + "/saves/" + Json.Text(pendingFileSave, "request_id"), lifetime.Token); }
            catch (CoreApiException error) when (error.StatusCode == 404) { }
            pendingFileSave = null; SaveFileDraft();
        }
        string draft = EditorText(); bool draftBom = fileBom;
        var disk = await client.Get(FilesPath("preview", previewPath), lifetime.Token);
        LoadEditor(disk, false);
        fileLoading = true; FileEditor.Text = draft; fileBom = draftBom; fileLoading = false;
        UpdateFileControls(); SaveFileDraft(); ShowFileReview(); await RefreshFileHistory();
        FilePreviewError.Text = "Disk reloaded; your draft is retained. Compare both versions before saving. This establishes a new revision check.";
    }
    private async void DiscardFileClicked(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        if (pendingFileSave != null) throw new IOException("Retry the unconfirmed save or use Reload disk to check it before discarding.");
        LoadEditor(await client!.Get(FilesPath("preview", previewPath), lifetime.Token));
        FileReviewPane.Visibility = FileEditor.Visibility = Visibility.Collapsed; FilePreview.Visibility = Visibility.Visible;
        await RefreshFileHistory();
    });
    private async Task RefreshFileHistory()
    {
        if (client == null || fileSession == null || previewPath.Length == 0) return;
        var history = await client.Get(FilesPath("history", previewPath), lifetime.Token);
        FileRecoveryList.ItemsSource = Json.Array(history, "saves").Where(r => r?["before"] is JsonValue)
            .Select(r => new FileRecovery(DateTimeOffset.FromUnixTimeMilliseconds(Json.Number(r, "at")).ToLocalTime().ToString("g") + " · " + Json.Text(r, "state") + " · " + Json.Text(r, "request")[..Math.Min(8, Json.Text(r, "request").Length)], Json.Text(r, "before") is var before && before.StartsWith('\uFEFF') ? before[1..] : before, before.StartsWith('\uFEFF'))).ToList();
        FileRecoveryList.SelectedIndex = -1;
    }
    private async void LoadFileRecovery(object sender, RoutedEventArgs e) => await Act(LoadRecoveryDraft);
    private Task LoadRecoveryDraft()
    {
        RequireSavedFile();
        if (FileRecoveryList.SelectedItem is not FileRecovery recovery) throw new IOException("Select a recovery copy first.");
        fileBom = recovery.Bom; FileEditor.Text = recovery.Text; UpdateFileControls(); SaveFileDraft(); FileEditor.Visibility = Visibility.Visible; FilePreview.Visibility = FileReviewPane.Visibility = Visibility.Collapsed;
        FilePreviewError.Text = "Recovery copy loaded into the draft. Review and Save file to write it; disk has not changed.";
        return Task.CompletedTask;
    }
    private string FileDraftPath() => Path.ChangeExtension(OutboxPath(), ".files.json");
    private void SaveFileDraft()
    {
        if (client == null || fileLoading || fileIdentity != client.Identity || previewPath.Length == 0) return;
        var draft = new JsonObject { ["session"] = fileSession, ["path"] = previewPath, ["revision"] = fileRevision, ["baseline"] = fileBaseline,
            ["text"] = EditorText(), ["bom"] = fileBom, ["baseline_bom"] = fileBaselineBom, ["newline"] = fileNewline, ["language"] = fileLanguage, ["markdown"] = fileMarkdown, ["pending"] = pendingFileSave?.DeepClone() };
        string path = FileDraftPath(); File.WriteAllText(path + ".pending", draft.ToJsonString()); File.Move(path + ".pending", path, true);
    }
    private void RestoreFileDraft()
    {
        if (client == null || fileIdentity == client.Identity) return;
        fileLoading = true;
        try
        {
            fileIdentity = client.Identity; fileRevision = fileBaseline = previewPath = previewText = ""; fileBom = fileBaselineBom = false; FileEditor.Text = ""; pendingFileSave = null;
            if (!File.Exists(FileDraftPath())) return;
            var draft = JsonNode.Parse(File.ReadAllText(FileDraftPath())) ?? throw new IOException("Cannot read the file draft.");
            fileSession = Json.Text(draft, "session"); previewPath = Json.Text(draft, "path"); fileRevision = Json.Text(draft, "revision");
            fileBaseline = Json.Text(draft, "baseline"); fileNewline = Json.Text(draft, "newline", "lf"); fileLanguage = Json.Text(draft, "language", "text"); fileMarkdown = Json.Flag(draft, "markdown");
            fileBom = Json.Flag(draft, "bom"); fileBaselineBom = Json.Flag(draft, "baseline_bom");
            previewText = fileBaseline; FileEditor.Text = Json.Text(draft, "text"); pendingFileSave = draft["pending"]?.DeepClone() as JsonObject;
            FilePreviewTitle.Text = previewPath; FilePreview.CodeLanguage = fileMarkdown ? "" : fileLanguage; FilePreview.SourceText = EditorText(); FilePreview.RenderNow();
            FilePreviewError.Text = "Restored file draft. Reload disk to compare against current contents; nothing was written automatically.";
            FileEditor.Visibility = Visibility.Visible; FilePreview.Visibility = FileReviewPane.Visibility = Visibility.Collapsed;
        }
        finally { fileLoading = false; UpdateFileControls(); }
    }
    private async Task FileEditorSmokeTest(string output)
    {
        EditFileClicked(this, new RoutedEventArgs());
        string original = EditorText();
        FileEditor.Text += "\n# Native edit — 蜂\n";
        string edited = EditorText();
        try { await SaveWorkspaceFile(); throw new IOException("The fixture did not drop the first save receipt."); }
        catch (Exception error) when (error is HttpRequestException or IOException && error is not CoreApiException)
        {
            if (pendingFileSave == null) throw;
        }
        string pending = pendingFileSave!.ToJsonString();
        fileIdentity = null; RestoreFileDraft();
        if (pendingFileSave?.ToJsonString() != pending || !FileEditor.IsReadOnly || EditorText() != edited) throw new IOException("Pending file save changed during draft recovery.");
        await SaveWorkspaceFile();
        if (FileDirty || pendingFileSave != null || Json.Text(await client!.Get(FilesPath("preview", previewPath)), "text") != edited) throw new IOException("Lost file receipt retry did not complete.");
        FileEditor.Text = edited + "# Draft after external change\n";
        string draft = EditorText();
        string external = "# External editor version" + (fileNewline == "crlf" ? "\r\n" : "\n");
        await client.Post("/v1/bee/files/" + fileSession + "/save", new JsonObject { ["request_id"] = Json.Id(), ["path"] = previewPath, ["expected_revision"] = fileRevision, ["text"] = external, ["authorize_write"] = true });
        try { await SaveWorkspaceFile(); throw new IOException("The stale editor revision overwrote an external change."); }
        catch (CoreApiException error) when (error.StatusCode == 409) { }
        if (EditorText() != draft) throw new IOException("A file conflict lost the draft.");
        await ReloadFileDisk();
        if (EditorText() != draft || Lf(FileDiskText.Text) != Lf(external) || FileReviewPane.Visibility != Visibility.Visible) throw new IOException("Conflict review lost a version.");
        await Capture(Path.Combine(output, "cloud-bee-editor.png"));
        await SaveWorkspaceFile();
        var recovery = FileRecoveryList.Items.OfType<FileRecovery>().First(r => r.Text == original);
        FileRecoveryList.SelectedItem = recovery; await LoadRecoveryDraft();
        if (EditorText() != original || Json.Text(await client.Get(FilesPath("preview", previewPath)), "text") != draft) throw new IOException("Loading recovery contents wrote disk automatically.");
        fileIdentity = null; RestoreFileDraft();
        if (EditorText() != original || !FileDirty) throw new IOException("Unsaved file draft was not restored.");
        try { await PreviewWorkspaceFile("preview.md"); throw new IOException("Opening another file discarded a draft."); }
        catch (IOException) when (FileDirty) { }
        await SaveWorkspaceFile();
        if (Json.Text(await client.Get(FilesPath("preview", previewPath)), "text") != original) throw new IOException("Reviewed recovery draft was not saved.");
        await File.WriteAllTextAsync(Path.Combine(output, "editor-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "native editing, immutable lost-receipt retry, disk conflict and comparison, recovery drafts, reconnect persistence, guarded file switching" }.ToJsonString());
    }
}
