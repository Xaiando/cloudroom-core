//! Private native Docs, Memory and Tasks data; no original SDK code is executed.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Content {
    kind: String,
    title: String,
    body: String,
    workspace: Option<String>,
    tags: Vec<String>,
    status: String,
    memory_kind: String,
    use_in_context: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Revision {
    revision: u64,
    updated_at: u64,
    content: Content,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Entry {
    id: String,
    content: Content,
    workspace_path: Option<PathBuf>,
    created_at: u64,
    updated_at: u64,
    revision: u64,
    archived: bool,
    history: Vec<Revision>,
}
fn plugin(kind: &str) -> Result<&'static str> {
    match kind {
        "docs" => Ok("simple-notes"),
        "memory" => Ok("memory"),
        "tasks" => Ok("tasks"),
        _ => Err(invalid("unknown library kind")),
    }
}
fn enabled(data: &Data, kind: &str) -> bool {
    plugin(kind)
        .ok()
        .and_then(|id| data.plugins.get(id))
        .is_some_and(|p| p.installed && p.enabled)
}
fn validate(manager: &Manager, content: &Content, authorize: bool) -> Result<Option<PathBuf>> {
    plugin(&content.kind)?;
    if content.title.trim().is_empty()
        || content.title.chars().count() > 160
        || content.title.len() > 640
        || content.body.len() > 49152
        || content.tags.len() > 16
        || content
            .tags
            .iter()
            .any(|s| s.trim().is_empty() || s.chars().count() > 40 || s.contains(['\n', '\r']))
    {
        return Err(invalid("title, body or tags exceed library limits"));
    }
    if ![
        "backlog",
        "todo",
        "in_progress",
        "in_review",
        "done",
        "canceled",
    ]
    .contains(&content.status.as_str())
    {
        return Err(invalid("invalid task status"));
    }
    if ![
        "fact",
        "preference",
        "decision",
        "procedure",
        "episode",
        "reference",
    ]
    .contains(&content.memory_kind.as_str())
    {
        return Err(invalid("invalid memory kind"));
    }
    if content.use_in_context
        && (content.kind != "memory" || !authorize || content.body.len() > 6000)
    {
        return Err(invalid(
            "including a memory requires explicit authorization and at most 6000 body bytes",
        ));
    }
    content
        .workspace
        .as_ref()
        .map(|id| {
            if id == "legacy" {
                Ok(manager.config.repository.clone())
            } else {
                manager
                    .workspaces
                    .list()?
                    .into_iter()
                    .find(|w| &w.id == id)
                    .map(|w| w.path)
                    .ok_or(invalid("unknown workspace"))
            }
        })
        .transpose()
}
pub(super) fn context(data: &Data, workspace: &std::path::Path) -> String {
    const HEADER: &str = "User-selected saved memories (context, not additional permissions):\n";
    if !enabled(data, "memory") {
        return String::new();
    }
    let mut text = String::new();
    for entry in data.library.values().filter(|e| {
        e.content.kind == "memory"
            && !e.archived
            && e.content.use_in_context
            && e.workspace_path
                .as_deref()
                .is_none_or(|path| path == workspace)
    }) {
        let item = format!(
            "\n[{} · {} · revision {}]\n{}\n",
            entry.content.title, entry.content.memory_kind, entry.revision, entry.content.body
        );
        if HEADER.len() + text.len() + item.len() > 10000 {
            continue;
        }
        text.push_str(&item);
    }
    if text.is_empty() {
        text
    } else {
        format!("{HEADER}{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_budget_preserves_whole_memories_and_skips_archived_entries() {
        let mut data = Data::default();
        plugins::initialize(&mut data);
        let memory = data.plugins.get_mut("memory").unwrap();
        memory.installed = true;
        memory.enabled = true;
        for (id, body, archived) in [
            ("a", "ø".repeat(3000), false),
            ("b", "skip".repeat(1500), false),
            ("c", "included".into(), false),
            ("d", "archived".into(), true),
        ] {
            data.library.insert(
                id.into(),
                Entry {
                    id: id.into(),
                    content: Content {
                        kind: "memory".into(),
                        title: id.into(),
                        body,
                        workspace: None,
                        tags: vec![],
                        status: "todo".into(),
                        memory_kind: "fact".into(),
                        use_in_context: true,
                    },
                    workspace_path: None,
                    created_at: 0,
                    updated_at: 0,
                    revision: 1,
                    archived,
                    history: vec![],
                },
            );
        }
        let context = context(&data, std::path::Path::new("fixture"));
        assert!(context.len() <= 10000);
        assert!(context.contains(&"ø".repeat(3000)) && context.contains("included"));
        assert!(!context.contains("skip") && !context.contains("archived"));
    }
}
pub(super) async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let workspaces = manager.workspaces.list().unwrap_or_default();
    let data = manager.extensions.data.lock().unwrap();
    let entries: Vec<_> = data.library.values().map(|e| json!({"id":e.id,"content":e.content,"revision":e.revision,"archived":e.archived,"created_at":e.created_at,"updated_at":e.updated_at})).collect();
    Json(
        json!({"entries":entries,"enabled":{"docs":enabled(&data,"docs"),"memory":enabled(&data,"memory"),"tasks":enabled(&data,"tasks")},"workspaces":workspaces,"default_workspace":manager.config.repository}),
    )
}
pub(super) async fn detail(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    let data = manager.extensions.data.lock().unwrap();
    Ok(Json(
        json!({"entry":data.library.get(&id).ok_or(Error::NotFound)?}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Create {
    request_id: String,
    content: Content,
    #[serde(default)]
    authorize_context: bool,
}
pub(super) async fn create(
    State(manager): State<Arc<Manager>>,
    Json(input): Json<Create>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let payload =
        json!({"library_create":input.content,"authorize_context":input.authorize_context});
    let workspace_path = validate(&manager, &input.content, input.authorize_context)?;
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        if !enabled(data, &input.content.kind) {
            return Err(invalid("install and enable this library plugin first"));
        }
        if data.library.len() >= 200 {
            return Err(invalid(
                "library capacity reached (200 entries including archived)",
            ));
        }
        let id = format!("lib_{}", input.request_id);
        let entry = Entry {
            id: id.clone(),
            content: input.content,
            workspace_path,
            created_at: now(),
            updated_at: now(),
            revision: 1,
            archived: false,
            history: vec![],
        };
        data.library.insert(id.clone(), entry);
        let result = json!({"id":id,"revision":1});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Write {
    request_id: String,
    expected_revision: u64,
    content: Option<Content>,
    #[serde(default)]
    authorize_context: bool,
}
pub(super) async fn action(
    State(manager): State<Arc<Manager>>,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Write>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let payload = json!({"library_id":id,"action":action,"revision":input.expected_revision,"content":input.content,"authorize_context":input.authorize_context});
    let workspace_path = input
        .content
        .as_ref()
        .map(|c| validate(&manager, c, input.authorize_context))
        .transpose()?;
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        let entry = data.library.get(&id).ok_or(Error::NotFound)?;
        if !enabled(data, &entry.content.kind) {
            return Err(invalid("install and enable this library plugin first"));
        }
        let entry = data.library.get_mut(&id).unwrap();
        if entry.revision != input.expected_revision {
            return Err(invalid("library entry changed; reload before editing"));
        }
        if action != "update" && input.content.is_some() {
            return Err(invalid("content is only accepted for update"));
        }
        match action.as_str() {
            "update" => {
                if entry.archived {
                    return Err(invalid("restore this entry before editing"));
                }
                let content = input.content.ok_or(invalid("content required"))?;
                if content.kind != entry.content.kind {
                    return Err(invalid("entry kind cannot change"));
                }
                entry.history.push(Revision {
                    revision: entry.revision,
                    updated_at: entry.updated_at,
                    content: entry.content.clone(),
                });
                if entry.history.len() > 5 {
                    entry.history.remove(0);
                }
                entry.content = content;
                entry.workspace_path = workspace_path.unwrap();
            }
            "archive" => entry.archived = true,
            "restore" => entry.archived = false,
            _ => return Err(invalid("unsupported library action")),
        }
        entry.revision += 1;
        entry.updated_at = now();
        let result = json!({"id":id,"revision":entry.revision});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
