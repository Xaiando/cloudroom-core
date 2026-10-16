using System.IO;
using System.Text.Json.Nodes;
using System.Windows;
using System.Windows.Controls;
using Microsoft.Win32;

namespace Cloudroom.Desktop;

public sealed record ProjectItem(string Id, string Name, string Color, string Folder, bool Archived, int Count)
{
    public string Label => Name + (Archived ? " · archived" : "");
    public string CountLabel => Count.ToString();
    public string Brush => Color switch { "blue" => "#8BB8FF", "green" => "#75D3AD", "amber" => "#E0AA63", "rose" => "#EE9DB6", _ => "#B6A9FF" };
    public Visibility Actions => Id.Length > 0 && Id != "unassigned" ? Visibility.Visible : Visibility.Collapsed;
}

public partial class MainWindow
{
    private JsonObject projects = new(), projectThreads = new();
    private string projectFilter = "", editingProject = "", projectRequest = "";
    private long projectRevision;
    private bool refreshingProjects;
    private string ActiveThreadId => CouncilWorkspacePanel.Visibility == Visibility.Visible && councilId != null ? councilId : sessionId ?? "";
    private string ThreadProject(string id) => projectThreads[id]?.GetValue<string>() ?? "";
    private static bool IsCouncil(string? id) => id?.StartsWith("council_", StringComparison.Ordinal) == true;
    private string NewChatProjectId => (NewChatProject.SelectedItem as ProjectItem)?.Id ?? "";
    private List<ProjectItem> ProjectChoices(bool includeArchived = false) => projects.Select(p => new ProjectItem(p.Key,
        Json.Text(p.Value, "name"), Json.Text(p.Value, "color"), Json.Text(p.Value, "folder"), Json.Flag(p.Value, "archived"),
        allSessionItems.Count(t => ThreadProject(t.Id) == p.Key)))
        .Where(p => includeArchived || !p.Archived).OrderBy(p => p.Name, StringComparer.OrdinalIgnoreCase).ToList();
    private void ApplyProjects(JsonNode state)
    {
        projects = state["projects"] as JsonObject ?? new(); projectThreads = state["threads"] as JsonObject ?? new();
        refreshingProjects = true;
        var choices = ProjectChoices(ShowArchivedProjects.IsChecked == true);
        choices.Insert(0, new("unassigned", "Unassigned", "blue", "", false, allSessionItems.Count(t => ThreadProject(t.Id).Length == 0)));
        choices.Insert(0, new("", "All chats", "lavender", "", false, allSessionItems.Count));
        if (!choices.Any(p => p.Id == projectFilter)) projectFilter = "";
        ProjectList.ItemsSource = choices; ProjectList.SelectedItem = choices.First(p => p.Id == projectFilter);
        ProjectChatHeading.Text = projectFilter.Length == 0 ? "CHATS" : projectFilter == "unassigned" ? "UNASSIGNED" : Json.Text(projects[projectFilter], "name").ToUpperInvariant();
        refreshingProjects = false;
    }
    private void ProjectSelected(object sender, SelectionChangedEventArgs e)
    {
        if (refreshingProjects || ProjectList.SelectedItem is not ProjectItem item) return;
        projectFilter = item.Id; ProjectChatHeading.Text = item.Name.ToUpperInvariant(); ApplyThreadFilter();
        SaveOutbox();
    }
    private void ArchivedProjectsChanged(object sender, RoutedEventArgs e) { ApplyProjects(new JsonObject { ["projects"] = projects.DeepClone(), ["threads"] = projectThreads.DeepClone() }); ApplyThreadFilter(); }
    private void NewProjectClicked(object sender, RoutedEventArgs e)
    {
        string? pending = extensionOutbox.Keys.FirstOrDefault(p => p.StartsWith("/v1/bee/projects/", StringComparison.Ordinal) && !p.Contains("/threads/", StringComparison.Ordinal));
        OpenProjectSettings(pending?.Split('/').Last());
    }
    private void EditProjectClicked(object sender, RoutedEventArgs e) => OpenProjectSettings(((Button)sender).DataContext is ProjectItem item ? item.Id : projectFilter);
    private void OpenProjectSettings(string? id)
    {
        editingProject = id is { Length: > 0 } && (projects.ContainsKey(id) || extensionOutbox.ContainsKey("/v1/bee/projects/" + id)) ? id : "project_" + Json.Id();
        extensionOutbox.TryGetValue("/v1/bee/projects/" + editingProject, out var pending);
        var value = pending ?? projects[editingProject]; projectRevision = Json.Number(value, pending == null ? "revision" : "expected_revision"); projectRequest = pending == null ? Json.Id() : Json.Text(pending, "request_id");
        ProjectEditorTitle.Text = value == null ? "New project" : "Project settings";
        ProjectName.Text = Json.Text(value, "name"); ProjectDescription.Text = Json.Text(value, "description");
        ProjectFolder.Text = Json.Text(value, "folder"); ProjectArchived.IsChecked = Json.Flag(value, "archived");
        foreach (var color in ProjectColor.Items.OfType<ComboBoxItem>()) if ((string)color.Tag == Json.Text(value, "color", "lavender")) ProjectColor.SelectedItem = color;
        ProjectEditorError.Text = pending == null ? "" : "Unconfirmed save restored. Save retries it unchanged."; ProjectDialog.Visibility = Visibility.Visible; ProjectName.Focus();
    }
    private void BrowseProjectFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new OpenFolderDialog { Title = "Choose this project's local workspace", Multiselect = false };
        if (dialog.ShowDialog(this) == true) ProjectFolder.Text = dialog.FolderName;
    }
    private void DefaultProjectFolder(object sender, RoutedEventArgs e) => ProjectFolder.Clear();
    private void DismissProjectEditor(object sender, RoutedEventArgs e) => ProjectDialog.Visibility = Visibility.Collapsed;
    private async void SaveProjectClicked(object sender, RoutedEventArgs e) => await Act(async () =>
    {
        try { await SaveProject(); } catch (Exception error) { ProjectEditorError.Text = SafeError(error); throw; }
    });
    private async Task SaveProject()
    {
        string route = "/v1/bee/projects/" + editingProject;
        var body = new JsonObject { ["request_id"] = projectRequest, ["expected_revision"] = projectRevision,
            ["name"] = ProjectName.Text, ["description"] = ProjectDescription.Text, ["color"] = ComboTag(ProjectColor),
            ["folder"] = ProjectFolder.Text.Length > 0 ? ProjectFolder.Text : null, ["archived"] = ProjectArchived.IsChecked == true };
        if (extensionOutbox.TryGetValue(route, out var prior) && prior.ToJsonString() != body.ToJsonString()) throw new IOException("Retry the unconfirmed project save unchanged before editing it.");
        try { await ExtensionPost(route, body); }
        catch (CoreApiException) { projectRequest = Json.Id(); throw; }
        projectFilter = body["archived"]!.GetValue<bool>() ? "" : editingProject;
        ProjectDialog.Visibility = Visibility.Collapsed; await RefreshSessions(); SaveOutbox();
    }
    private void PrepareNewChat(string? project = null, bool? council = null)
    {
        var choices = ProjectChoices(); choices.Insert(0, new("", "Unassigned · Core workspace", "lavender", "", false, 0));
        NewChatProject.ItemsSource = choices;
        NewChatProject.SelectedItem = choices.FirstOrDefault(p => p.Id == (project ?? projectFilter)) ?? choices[0];
        bool councilDefault = CouncilWorkspacePanel.Visibility == Visibility.Visible || !ChatAvailable(HarnessPicker.SelectedItem as string ?? "codex", ModelPicker.SelectedItem as string ?? "");
        NewThreadKind.SelectedIndex = (council ?? councilDefault) ? 0 : 1;
        if (NewChatProjectId.Length > 0) { projectFilter = NewChatProjectId; ApplyProjects(new JsonObject { ["projects"] = projects.DeepClone(), ["threads"] = projectThreads.DeepClone() }); ApplyThreadFilter(); }
        NewThreadTitle.Text = ""; NewSessionError.Text = ""; NewWorkspaceCheck.IsChecked = false; LegacyWorkCheck.IsChecked = false;
        NewDialog.Visibility = Visibility.Visible; UpdateNewChatOptions();
    }
    private void ProjectNewChatClicked(object sender, RoutedEventArgs e) => PrepareNewChat(((Button)sender).DataContext is ProjectItem item ? item.Id : projectFilter);
    private void NewChatOptionsChanged(object sender, SelectionChangedEventArgs e) => UpdateNewChatOptions();
    private void UpdateNewChatOptions()
    {
        if (NewChatWorkspaceHint == null || NewChatProject == null || NewThreadKind == null) return;
        string project = NewChatProjectId;
        bool council = NewThreadKind.SelectedIndex == 0;
        NewAgentOptions.Visibility = council ? Visibility.Collapsed : Visibility.Visible;
        NewStandaloneFolder.Visibility = project.Length == 0 && !council ? Visibility.Visible : Visibility.Collapsed;
        if (project.Length > 0 || council) NewWorkspaceCheck.IsChecked = false;
        NewChatWorkspaceHint.Text = project.Length == 0 ? "Uses Core's workspace. Each chat keeps its own history."
            : "Project workspace: " + (Json.Text(projects[project], "folder") is { Length: > 0 } path ? path : "Core workspace") + "\nWork and folder access are authorized separately.";
        if (CreateButton != null) CreateButton.Content = council ? "Create Council chat" : "Create chat";
    }
    private async Task AttachCreatedThread(string id, JsonObject start)
    {
        if (client == null) return;
        string project = Json.Text(start, "desktop_project"), request = Json.Text(start, "request_id");
        if (project.Length > 0) await client.Post("/v1/bee/projects/threads/" + id, new JsonObject { ["request_id"] = request + "_project", ["project_id"] = project, ["expected_project"] = null }, lifetime.Token);
        string title = Json.Text(start, "desktop_title");
        if (title.Length > 0) await client.Post("/v1/bee/threads/" + id + "/organize", new JsonObject { ["request_id"] = request + "_title", ["expected_revision"] = 0, ["title"] = title, ["collection"] = "", ["pinned"] = false, ["archived"] = false }, lifetime.Token);
    }
    private static JsonObject StartPayload(JsonObject start)
    {
        var body = (JsonObject)start.DeepClone(); body.Remove("desktop_project"); body.Remove("desktop_title"); return body;
    }
    private async Task CreateCouncilChat()
    {
        if (client == null) throw new IOException("Connect to Core first.");
        if (pendingCouncilMessage != null) throw new IOException("Retry the unconfirmed Council message before creating another chat.");
        if (pendingCouncilStart == null)
        {
            string project = NewChatProjectId, title = NewThreadTitle.Text.Trim();
            await OpenCouncilWorkspace(); ResetCouncilDraft();
            CouncilFolder.Text = Json.Text(projects[project], "folder"); CouncilAccess.SelectedIndex = 0;
            pendingCouncilStart = new JsonObject { ["request_id"] = Json.Id(), ["title"] = title.Length > 0 ? title : "New chat",
                ["members"] = SelectedCouncilMembers(), ["workspace"] = CouncilFolder.Text.Length > 0 ? CouncilFolder.Text : null,
                ["access"] = "discuss", ["authorize_access"] = false, ["desktop_project"] = project };
            SaveCouncilDraft();
        }
        await ConfirmCouncilStart(); NewDialog.Visibility = Visibility.Collapsed;
        await RefreshSessions(); CouncilWorkspacePanel.Visibility = Visibility.Visible; await RefreshCouncilWorkspace(true); SaveOutbox();
    }
    private async Task ConfirmCouncilStart()
    {
        if (client == null || pendingCouncilStart == null) return;
        councilId = Json.Text(await client.Post("/v1/bee/councils", StartPayload(pendingCouncilStart), lifetime.Token), "council_id");
        await AttachCreatedThread(councilId, pendingCouncilStart);
        pendingCouncilStart = null; SaveCouncilDraft();
    }
    private async Task SelectCouncilThread(string id)
    {
        if (pendingCouncilStart != null || pendingCouncilMessage != null) throw new IOException("Retry the unconfirmed Council request before switching chats.");
        await OpenCouncilWorkspace(); SaveCouncilDraft(); councilId = id;
        CouncilComposer.Text = councilDrafts.GetValueOrDefault(id, ""); councilRendered = ""; councilSelectionDirty = false;
        await RefreshCouncilWorkspace(true); SaveCouncilDraft(); ApplyThreadFilter();
    }
    private void PopulateThreadProject(string id)
    {
        var choices = ProjectChoices(true); choices.Insert(0, new("", "Unassigned", "lavender", "", false, 0));
        ThreadProjectPicker.ItemsSource = choices; ThreadProjectPicker.SelectedItem = choices.FirstOrDefault(p => p.Id == ThreadProject(id)) ?? choices[0];
    }
    private async Task MoveOrganizedThread(string id)
    {
        string target = (ThreadProjectPicker.SelectedItem as ProjectItem)?.Id ?? "", previous = ThreadProject(id);
        if (target == previous && !extensionOutbox.ContainsKey("/v1/bee/projects/threads/" + id)) return;
        await ExtensionPost("/v1/bee/projects/threads/" + id, new JsonObject { ["request_id"] = organizationRequest + "_project",
            ["project_id"] = target.Length > 0 ? target : null, ["expected_project"] = previous.Length > 0 ? previous : null });
    }
    private async Task ProjectsSmokeTest(string output)
    {
        if (client == null) throw new IOException("Project smoke requires Core.");
        string workspace = Path.Combine(Path.GetDirectoryName(output)!, "project-workspace"); Directory.CreateDirectory(workspace);
        OpenProjectSettings(null); ProjectName.Text = "Cloud Bee project"; ProjectDescription.Text = "Separate research and implementation threads";
        ProjectFolder.Text = workspace; ProjectColor.SelectedIndex = 1; await SaveProject(); string project = editingProject;
        var ids = new List<string>();
        foreach (string title in new[] { "Design discussion", "Implementation plan" })
        {
            PrepareNewChat(project, false); HarnessPicker.SelectedItem = "codex"; ApplyHarness("codex"); ModelPicker.SelectedItem = "fixture"; NewThreadTitle.Text = title;
            ids.Add(await CreateSession());
        }
        if (ids.Distinct().Count() != 2 || ids.Any(id => ThreadProject(id) != project)) throw new IOException("Project chats were not separate or assigned.");
        SelectSession(ids[0]); await Poll(); Composer.Text = "Design draft retained"; SaveOutbox();
        SelectSession(ids[1]); await Poll(); if (Composer.Text.Length != 0) throw new IOException("Chat drafts leaked between threads.");
        SelectSession(ids[0]); await Poll(); if (Composer.Text != "Design draft retained") throw new IOException("Project draft was lost.");
        PrepareNewChat(project, true); NewThreadTitle.Text = "Architecture Council"; await CreateCouncilChat(); string council = councilId!;
        if (ThreadProject(council) != project || SessionList.Items.Count != 3) throw new IOException("Project Council did not appear alongside its chats.");
        await SelectCouncilThread(council);
        OpenThreadOrganization(ids[0]); ThreadProjectPicker.SelectedIndex = 0; await SaveOrganization();
        if (ThreadProject(ids[0]).Length > 0 || SessionList.Items.Count != 2) throw new IOException("Moving a chat did not update project filtering.");
        OpenProjectSettings(project); ProjectName.Text = "Cloud Bee workspace"; ProjectArchived.IsChecked = true; await SaveProject();
        if (ProjectChoices().Any(p => p.Id == project)) throw new IOException("Archived project stayed visible.");
        OpenProjectSettings(project); ProjectArchived.IsChecked = false; await SaveProject();
        await Connect(); await OpenCouncilWorkspace(); await SelectCouncilThread(council);
        if (Json.Text(projects[project], "name") != "Cloud Bee workspace" || ThreadProject(ids[1]) != project) throw new IOException("Projects did not survive reconnect.");
        await Capture(Path.Combine(output, "cloud-bee-projects.png"));
        await File.WriteAllTextAsync(Path.Combine(output, "projects-result.json"), new JsonObject { ["passed"] = true, ["checks"] = "project name/color/folder, multiple Chat and Council threads, independent drafts, sidebar project filtering, move, archive/restore and reconnect" }.ToJsonString());
    }
}
