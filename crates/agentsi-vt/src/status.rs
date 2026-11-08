//! The program status protocol (OSC 7501) as Rust types.

use std::collections::BTreeMap;

/// What a blocked program needs from the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockedKind {
    /// The program did not say, or sent a kind that this version does not know.
    Unspecified,
    /// Approval to do something.
    Permission,
    /// An answer that the user types.
    Question,
    /// A login, a password, a token or another credential.
    Auth,
}

/// What a program says it is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentState {
    /// At rest, waiting for the next instruction.
    Idle,
    /// Running on its own.
    Working,
    /// Finished a piece of work. The result is ready.
    Done,
    /// Cannot continue until the user acts. `msg` says why.
    Blocked { kind: BlockedKind, msg: String },
    /// Failed and stopped.
    Error,
}

impl AgentState {
    /// The protocol name of the state: `idle`, `working`, `done`, `blocked` or `error`.
    pub fn name(&self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Done => "done",
            AgentState::Blocked { .. } => "blocked",
            AgentState::Error => "error",
        }
    }
}

/// One valid report from the program. The text is untrusted: do not treat it as markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReport {
    /// The record that the report is about. Empty for the program itself.
    pub id: String,
    /// A stable machine name of the program, such as `claude-code`. Empty if the report has none.
    pub app: String,
    /// A short label for people.
    pub title: String,
    /// One line for people, already decoded.
    pub message: String,
    /// 0 to 100, for `Working` and `Blocked` when the program sent one.
    pub progress: Option<u8>,
    pub state: AgentState,
}

/// The records of the protocol: one for each id, with the rules of `GhosttyTerminalProgramStatusFn`.
#[derive(Clone, Debug, Default)]
pub struct StatusRecords {
    records: BTreeMap<String, (u64, StatusReport)>,
    /// Counts the reports, so the record that was updated longest ago is known.
    clock: u64,
}

impl StatusRecords {
    const MAX_RECORDS: usize = 256;

    /// A report replaces its record completely. At the limit, the record updated longest ago goes
    /// (never the record of the program itself), and its id is returned.
    pub(crate) fn put(&mut self, report: StatusReport) -> Option<String> {
        let mut evicted = None;
        if !self.records.contains_key(&report.id) && self.records.len() >= Self::MAX_RECORDS {
            let oldest = self
                .records
                .iter()
                .filter(|(id, _)| !id.is_empty())
                .min_by_key(|(_, (seq, _))| *seq)
                .map(|(id, _)| id.clone());
            if let Some(id) = oldest {
                self.records.remove(&id);
                evicted = Some(id);
            }
        }
        self.clock += 1;
        self.records.insert(report.id.clone(), (self.clock, report));
        evicted
    }

    /// Removes the record `id` and every record beneath it. An empty `id` removes all.
    pub(crate) fn clear(&mut self, id: &str) {
        if id.is_empty() {
            self.records.clear();
            return;
        }
        let prefix = format!("{id}/");
        self.records
            .retain(|k, _| k != id && !k.starts_with(&prefix));
    }

    /// The record of the program itself.
    pub fn root(&self) -> Option<&StatusReport> {
        self.records.get("").map(|(_, r)| r)
    }

    pub fn get(&self, id: &str) -> Option<&StatusReport> {
        self.records.get(id).map(|(_, r)| r)
    }

    pub fn iter(&self) -> impl Iterator<Item = &StatusReport> {
        self.records.values().map(|(_, r)| r)
    }

    /// The `app` of a record, or the app of its nearest ancestor that has one.
    pub fn app_of(&self, id: &str) -> &str {
        let mut id = id;
        loop {
            if let Some((_, r)) = self.records.get(id)
                && !r.app.is_empty()
            {
                return &r.app;
            }
            match id.rsplit_once('/') {
                Some((parent, _)) => id = parent,
                None if !id.is_empty() => id = "",
                None => return "",
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str, app: &str) -> StatusReport {
        StatusReport {
            id: id.into(),
            app: app.into(),
            title: String::new(),
            message: String::new(),
            progress: None,
            state: AgentState::Working,
        }
    }

    #[test]
    fn clear_removes_the_record_and_the_ones_beneath() {
        let mut r = StatusRecords::default();
        for id in ["", "build", "build/test", "builder", "deploy"] {
            r.put(report(id, ""));
        }
        r.clear("build");
        let ids: Vec<_> = r.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["", "builder", "deploy"]);
        r.clear("");
        assert_eq!(r.iter().count(), 0);
    }

    #[test]
    fn a_record_takes_the_app_of_its_nearest_ancestor() {
        let mut r = StatusRecords::default();
        r.put(report("", "deploy"));
        r.put(report("us-east", ""));
        r.put(report("build", "cargo"));
        r.put(report("build/test", ""));
        assert_eq!(r.app_of("us-east"), "deploy");
        assert_eq!(r.app_of("build/test"), "cargo");
        assert_eq!(r.app_of("nothing"), "deploy");
    }

    #[test]
    fn the_store_keeps_at_most_256_records_and_never_evicts_the_root() {
        let mut r = StatusRecords::default();
        r.put(report("", ""));
        let mut evicted = 0;
        for i in 0..300 {
            evicted += usize::from(r.put(report(&format!("r{i:03}"), "")).is_some());
        }
        assert_eq!(r.iter().count(), 256);
        assert_eq!(evicted, 301 - 256);
        assert!(r.root().is_some());
        assert!(r.get("r000").is_none() && r.get("r299").is_some());
    }

    #[test]
    fn an_update_makes_a_record_young_again() {
        let mut r = StatusRecords::default();
        for i in 0..256 {
            r.put(report(&format!("r{i:03}"), ""));
        }
        r.put(report("r000", ""));
        assert_eq!(r.put(report("new", "")).as_deref(), Some("r001"));
    }
}
