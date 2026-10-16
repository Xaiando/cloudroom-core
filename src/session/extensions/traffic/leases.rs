//! Deterministic all-or-nothing resource acquisition. No model participates in arbitration.
use super::*;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Resource {
    pub id: String,
    pub workspace_id: String,
    pub path: String,
    pub kind: String,
    pub key: String,
    pub directory: bool,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(super) struct Owner {
    pub agent_id: String,
    pub session_id: String,
    pub task_id: String,
    pub can_work: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Reservation {
    pub id: String,
    pub owner: Owner,
    pub resources: Vec<String>,
    pub state: String,
    pub requested_at: u64,
    pub expires_at: u64,
    pub ttl: u64,
    pub lease_id: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Lease {
    pub id: String,
    pub reservation_id: String,
    pub owner: Owner,
    pub resources: Vec<String>,
    pub token: String,
    pub generation: u64,
    pub state: String,
    pub acquired_at: u64,
    pub expires_at: u64,
    pub heartbeat_at: u64,
    pub quiescent: bool,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub(super) struct Engine {
    pub resources: BTreeMap<String, Resource>,
    pub reservations: Vec<Reservation>,
    pub leases: BTreeMap<String, Lease>,
    pub generation: u64,
}
pub(super) fn held(state: &str) -> bool {
    matches!(
        state,
        "reserved" | "active" | "verification" | "integrating" | "recovery"
    )
}
pub(super) fn overlap(a: &Resource, b: &Resource) -> bool {
    a.key == b.key
        || a.directory && b.key.starts_with(&(a.key.clone() + "/"))
        || b.directory && a.key.starts_with(&(b.key.clone() + "/"))
}
impl Engine {
    fn conflicts(&self, a: &[String], b: &[String]) -> bool {
        a.iter().any(|x| {
            b.iter()
                .any(|y| overlap(&self.resources[x], &self.resources[y]))
        })
    }
    pub fn register(&mut self, r: Resource) -> Result<String> {
        if let Some(old) = self.resources.get(&r.id) {
            if old.key != r.key || old.directory != r.directory {
                return Err(invalid("resource identity collision"));
            }
            return Ok(r.id);
        }
        if self.resources.len() >= 2048 {
            return Err(invalid("traffic resource limit reached"));
        }
        let id = r.id.clone();
        self.resources.insert(id.clone(), r);
        Ok(id)
    }
    pub fn reserve(
        &mut self,
        id: &str,
        owner: Owner,
        mut resources: Vec<String>,
        ttl: u64,
        now: u64,
    ) -> Result<String> {
        if !owner.can_work {
            return Err(invalid(
                "Chat-only participants cannot reserve write execution",
            ));
        }
        resources.sort();
        resources.dedup();
        if resources.is_empty()
            || resources.len() > 16
            || resources.iter().any(|r| !self.resources.contains_key(r))
            || !(1000..=120000).contains(&ttl)
        {
            return Err(invalid(
                "reserve 1–16 registered resources with a 1–120 second lease",
            ));
        }
        if let Some(r) = self.reservations.iter().find(|r| r.id == id) {
            if r.owner.task_id != owner.task_id
                || r.owner.session_id != owner.session_id
                || r.resources != resources
                || r.ttl != ttl
            {
                return Err(invalid(
                    "reservation ID already has different ownership or resources",
                ));
            }
            return Ok(r.id.clone());
        }
        // An owner must declare its entire set up front. Incremental acquisition is forbidden.
        if self
            .leases
            .values()
            .any(|l| held(&l.state) && l.owner.task_id == owner.task_id)
            || self
                .reservations
                .iter()
                .any(|r| r.state == "waiting" && r.owner.task_id == owner.task_id)
        {
            return Err(invalid(
                "release the current resource set before requesting another; incremental acquisition is forbidden",
            ));
        }
        if self.reservations.len() >= 4096 {
            return Err(invalid("traffic reservation limit reached"));
        }
        self.reservations.push(Reservation {
            id: id.into(),
            owner,
            resources,
            state: "waiting".into(),
            requested_at: now,
            expires_at: now.saturating_add(120000),
            ttl,
            lease_id: None,
        });
        self.drain(now)?;
        Ok(id.into())
    }
    pub fn drain(&mut self, now: u64) -> Result<()> {
        for index in 0..self.reservations.len() {
            if self.reservations[index].state != "waiting" {
                continue;
            }
            if self.reservations[index].expires_at <= now {
                self.reservations[index].state = "expired".into();
                continue;
            }
            let r = self.reservations[index].clone();
            let owned = self
                .leases
                .values()
                .any(|l| held(&l.state) && self.conflicts(&r.resources, &l.resources));
            let earlier = self.reservations[..index]
                .iter()
                .any(|p| p.state == "waiting" && self.conflicts(&r.resources, &p.resources));
            if owned || earlier {
                continue;
            }
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(invalid("lease generation exhausted"))?;
            let id = format!("lease_{}", short(&r.id));
            let lease = Lease {
                id: id.clone(),
                reservation_id: r.id.clone(),
                owner: r.owner,
                resources: r.resources,
                token: random_token()?,
                generation: self.generation,
                state: "reserved".into(),
                acquired_at: now,
                expires_at: now.saturating_add(r.ttl),
                heartbeat_at: now,
                quiescent: false,
            };
            self.leases.insert(id.clone(), lease);
            self.reservations[index].state = "granted".into();
            self.reservations[index].lease_id = Some(id);
        }
        Ok(())
    }
    pub fn validate(&self, id: &str, token: &str, generation: u64, now: u64) -> Result<&Lease> {
        let l = self.leases.get(id).ok_or(Error::NotFound)?;
        if l.token != token
            || l.generation != generation
            || !held(&l.state)
            || l.state == "recovery"
            || l.expires_at <= now
        {
            return Err(invalid(
                "lease is stale, expired, revoked or requires recovery",
            ));
        }
        Ok(l)
    }
    pub fn renew(&mut self, id: &str, token: &str, generation: u64, now: u64) -> Result<()> {
        let l = self.validate(id, token, generation, now)?;
        let ttl = self
            .reservations
            .iter()
            .find(|r| r.id == l.reservation_id)
            .ok_or(Error::NotFound)?
            .ttl;
        let l = self.leases.get_mut(id).unwrap();
        l.heartbeat_at = now;
        l.expires_at = now.saturating_add(ttl);
        Ok(())
    }
    pub fn phase(
        &mut self,
        id: &str,
        token: &str,
        generation: u64,
        state: &str,
        quiescent: bool,
        now: u64,
    ) -> Result<()> {
        self.validate(id, token, generation, now)?;
        if !matches!(state, "active" | "verification" | "integrating")
            || state == "verification" && !quiescent
        {
            return Err(invalid(
                "verification requires confirmed execution termination",
            ));
        }
        let l = self.leases.get_mut(id).unwrap();
        l.state = state.into();
        l.quiescent = quiescent;
        Ok(())
    }
    pub fn expire(&mut self, now: u64) -> Result<()> {
        for l in self.leases.values_mut() {
            if held(&l.state) && l.state != "recovery" && l.expires_at <= now {
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or(invalid("lease generation exhausted"))?;
                l.generation = self.generation;
                l.state = "recovery".into();
            }
        }
        // Expiration fences old owners but deliberately retains overlapping ownership.
        self.drain(now)
    }
    pub fn release(
        &mut self,
        id: &str,
        token: &str,
        generation: u64,
        quiescent: bool,
        now: u64,
    ) -> Result<()> {
        self.validate(id, token, generation, now)?;
        if !quiescent {
            return Err(invalid(
                "termination must be confirmed before releasing write ownership",
            ));
        }
        let l = self.leases.get_mut(id).unwrap();
        l.quiescent = true;
        l.state = "released".into();
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(invalid("lease generation exhausted"))?;
        l.generation = self.generation;
        self.drain(now)
    }
    pub fn recover_release(&mut self, id: &str, quiescent: bool, now: u64) -> Result<()> {
        if !quiescent {
            return Err(invalid("recovery cannot release an unconfirmed writer"));
        }
        let l = self.leases.get_mut(id).ok_or(Error::NotFound)?;
        if !held(&l.state) {
            return Ok(());
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(invalid("lease generation exhausted"))?;
        l.state = "released".into();
        l.quiescent = true;
        l.generation = self.generation;
        self.drain(now)
    }
    pub fn cancel_wait(&mut self, id: &str, now: u64) -> Result<()> {
        if let Some(r) = self
            .reservations
            .iter_mut()
            .find(|r| r.id == id && r.state == "waiting")
        {
            r.state = "cancelled".into();
        }
        self.drain(now)
    }
    pub fn restart(&mut self) -> Result<()> {
        for l in self.leases.values_mut().filter(|l| held(&l.state)) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or(invalid("lease generation exhausted"))?;
            l.generation = self.generation;
            l.state = "recovery".into();
        }
        for r in self
            .reservations
            .iter_mut()
            .filter(|r| r.state == "waiting")
        {
            r.state = "recovery".into();
        }
        Ok(())
    }
    pub fn reservation_lease(&self, id: &str) -> Option<Lease> {
        self.reservations
            .iter()
            .find(|r| r.id == id)
            .and_then(|r| r.lease_id.as_ref())
            .and_then(|id| self.leases.get(id))
            .cloned()
    }
    pub fn traffic(&self, resources: &[String]) -> (&'static str, Option<&Lease>) {
        if let Some(l) = self
            .leases
            .values()
            .find(|l| held(&l.state) && self.conflicts(resources, &l.resources))
        {
            return (
                match l.state.as_str() {
                    "active" | "integrating" => "red",
                    "reserved" => "yellow",
                    _ => "blue",
                },
                Some(l),
            );
        }
        if self
            .reservations
            .iter()
            .any(|r| r.state == "waiting" && self.conflicts(resources, &r.resources))
        {
            return ("yellow", None);
        }
        ("green", None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(id: &str) -> Owner {
        Owner {
            agent_id: id.into(),
            session_id: format!("session_{id}"),
            task_id: id.into(),
            can_work: true,
        }
    }
    fn resource(id: &str, path: &str, dir: bool) -> Resource {
        Resource {
            id: id.into(),
            workspace_id: "fixture".into(),
            path: path.into(),
            key: format!("d:/fixture/{path}"),
            kind: if dir { "directory" } else { "file" }.into(),
            directory: dir,
        }
    }
    fn engine() -> Engine {
        let mut e = Engine::default();
        for r in [
            resource("a", "src/a.rs", false),
            resource("b", "src/b.rs", false),
            resource("src", "src", true),
        ] {
            e.register(r).unwrap();
        }
        e
    }
    #[test]
    fn same_resource_queues_and_independent_resources_proceed() {
        let mut e = engine();
        e.reserve("one", owner("one"), vec!["a".into()], 1000, 0)
            .unwrap();
        e.reserve("two", owner("two"), vec!["a".into()], 1000, 0)
            .unwrap();
        e.reserve("other", owner("other"), vec!["b".into()], 1000, 0)
            .unwrap();
        assert!(e.reservation_lease("two").is_none());
        assert!(e.reservation_lease("other").is_some());
        let l = e.reservation_lease("one").unwrap();
        e.release(&l.id, &l.token, l.generation, true, 1).unwrap();
        assert!(e.reservation_lease("two").is_some());
    }
    #[test]
    fn directory_conflicts_atomic_sets_and_deadlock_prevention() {
        let mut e = engine();
        e.reserve("one", owner("one"), vec!["src".into()], 1000, 0)
            .unwrap();
        e.reserve("two", owner("two"), vec!["a".into(), "b".into()], 1000, 0)
            .unwrap();
        assert!(e.reservation_lease("two").is_none());
        assert!(
            e.reserve("increment", owner("one"), vec!["b".into()], 1000, 0)
                .is_err()
        );
        let l = e.reservation_lease("one").unwrap();
        e.release(&l.id, &l.token, l.generation, true, 1).unwrap();
        assert_eq!(e.reservation_lease("two").unwrap().resources.len(), 2);
    }
    #[test]
    fn expiration_fences_stale_commits_and_never_assumes_termination() {
        let mut e = engine();
        e.reserve("one", owner("one"), vec!["a".into()], 1000, 0)
            .unwrap();
        e.reserve("two", owner("two"), vec!["a".into()], 1000, 0)
            .unwrap();
        let old = e.reservation_lease("one").unwrap();
        e.expire(1001).unwrap();
        assert!(
            e.validate(&old.id, &old.token, old.generation, 1001)
                .is_err()
        );
        assert!(e.reservation_lease("two").is_none());
        assert!(e.recover_release(&old.id, false, 1001).is_err());
        e.recover_release(&old.id, true, 1001).unwrap();
        assert!(e.reservation_lease("two").is_some());
    }
    #[test]
    fn duplicate_reservations_and_restart_keep_authoritative_fences() {
        let mut e = engine();
        for _ in 0..2 {
            e.reserve("one", owner("one"), vec!["a".into()], 1000, 0)
                .unwrap();
        }
        assert_eq!(e.leases.len(), 1);
        let old = e.reservation_lease("one").unwrap();
        let mut restored: Engine =
            serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
        restored.restart().unwrap();
        assert!(
            restored
                .validate(&old.id, &old.token, old.generation, 1)
                .is_err()
        );
        assert_eq!(restored.traffic(&["a".into()]).0, "blue");
    }
    #[test]
    fn chat_owners_and_unconfirmed_release_are_denied() {
        let mut e = engine();
        let mut chat = owner("chat");
        chat.can_work = false;
        assert!(e.reserve("chat", chat, vec!["a".into()], 1000, 0).is_err());
        e.reserve("work", owner("work"), vec!["a".into()], 1000, 0)
            .unwrap();
        let l = e.reservation_lease("work").unwrap();
        assert!(e.release(&l.id, &l.token, l.generation, false, 1).is_err());
    }
    #[test]
    fn simultaneous_callers_cannot_obtain_conflicting_owners() {
        let e = Arc::new(Mutex::new(engine()));
        let mut threads = vec![];
        for n in 0..16 {
            let e = e.clone();
            threads.push(std::thread::spawn(move || {
                e.lock()
                    .unwrap()
                    .reserve(
                        &format!("r{n}"),
                        owner(&format!("t{n}")),
                        vec!["a".into()],
                        1000,
                        0,
                    )
                    .unwrap()
            }));
        }
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(e.lock().unwrap().leases.len(), 1);
    }
}
