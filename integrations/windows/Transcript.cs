using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Runtime.CompilerServices;
using System.Text.Json.Nodes;

namespace Cloudroom.Desktop;

public sealed class ChatMessage : INotifyPropertyChanged
{
    public string Role { get; init; } = "CODEX";
    public bool IsTool => Role == "TOOL";
    public bool IsMessage => !IsTool;
    private string text = "", detail = "";
    public string Text { get => text; set { text = value; Changed(); } }
    public string Detail { get => detail; set { detail = value; Changed(); } }
    public event PropertyChangedEventHandler? PropertyChanged;
    private void Changed([CallerMemberName] string? property = null) => PropertyChanged?.Invoke(this, new(property));
}

public sealed record SessionItem(string Id, string Title, string State, string Model, string Folder)
{
    public string Subtitle => (string.IsNullOrEmpty(Folder) ? "Default workspace" : Folder) + "  ·  " + State.Replace('_', ' ');
}

/// <summary>One model's session inside a multi-model chat.</summary>
public sealed record GroupTab(string Id, string Model, string State, string Mode = "work")
{
    public string Label => Model + " · " + Mode + (State is "running" or "starting_turn" ? "  ·  working" : State is "failed" or "process_lost" ? "  ·  failed" : State == "closed" ? "  ·  closed" : "");
}

public sealed class Transcript
{
    public ObservableCollection<ChatMessage> Messages { get; } = [];
    private readonly Dictionary<string, ChatMessage> byId = [];
    private string prompt = "initial";
    public long Cursor { get; private set; }
    public void Reset() { Messages.Clear(); byId.Clear(); Cursor = 0; prompt = "initial"; }
    private ChatMessage Message(string key, string role)
    {
        if (!byId.TryGetValue(key, out var message))
        {
            message = new ChatMessage { Role = role };
            byId[key] = message;
            Messages.Add(message);
        }
        return message;
    }
    public void Apply(JsonNode record)
    {
        long sequence = Json.Number(record, "sequence");
        if (sequence <= Cursor) return;
        var data = record["data"];
        string kind = Json.Text(record, "kind");
        if (kind == "receipt" && Json.Text(data, "command") == "prompt")
        {
            string id = Json.Text(data, "request_id"), state = Json.Text(data, "state");
            var message = Message("user:" + id, "YOU");
            message.Text = Json.Text(data?["input"], "text");
            message.Detail = state.Replace('_', ' ');
            if (state == "delivered" || prompt == "initial") prompt = id;
            string error = Json.Text(data, "error");
            if (error.Length > 0) Message("error:" + id, "CORE").Text = error;
        }
        else if (kind == "text_delta")
        {
            var message = Message("assistant:" + prompt + ":" + Json.Text(data, "item_id", "response"), "CODEX");
            message.Text = Limit(message.Text + Json.Text(data, "delta"));
            message.Detail = "";
        }
        else if (kind == "item_completed" && Json.Text(data, "text").Length > 0)
            Message("assistant:" + prompt + ":" + Json.Text(data, "item_id", "response"), "CODEX").Text = Limit(Json.Text(data, "text"));
        else if (kind is "item_started" or "item_completed" or "tool_delta" or "tool_snapshot")
        {
            string id = "tool:" + prompt + ":" + Json.Text(data, "item_id", "command");
            string command = Json.Text(data, "tool_name");
            if (command.Length > 0 || byId.ContainsKey(id))
            {
                var message = Message(id, "TOOL");
                if (command.Length > 0) message.Detail = command;
                string output = Json.Text(data, "output");
                if (output.Length > 0) message.Text = Limit(output);
                else message.Text = Limit((message.Text == "Running…" ? "" : message.Text) + Json.Text(data, "delta"));
                if (message.Text.Length == 0) message.Text = kind == "item_completed" ? "Completed" : "Running…";
            }
        }
        else if (kind == "state" && Json.Text(data, "reason").Length > 0)
            Message("state:" + sequence, "CORE").Text = Json.Text(data, "reason");
        Cursor = sequence;
    }
    private static string Limit(string text) => text.Length <= 96000 ? text : text[..96000] + "\n[Output display limit reached]";
}
