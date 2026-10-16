use super::*;

#[derive(Clone, Default, Serialize, Deserialize)]
pub(super) struct Organization {
    title: String,
    collection: String,
    pinned: bool,
    archived: bool,
    revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Write {
    request_id: String,
    expected_revision: u64,
    title: String,
    collection: String,
    pinned: bool,
    archived: bool,
}
pub(super) async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    Json(json!({"threads":manager.extensions.data.lock().unwrap().threads}))
}
pub(super) async fn write(
    State(manager): State<Arc<Manager>>,
    Path(id): Path<String>,
    Json(input): Json<Write>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    let exists = manager.bee.contains(&id);
    #[cfg(windows)]
    let exists = exists || manager.bee.council_contains(&id);
    if !exists && manager.session(&id).await.is_err() {
        return Err(Error::NotFound);
    }
    if input.title.chars().count() > 160
        || input.collection.chars().count() > 80
        || input.title.contains(['\n', '\r'])
        || input.collection.contains(['\n', '\r'])
    {
        return Err(invalid("thread title or collection exceeds limits"));
    }
    let payload = json!({"thread":id,"revision":input.expected_revision,"title":input.title,"collection":input.collection,"pinned":input.pinned,"archived":input.archived});
    let result = manager.extensions.transact(|data| {
        if let Some(prior) = replay(data, &input.request_id, &payload)? {
            return Ok(prior);
        }
        let current = data.threads.entry(id.clone()).or_default();
        if current.revision != input.expected_revision {
            return Err(invalid("thread organization changed; reload before saving"));
        }
        *current = Organization {
            title: input.title.trim().into(),
            collection: input.collection.trim().into(),
            pinned: input.pinned,
            archived: input.archived,
            revision: current.revision + 1,
        };
        let result = json!({"id":id,"revision":current.revision});
        data.ledger
            .insert(input.request_id, (payload, result.clone()));
        Ok(result)
    })?;
    Ok(Json(result))
}
