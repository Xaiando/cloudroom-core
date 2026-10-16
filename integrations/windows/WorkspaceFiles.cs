using System.Diagnostics;
using System.IO;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;

namespace Cloudroom.Desktop;

public sealed record WorkspaceFileItem(string Name, bool Directory, bool Link)
{
    public string Label => (Directory ? "▣  " : "▤  ") + Name + (Link ? "  ↗" : "");
}
public partial class MainWindow
{
    private string? fileSession;
    private string fileDirectory = "", previewPath = "", previewText = "";
    private bool filesBusy;
    private long fileVersion;

    private async void ShowWorkspaceFiles(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        RestoreFileDraft();
        FilePreviewDialog.Visibility = Visibility.Visible;
        if (FileDirty || pendingFileSave != null)
        {
            await ListWorkspaceFiles(previewPath.Contains('/') ? previewPath[..previewPath.LastIndexOf('/')] : "");
            await RefreshFileHistory(); return;
        }
        fileSession = sessionId ?? throw new IOException("Select a conversation first.");
        FilePreviewDialog.Visibility = Visibility.Visible; FilePreviewError.Text = "";
        previewText = previewPath = ""; FilePreview.CodeLanguage = ""; FilePreview.SourceText = "Select a UTF-8 text file to preview. Files stay inside this conversation's workspace.";
        FilePreviewTitle.Text = "Workspace files";
        fileLoading = true; FileEditor.Text = ""; fileRevision = fileBaseline = ""; fileBom = fileBaselineBom = false; fileLoading = false;
        FileEditor.Visibility = FileReviewPane.Visibility = Visibility.Collapsed; FilePreview.Visibility = Visibility.Visible; UpdateFileControls();
        await ListWorkspaceFiles("");
    });
    private string FilesPath(string action, string path) => "/v1/bee/files/" + fileSession + "/" + action + "?path=" + Uri.EscapeDataString(path);
    private async Task ListWorkspaceFiles(string path)
    {
        if (client == null || fileSession == null) return;
        long version = ++fileVersion;
        var result = await client.Get(FilesPath("list", path), lifetime.Token);
        if (version != fileVersion) return;
        fileDirectory = Json.Text(result, "path"); FileDirectory.Text = fileDirectory.Length == 0 ? "Workspace root" : fileDirectory;
        WorkspaceFileList.ItemsSource = Json.Array(result, "entries").Select(e => new WorkspaceFileItem(Json.Text(e, "name"), Json.Flag(e, "directory"), Json.Flag(e, "link"))).ToList();
        FilePreviewError.Text = Json.Flag(result, "truncated") ? "Showing the first 500 entries. Open a subfolder to browse further." : "";
    }
    private async void WorkspaceFileSelected(object sender, SelectionChangedEventArgs e)
    {
        if (WorkspaceFileList.SelectedItem is not WorkspaceFileItem item || filesBusy) return;
        await Act(async () =>
        {
            filesBusy = true;
            try
            {
                string path = fileDirectory.Length == 0 ? item.Name : fileDirectory + "/" + item.Name;
                if (item.Directory) await ListWorkspaceFiles(path);
                else await PreviewWorkspaceFile(path);
            }
            catch (Exception error) { FilePreviewError.Text = SafeError(error); throw; }
            finally { filesBusy = false; }
        });
    }
    private async Task PreviewWorkspaceFile(string path, int line = 0)
    {
        if (client == null || fileSession == null) return;
        RequireSavedFile();
        long version = ++fileVersion;
        var result = await client.Get(FilesPath("preview", path), lifetime.Token);
        if (version != fileVersion) return;
        LoadEditor(result);
        FileEditor.Visibility = FileReviewPane.Visibility = Visibility.Collapsed; FilePreview.Visibility = Visibility.Visible;
        await RefreshFileHistory();
        FilePreviewError.Text = previewText.Length > 96000 ? "The rendered preview is limited to 96,000 characters. Copy source retains the complete downloaded text (up to 256 KiB)." : "";
        if (line > 0)
        {
            FilePreviewError.Text = FilePreview.SelectSourceLine(line) ? "Requested line " + line + " · plain text view" : "Line " + line + " is outside the rendered preview.";
        }
    }
    private async void FileParent(object sender, RoutedEventArgs e) => await Act(() => ListWorkspaceFiles(fileDirectory.Contains('/') ? fileDirectory[..fileDirectory.LastIndexOf('/')] : ""));
    private async void RefreshWorkspaceFiles(object sender, RoutedEventArgs e) => await Act(() => ListWorkspaceFiles(fileDirectory));
    private void DismissFilePreview(object sender, RoutedEventArgs e) { SaveFileDraft(); ++fileVersion; FilePreviewDialog.Visibility = Visibility.Collapsed; }
    private async void CopyPreviewSource(object sender, RoutedEventArgs e) => await Act(() => { Clipboard.SetText(fileRevision.Length > 0 ? EditorText() : previewText); return Task.CompletedTask; });
    private async void CopyMessageSource(object sender, RoutedEventArgs e) => await Act(() => { Clipboard.SetText(((Button)sender).Tag as string ?? ""); return Task.CompletedTask; });
    private async void MarkdownLinkActivated(object sender, RoutedEventArgs e)
    {
        if (e is not MarkdownLinkEventArgs link) return;
        e.Handled = true;
        bool fromPreview = ReferenceEquals(e.OriginalSource, FilePreview) || ReferenceEquals(e.Source, FilePreview);
        await Act(async () =>
        {
            try
            {
                if (Uri.TryCreate(link.Target, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https")
                {
                    if (!MarkdownView.AllowedLink(link.Target)) throw new IOException("This link type is unavailable.");
                    Process.Start(new ProcessStartInfo(uri.AbsoluteUri) { UseShellExecute = true }); return;
                }
                string context = (e.OriginalSource as MarkdownView)?.ContextSession ?? "";
                RestoreFileDraft(); RequireSavedFile();
                fileSession = fromPreview ? fileSession : context.Length > 0 ? context : sessionId;
                if (fileSession == null || client == null) throw new IOException("Select a conversation to open a workspace file.");
                string root;
                if (fileSession.StartsWith("council_", StringComparison.Ordinal)) root = (await client.Get("/v1/bee/councils/" + fileSession, lifetime.Token))["council"]?["workspace"]?.GetValue<string>() ?? "";
                else { var state = (await client.Get(SessionPath(fileSession), lifetime.Token))["session"]; root = Json.Text(state?["workspace"], "path"); }
                string parent = fromPreview && previewPath.Contains('/') ? previewPath[..previewPath.LastIndexOf('/')] : "";
                var location = ResolveFileLink(root, parent, link.Target);
                FilePreviewDialog.Visibility = Visibility.Visible; FilePreviewError.Text = "";
                await ListWorkspaceFiles(location.Path.Contains('/') ? location.Path[..location.Path.LastIndexOf('/')] : "");
                await PreviewWorkspaceFile(location.Path, location.Line);
                if (location.Anchor.Length > 0 && !FilePreview.ScrollToAnchor(location.Anchor)) FilePreviewError.Text = "Heading anchor was not found in this preview.";
            }
            catch (Exception error) { FilePreviewError.Text = SafeError(error); throw; }
        });
    }
    private static string OrdinaryPath(string path) => path.StartsWith("\\\\?\\UNC\\", StringComparison.OrdinalIgnoreCase) ? "\\\\" + path[8..] : path.StartsWith("\\\\?\\", StringComparison.Ordinal) ? path[4..] : path;
    public static (string Path, int Line, string Anchor) ResolveFileLink(string root, string parent, string target)
    {
        if (!MarkdownView.AllowedLink(target) || root.Length == 0) throw new IOException("This file link is unavailable.");
        int line = 0;
        string heading = "";
        var suffix = Regex.Match(target, @"(?:(?:#L|:)(\d+)(?::\d+)?)$");
        if (suffix.Success) { if (!int.TryParse(suffix.Groups[1].Value, out line) || line < 1) throw new IOException("Invalid source line number."); target = target[..suffix.Index]; }
        else if (target.IndexOf('#') is int fragment && fragment >= 0) { heading = target[(fragment + 1)..]; target = target[..fragment]; }
        if (line > 100000) throw new IOException("Line number exceeds preview limits.");
        if (Uri.TryCreate(target, UriKind.Absolute, out var uri) && uri.IsFile) target = uri.LocalPath;
        else target = Uri.UnescapeDataString(target);
        target = target.Replace('/', '\\');
        string anchor = Path.GetFullPath(OrdinaryPath(root)).TrimEnd('\\') + "\\";
        target = OrdinaryPath(target);
        string path = Path.GetFullPath(Path.IsPathRooted(target) ? target : Path.Combine(anchor, parent.Replace('/', '\\'), target));
        if (!path.StartsWith(anchor, StringComparison.OrdinalIgnoreCase)) throw new IOException("File links must stay inside this conversation's workspace.");
        string relative = path[anchor.Length..].Replace('\\', '/');
        if (relative.Length == 0 || relative.Contains(':') || relative.Split('/').Any(p => p.EndsWith(' ') || p.EndsWith('.'))) throw new IOException("This file path is unavailable.");
        return (relative, line, heading);
    }
    private async Task PresentationSmokeTest(string output)
    {
        foreach (string target in new[] { "javascript:alert(1)", "data:text/html,hello", "shell:AppsFolder", "\\\\host\\share\\file", "https://name:secret@example.com" })
            if (MarkdownView.AllowedLink(target)) throw new IOException("An unsafe Markdown link was enabled.");
        string root = "C:\\fixture\\workspace";
        if (ResolveFileLink(root, "docs", "../src/main.rs#L12") != ("src/main.rs", 12, "")) throw new IOException("Workspace-relative link resolution failed.");
        if (ResolveFileLink("\\\\?\\" + root, "", root + "\\main.rs#L2") != ("main.rs", 2, "")) throw new IOException("Absolute link did not match the canonical Windows workspace.");
        foreach (string target in new[] { "../../private/key", "C:/outside/key", "%2e%2e/private", "file://host/share/key" })
        {
            try { ResolveFileLink(root, "", target); throw new InvalidOperationException("Outside file link accepted."); }
            catch (IOException) { }
        }
        fileSession = sessionId ?? throw new IOException("Presentation smoke needs a conversation.");
        if (CouncilAnswers.Items.OfType<ChatMessage>().Any(m => m.SourceSession.Length == 0)) throw new IOException("Council file links lost their model's workspace identity.");
        FilePreviewDialog.Visibility = Visibility.Visible;
        await ListWorkspaceFiles("");
        await PreviewWorkspaceFile("preview.md");
        string rendered = new TextRange(FilePreview.Document.ContentStart, FilePreview.Document.ContentEnd).Text;
        if (!rendered.Contains("Native preview") || !rendered.Contains("Unicode ø")) throw new IOException("Markdown preview did not render its content.");
        if (FilePreview.Document.Blocks.OfType<System.Windows.Documents.Table>().Count() != 1) throw new IOException("Markdown table did not render.");
        if (!FilePreview.ScrollToAnchor("native-preview")) throw new IOException("Markdown heading anchors were not registered.");
        await Capture(Path.Combine(output, "cloud-bee-markdown.png"));
        Hyperlink? sourceLink = null;
        for (var pointer = FilePreview.Document.ContentStart; pointer != null; pointer = pointer.GetNextContextPosition(LogicalDirection.Forward))
            if (pointer.GetAdjacentElement(LogicalDirection.Forward) is Hyperlink hyperlink && hyperlink.ToolTip as string == "source.py#L2") { sourceLink = hyperlink; break; }
        if (sourceLink == null) throw new IOException("Native Markdown did not create the workspace source link.");
        sourceLink.RaiseEvent(new RoutedEventArgs(Hyperlink.ClickEvent));
        for (int i = 0; i < 100 && (acting || previewPath != "source.py"); i++) await Task.Delay(50);
        if (previewPath != "source.py" || !FilePreview.Selection.Start.GetTextInRun(LogicalDirection.Forward).StartsWith("    print", StringComparison.Ordinal)) throw new IOException("Clicking a file link did not select its source line.");
        if (!new TextRange(FilePreview.Document.ContentStart, FilePreview.Document.ContentEnd).Text.Contains("print")) throw new IOException("Code preview lost source text.");
        await PreviewWorkspaceFile("source.py");
        await Capture(Path.Combine(output, "cloud-bee-files.png"));
        await FileEditorSmokeTest(output);
        DismissFilePreview(this, new RoutedEventArgs());
        await File.WriteAllTextAsync(Path.Combine(output, "presentation-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "native Markdown/table/code rendering, safe links, workspace file browser and previews" }.ToJsonString());
    }
}
