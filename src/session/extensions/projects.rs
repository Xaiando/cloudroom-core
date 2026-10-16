//! Project metadata never changes a saved thread's workspace or permissions.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Project {
    name: String,
    description: String,
    color: String,
    folder: Option<PathBuf>,
    workspace_id: Option<String>,
    archived: bool,
    revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Write {
    request_id: String,
    expected_revision: u64,
    name: String,
    description: String,
    color: String,
    folder: Option<PathBuf>,
    archived: bool,
}
pub(super) async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let data = manager.extensions.data.lock().unwrap();
    Json(json!({"projects":data.projects,"threads":data.project_threads}))
}
pub(super) async fn write(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Write>,
) -> Result<Json<Value>> {
    key(&id)?;
    key(&input.request_id)?;
    if input.name.trim().is_empty()
        || input.name.chars().count() > 80
        || input.name.chars().any(char::is_control)
        || input.description.len() > 4000
        || !matches!(
            input.color.as_str(),
            "lavender" | "blue" | "green" | "amber" | "rose"
        )
    {
        return Err(invalid(
            "choose a project name, supported color and short description",
        ));
    }
    let payload = json!({"id":id,"revision":input.expected_revision,"name":input.name,"description":input.description,"color":input.color,"folder":input.folder,"archived":input.archived});
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        let prior = data.projects.get(&id);
        let revision = prior.map_or(0, |p| p.revision);
        if revision != input.expected_revision {
            return Err(invalid(
                "project changed; reopen its settings before saving",
            ));
        }
        let (folder, workspace_id) = match &input.folder {
            Some(path) => {
                #[cfg(windows)]
                {
                    let workspace = manager
                        .workspaces
                        .register_local(&format!("local_{}", input.request_id), path)
                        .map_err(|_| invalid("choose an existing ordinary local Windows folder"))?;
                    let display = workspace.path.to_string_lossy();
                    let display = display.strip_prefix(r"\\?\").unwrap_or(&display);
                    (Some(PathBuf::from(display)), Some(workspace.id))
                }
                #[cfg(not(windows))]
                {
                    let _ = path;
                    return Err(invalid("external project folders require native Windows"));
                }
            }
            None => (None, None),
        };
        data.projects.insert(
            id.clone(),
            Project {
                name: input.name.trim().into(),
                description: input.description.trim().into(),
                color: input.color,
                folder,
                workspace_id,
                archived: input.archived,
                revision: revision + 1,
            },
        );
        let result = json!({"id":id,"revision":revision+1});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Assign {
    request_id: String,
    project_id: Option<String>,
    expected_project: Option<String>,
}
pub(super) async fn assign(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Assign>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let exists = manager.bee.contains(&id);
    #[cfg(windows)]
    let exists = exists || manager.bee.council_contains(&id);
    if !exists && manager.session(&id).await.is_err() {
        return Err(Error::NotFound);
    }
    let payload = json!({"thread":id,"project":input.project_id,"expected":input.expected_project});
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        if data.project_threads.get(&id) != input.expected_project.as_ref() {
            return Err(invalid("thread project changed; reload before moving it"));
        }
        if let Some(project) = input.project_id {
            if !data.projects.contains_key(&project) {
                return Err(Error::NotFound);
            }
            data.project_threads.insert(id.clone(), project);
        } else {
            data.project_threads.remove(&id);
        }
        let result = json!({"id":id,"project_id":data.project_threads.get(&id)});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
