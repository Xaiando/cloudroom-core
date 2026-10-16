using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;

namespace Cloudroom.Desktop;

public partial class MainWindow
{
    private JsonObject threadOrganization = new();
    private List<SessionItem> allSessionItems = [];
    private readonly Dictionary<string, string> nativeOwners = [];
    private string? organizingThread;
    private string organizationRequest = "";
    private long organizationRevision;
    private long listVersion;
    private void ApplyThreadFilter()
    {
        if (SessionList == null || ThreadSearch == null || ThreadArchived == null) return;
        string search = ThreadSearch.Text.Trim();
        var items = allSessionItems.Where(i => Json.Flag(threadOrganization[i.Id], "archived") == (ThreadArchived.IsChecked == true))
            .Where(i => search.Length == 0 || (i.Title + "\n" + i.Model + "\n" + i.Folder + "\n" + Json.Text(threadOrganization[i.Id], "collection")).Contains(search, StringComparison.OrdinalIgnoreCase))
            .OrderByDescending(i => Json.Flag(threadOrganization[i.Id], "pinned")).ThenBy(i => Json.Text(threadOrganization[i.Id], "collection"), StringComparer.OrdinalIgnoreCase).ToList();
        refreshingList = true;
        SessionList.ItemsSource = items;
        SessionList.SelectedItem = items.FirstOrDefault(i => i.Id == sessionId);
        SessionCount.Text = items.Count + " / " + allSessionItems.Count;
        refreshingList = false;
    }
    private void ThreadSearchChanged(object sender, TextChangedEventArgs e) => ApplyThreadFilter();
    private void ThreadFilterChanged(object sender, RoutedEventArgs e) => ApplyThreadFilter();
    private async void ShowThreadOrganization(object sender, RoutedEventArgs e) => await Act(() =>
    {
        OpenThreadOrganization(sessionId ?? throw new IOException("Select a conversation first."));
        return Task.CompletedTask;
    });
    private void OpenThreadOrganization(string id)
    {
        id = nativeOwners.GetValueOrDefault(id, id);
        organizingThread = id;
        string path = "/v1/bee/threads/" + id + "/organize";
        extensionOutbox.TryGetValue(path, out var pending);
        var metadata = pending ?? threadOrganization[id];
        organizationRevision = Json.Number(metadata, pending == null ? "revision" : "expected_revision");
        organizationRequest = pending == null ? Json.Id() : Json.Text(pending, "request_id");
        ThreadTitle.Text = Json.Text(metadata, "title", titles.GetValueOrDefault(id, ""));
        ThreadCollection.Text = Json.Text(metadata, "collection");
        ThreadPinned.IsChecked = Json.Flag(metadata, "pinned"); ThreadArchiveValue.IsChecked = Json.Flag(metadata, "archived");
        ThreadOrganizationError.Text = pending == null ? "" : "An unconfirmed save is restored. Save retries that request.";
        ThreadOrganizationDialog.Visibility = Visibility.Visible;
        ThreadTitle.Focus();
    }
    private void DismissThreadOrganization(object sender, RoutedEventArgs e) => ThreadOrganizationDialog.Visibility = Visibility.Collapsed;
    private async void SaveThreadOrganization(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        try { await SaveOrganization(); }
        catch (Exception error) { ThreadOrganizationError.Text = SafeError(error); throw; }
    });
    private async Task SaveOrganization()
    {
        if (organizingThread == null) return;
        string path = "/v1/bee/threads/" + organizingThread + "/organize";
        var body = new JsonObject { ["request_id"] = organizationRequest, ["expected_revision"] = organizationRevision, ["title"] = ThreadTitle.Text, ["collection"] = ThreadCollection.Text, ["pinned"] = ThreadPinned.IsChecked == true, ["archived"] = ThreadArchiveValue.IsChecked == true };
        if (extensionOutbox.TryGetValue(path, out var pending) && pending.ToJsonString() != body.ToJsonString()) throw new IOException("An earlier save has no receipt. Close and reopen Organize selected to recover and retry it unchanged.");
        ThreadOrganizationDialog.IsEnabled = false;
        try
        {
            await ExtensionPost(path, body);
            if (ThreadTitle.Text.Trim().Length == 0) titles.Remove(organizingThread);
            await RefreshSessions();
            ThreadOrganizationDialog.Visibility = Visibility.Collapsed;
            if (sessionId != null && nativeOwners.GetValueOrDefault(sessionId, sessionId) == organizingThread) HeaderTitle.Text = titles.GetValueOrDefault(organizingThread, "Conversation");
        }
        catch (CoreApiException) { organizationRequest = Json.Id(); throw; }
        finally { ThreadOrganizationDialog.IsEnabled = true; }
    }
    private async Task OrganizationSmokeTest(string output)
    {
        string id = sessionId ?? throw new IOException("Organization smoke needs a saved conversation.");
        id = nativeOwners.GetValueOrDefault(id, id);
        SelectSession(id); await Poll();
        OpenThreadOrganization(id); ThreadTitle.Text = "Windows parity work"; ThreadCollection.Text = "Cloud Bee"; ThreadPinned.IsChecked = true;
        await SaveOrganization();
        if (!SessionList.Items.OfType<SessionItem>().Any(i => i.Id == id && i.Title == "★ Windows parity work")) throw new IOException("Saved thread name or pin did not render.");
        ThreadSearch.Text = "missing search result";
        if (SessionList.Items.Count != 0) throw new IOException("Thread search did not filter.");
        ThreadSearch.Text = "Cloud Bee";
        if (!SessionList.Items.OfType<SessionItem>().Any(i => i.Id == id)) throw new IOException("Thread collection was not searchable.");
        ThreadSearch.Text = "";
        OpenThreadOrganization(id); ThreadArchiveValue.IsChecked = true; await SaveOrganization();
        if (SessionList.Items.OfType<SessionItem>().Any(i => i.Id == id)) throw new IOException("Archived thread stayed in active list.");
        ThreadArchived.IsChecked = true; ApplyThreadFilter();
        if (!SessionList.Items.OfType<SessionItem>().Any(i => i.Id == id)) throw new IOException("Archived thread was not recoverable.");
        OpenThreadOrganization(id); ThreadArchiveValue.IsChecked = false; await SaveOrganization();
        ThreadArchived.IsChecked = false; ApplyThreadFilter();
        await Connect();
        if (Json.Text(threadOrganization[id], "title") != "Windows parity work") throw new IOException("Thread title did not survive reconnect.");
        await Capture(Path.Combine(output, "cloud-bee-organization.png"));
        await File.WriteAllTextAsync(Path.Combine(output, "organization-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "saved rename/pin/collections, sidebar search, archive/restore and reconnect" }.ToJsonString());
    }
}
