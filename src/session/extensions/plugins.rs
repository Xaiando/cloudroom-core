use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Plugin {
    pub installed: bool,
    pub enabled: bool,
    pub settings: Value,
}
const CATALOG: &[(&str, &str, &str, &str)] = &[
    (
        "automations",
        "Automations",
        "Scheduled scripts and authorized agent tasks, with persisted run history.",
        "bb-plugin-automations",
    ),
    (
        "custom-instructions",
        "Custom instructions",
        "Append persistent instructions to new Cloud Bee messages without changing accepted requests.",
        "bb-plugin-custom-instructions",
    ),
    (
        "keep-awake",
        "Keep Awake",
        "Prevent idle system sleep while Core runs. The display may still turn off.",
        "bb-plugin-keep-awake",
    ),
];
pub(super) fn initialize(data: &mut Data) {
    for (id, _, _, _) in CATALOG {
        data.plugins.entry((*id).into()).or_insert_with(|| Plugin {
            installed: *id != "keep-awake",
            enabled: *id != "keep-awake",
            settings: if *id == "custom-instructions" {
                json!({"instructions":""})
            } else {
                json!({})
            },
        });
    }
}
pub(super) async fn list(State(manager): State<Arc<Manager>>) -> Json<Value> {
    let data = manager.extensions.data.lock().unwrap();
    let catalog:Vec<_>=CATALOG.iter().map(|(id,name,description,package)| {
        let p=&data.plugins[*id];
        json!({"id":id,"name":name,"description":description,"package":package,"version":"0.1.0-windows","source":"Cloudroom native port","installed":p.installed,"enabled":p.enabled,"settings":p.settings,"settings_schema":if *id=="custom-instructions" { json!({"instructions":{"type":"string","multiline":true,"max_length":4096}}) } else { json!({}) },"runtime_status":if *id=="keep-awake" { manager.extensions.power_status.lock().unwrap().clone() } else if p.installed&&p.enabled { "active".into() } else { "disabled".into() }})
    }).collect();
    Json(json!({"plugins":catalog,"general_cloudroom_sdk_compatible":false}))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Write {
    request_id: String,
    settings: Option<Value>,
}
pub(super) async fn action(
    State(manager): State<Arc<Manager>>,
    Path((id, action)): Path<(String, String)>,
    Json(input): Json<Write>,
) -> Result<Json<Value>> {
    key(&input.request_id)?;
    if !CATALOG.iter().any(|p| p.0 == id) {
        return Err(invalid(
            "this original plugin has not been ported to native Windows",
        ));
    }
    let payload = json!({"plugin":id,"action":action,"settings":input.settings});
    let result=manager.extensions.transact(|data| {
        if let Some(prior)=replay(data,&input.request_id,&payload)? { return Ok(prior); }
        if id=="automations" && action=="uninstall" && data.runs.values().any(|r|r.state=="running") { return Err(invalid("cancel active automation runs before uninstalling")); }
        let plugin=data.plugins.get_mut(&id).ok_or(Error::NotFound)?;
        match action.as_str() {
            "install"=>{plugin.installed=true;plugin.enabled=true;},
            "uninstall"=>{plugin.installed=false;plugin.enabled=false;},
            "enable"=>{if !plugin.installed {return Err(invalid("install the plugin first"));}plugin.enabled=true;},
            "disable"=>plugin.enabled=false,
            "settings"=>{
                if !plugin.installed {return Err(invalid("install the plugin first"));}
                let settings=input.settings.ok_or(invalid("settings required"))?;
                if id!="custom-instructions" || !settings.is_object() || settings.as_object().unwrap().len()!=1 || settings["instructions"].as_str().is_none_or(|s|s.chars().count()>4096||s.len()>12000) { return Err(invalid("Custom instructions accepts one instructions string within 4096 characters")); }
                plugin.settings=settings;
            },
            _=>return Err(invalid("unsupported plugin action")),
        }
        let result=json!({"id":id,"installed":plugin.installed,"enabled":plugin.enabled});data.ledger.insert(input.request_id,(payload,result.clone()));Ok(result)
    })?;
    Ok(Json(result))
}
pub(super) fn start_power(manager: &Arc<Manager>) {
    let weak = Arc::downgrade(manager);
    std::thread::spawn(move || {
        let mut active = false;
        loop {
            let Some(manager) = weak.upgrade() else { break };
            let stopping = manager.is_stopping()
                || manager
                    .extensions
                    .stopped
                    .load(std::sync::atomic::Ordering::Relaxed);
            let wanted = !stopping && manager.extensions.enabled("keep-awake");
            let (next, status) = power(wanted, active);
            active = next;
            *manager.extensions.power_status.lock().unwrap() = status.into();
            if stopping {
                break;
            }
            drop(manager);
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        #[cfg(windows)]
        if active {
            unsafe {
                windows_sys::Win32::System::Power::SetThreadExecutionState(
                    windows_sys::Win32::System::Power::ES_CONTINUOUS,
                );
            }
        }
    });
}

#[cfg(windows)]
fn power(wanted: bool, active: bool) -> (bool, &'static str) {
    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };
    if wanted != active
        && unsafe {
            SetThreadExecutionState(ES_CONTINUOUS | if wanted { ES_SYSTEM_REQUIRED } else { 0 })
        } == 0
    {
        return (active, "request_failed");
    }
    (wanted, if wanted { "active" } else { "disabled" })
}
#[cfg(not(windows))]
fn power(wanted: bool, _active: bool) -> (bool, &'static str) {
    (false, if wanted { "unsupported" } else { "disabled" })
}
