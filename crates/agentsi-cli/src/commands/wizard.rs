//! The terminal wizard of `agentsi mark`: it asks about each pending item,
//! then applies the answers. A port of `mark-wizard.ts`.

use agentsi_proto::marks_api::{
    DecideResult, Mark, MarkEntry, Outcome, PendingDecisionItem, PendingItem, Verdict,
};
use std::io::{BufRead, Write};

/// What the wizard calls on the core. A test replaces it.
pub trait Deps {
    fn apply_marks(&mut self, entries: &[MarkEntry]) -> Result<(), String>;
    fn decide(&mut self, id: i64, status: Verdict) -> Result<DecideResult, String>;
}

/// The lines that describe an item. The first one is the title.
pub fn describe(item: &PendingItem) -> Vec<String> {
    match item {
        PendingItem::Priority(p) => {
            let mut out = vec![
                format!(
                    "Priority {} #{} · owner {}{}",
                    p.minutes_id,
                    p.idx,
                    p.owner_role,
                    p.objective
                        .as_deref()
                        .filter(|o| !o.is_empty())
                        .map_or(String::new(), |o| format!(" · objective {o}"))
                ),
                format!("  {}", p.title),
                format!("  why: {}", p.why),
            ];
            if !p.sources.is_empty() {
                let refs: Vec<&str> = p
                    .sources
                    .iter()
                    .map(|s| s.get("ref").and_then(|r| r.as_str()).unwrap_or(""))
                    .collect();
                out.push(format!("  sources: {}", refs.join(", ")));
            }
            out
        }
        PendingItem::Decision(d) => {
            let mut out = vec![
                format!(
                    "Decision {}{}",
                    d.id,
                    d.minutes_id
                        .as_deref()
                        .filter(|m| !m.is_empty())
                        .map_or(" · product".to_string(), |m| format!(" · {m}"))
                ),
                format!("  {}", d.text),
            ];
            if let Some(why) = d.why.as_deref().filter(|w| !w.is_empty()) {
                out.push(format!("  why: {why}"));
            }
            if let Some(action) = d.action.as_deref().filter(|a| !a.is_empty()) {
                out.push(format!("  action: {action}"));
            }
            out
        }
    }
}

struct Reader<R: BufRead> {
    input: R,
}

impl<R: BufRead> Reader<R> {
    /// The first accepted key, or `None` when the input ends.
    fn ask(&mut self, out: &mut dyn Write, prompt: &str, keys: &[&str]) -> Option<String> {
        loop {
            let _ = write!(out, "{prompt}");
            let _ = out.flush();
            let mut line = String::new();
            if self.input.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let key = line.trim().to_lowercase();
            if keys.contains(&key.as_str()) {
                return Some(key);
            }
        }
    }
}

fn mark_of(key: &str) -> Mark {
    match key {
        "a" => Mark::Accepted,
        "c" => Mark::Changed,
        _ => Mark::Rejected,
    }
}

struct Verdicted {
    id: i64,
    status: Verdict,
    links: Vec<MarkEntry>,
}

/// Asks about each pending item, then applies the answers: one write for the
/// marks, one for each decision. Returns the exit code.
pub fn run(
    items: &[PendingItem],
    input: impl BufRead,
    out: &mut dyn Write,
    err: &mut dyn Write,
    deps: &mut dyn Deps,
) -> i32 {
    if items.is_empty() {
        let _ = writeln!(out, "Nothing pending.");
        return 0;
    }
    let mut reader = Reader { input };
    let priorities = items
        .iter()
        .filter(|i| matches!(i, PendingItem::Priority(_)))
        .count();
    let _ = writeln!(
        out,
        "{priorities} priorities and {} decisions pending.",
        items.len() - priorities
    );
    let mut marks: Vec<MarkEntry> = Vec::new();
    let mut verdicts: Vec<Verdicted> = Vec::new();
    let mut skipped = 0;
    for (n, item) in items.iter().enumerate() {
        let lines = describe(item);
        let _ = writeln!(out, "\n[{}/{}] {}", n + 1, items.len(), lines[0]);
        for line in &lines[1..] {
            let _ = writeln!(out, "{line}");
        }
        match item {
            PendingItem::Priority(p) => {
                let Some(key) = reader.ask(
                    out,
                    "[a]ccept [c]hange [r]eject [s]kip [q]uit > ",
                    &["a", "c", "r", "s", "q"],
                ) else {
                    break;
                };
                match key.as_str() {
                    "q" => break,
                    "s" => skipped += 1,
                    _ => {
                        let Some(answer) = reader.ask(
                            out,
                            "New (you did not have it in mind)? [y/N] > ",
                            &["y", "n", ""],
                        ) else {
                            break;
                        };
                        marks.push(MarkEntry {
                            minutes_id: p.minutes_id.clone(),
                            idx: p.idx,
                            mark: mark_of(&key),
                            is_new: answer == "y",
                        });
                    }
                }
            }
            PendingItem::Decision(d) => {
                let Some(key) = reader.ask(
                    out,
                    "[a]pprove [r]eject [s]kip [q]uit > ",
                    &["a", "r", "s", "q"],
                ) else {
                    break;
                };
                match key.as_str() {
                    "q" => break,
                    "s" => skipped += 1,
                    _ => verdicts.push(Verdicted {
                        id: d.id,
                        status: if key == "a" {
                            Verdict::Approved
                        } else {
                            Verdict::Rejected
                        },
                        links: links(d, key == "a"),
                    }),
                }
            }
        }
    }

    let mut code = 0;
    let mut fail = |err: &mut dyn Write, line: String| {
        let _ = writeln!(err, "{line}");
        code = 1;
    };
    let _ = writeln!(out);
    if !marks.is_empty()
        && let Err(e) = deps.apply_marks(&marks)
    {
        fail(err, format!("Marks not applied: {e}"));
        marks.clear();
    }
    let (mut approved, mut rejected) = (0, 0);
    // Priorities that the owner marked in this run: an approval never overwrites them.
    let mut taken: Vec<(String, u32)> = marks
        .iter()
        .map(|e| (e.minutes_id.clone(), e.idx))
        .collect();
    for v in verdicts {
        let res = match deps.decide(v.id, v.status) {
            Ok(res) => res,
            Err(e) => {
                fail(err, format!("Decision {}: {e}", v.id));
                continue;
            }
        };
        if let Some(error) = &res.commit_error {
            let _ = writeln!(err, "Commit skipped: {error}");
        }
        if res.status == Outcome::Stale {
            fail(
                err,
                format!(
                    "Decision {} not applied: the target changed since the proposal, so it is now stale.",
                    v.id
                ),
            );
            continue;
        }
        match v.status {
            Verdict::Approved => approved += 1,
            Verdict::Rejected => rejected += 1,
        }
        let name = v.status.as_str();
        match &res.minutes_id {
            Some(m) => {
                let _ = writeln!(out, "Decision {} {name} ({m}).", v.id);
            }
            None => {
                let _ = writeln!(out, "Decision {} {name}.", v.id);
            }
        }
        if res.status == Outcome::Approved
            && let Some(action) = res.action.as_deref().filter(|a| !a.is_empty())
        {
            let _ = writeln!(out, "Action: {action}");
        }
        let links: Vec<MarkEntry> = v
            .links
            .into_iter()
            .filter(|e| !taken.contains(&(e.minutes_id.clone(), e.idx)))
            .collect();
        if !links.is_empty() {
            match deps.apply_marks(&links) {
                Ok(()) => {
                    taken.extend(links.iter().map(|e| (e.minutes_id.clone(), e.idx)));
                    let list: Vec<String> = links.iter().map(|e| format!("#{}", e.idx)).collect();
                    let _ = writeln!(
                        out,
                        "Marked accepted: {} ({}).",
                        list.join(", "),
                        links[0].minutes_id
                    );
                }
                Err(e) => fail(
                    err,
                    format!("Linked priorities of decision {} not marked: {e}", v.id),
                ),
            }
        }
    }
    let _ = writeln!(
        out,
        "Marked {}, approved {approved}, rejected {rejected}, skipped {skipped}.",
        marks.len()
    );
    code
}

/// The priorities that an approval marks accepted: the unmarked ones that the
/// decision links in its minutes.
fn links(d: &PendingDecisionItem, approved: bool) -> Vec<MarkEntry> {
    match (&d.minutes_id, approved) {
        (Some(minutes), true) if !minutes.is_empty() => d
            .priorities
            .iter()
            .map(|idx| MarkEntry {
                minutes_id: minutes.clone(),
                idx: *idx,
                mark: Mark::Accepted,
                is_new: false,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::marks_api::PendingPriority;

    #[derive(Default)]
    struct Fake {
        marks: Vec<Vec<MarkEntry>>,
        decided: Vec<(i64, Verdict)>,
        stale: Vec<i64>,
    }

    impl Deps for Fake {
        fn apply_marks(&mut self, entries: &[MarkEntry]) -> Result<(), String> {
            self.marks.push(entries.to_vec());
            Ok(())
        }
        fn decide(&mut self, id: i64, status: Verdict) -> Result<DecideResult, String> {
            self.decided.push((id, status));
            Ok(DecideResult {
                product: "p".into(),
                minutes_id: Some("m1".into()),
                status: if self.stale.contains(&id) {
                    Outcome::Stale
                } else if status == Verdict::Approved {
                    Outcome::Approved
                } else {
                    Outcome::Rejected
                },
                action: Some("Run it".into()),
                commit_error: None,
                refused: None,
            })
        }
    }

    fn priority(idx: u32) -> PendingItem {
        PendingItem::Priority(PendingPriority {
            minutes_id: "m1".into(),
            idx,
            title: "Ship".into(),
            why: "Soon".into(),
            owner_role: "pm".into(),
            objective: Some("o1".into()),
            sources: vec![serde_json::json!({"type": "path", "ref": "a.md"})],
        })
    }

    fn decision(id: i64, linked: Vec<u32>) -> PendingItem {
        PendingItem::Decision(PendingDecisionItem {
            id,
            minutes_id: Some("m1".into()),
            text: "Go".into(),
            why: Some("Because".into()),
            action: Some("Run it".into()),
            priorities: linked,
        })
    }

    fn go(items: &[PendingItem], answers: &str, fake: &mut Fake) -> (i32, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(items, answers.as_bytes(), &mut out, &mut err, fake);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn it_applies_the_marks_in_one_batch_and_the_decisions_one_by_one() {
        let mut fake = Fake::default();
        let items = [priority(0), priority(1), decision(7, vec![])];
        let (code, out, _) = go(&items, "a\ny\nr\n\na\n", &mut fake);
        assert_eq!(code, 0);
        assert_eq!(fake.marks.len(), 1);
        assert_eq!(fake.marks[0].len(), 2);
        assert!(fake.marks[0][0].is_new && !fake.marks[0][1].is_new);
        assert_eq!(fake.decided, [(7, Verdict::Approved)]);
        assert!(
            out.contains("2 priorities and 1 decisions pending."),
            "{out}"
        );
        assert!(
            out.contains("[1/3] Priority m1 #0 · owner pm · objective o1"),
            "{out}"
        );
        assert!(
            out.contains("Decision 7 approved (m1).\nAction: Run it"),
            "{out}"
        );
        assert!(
            out.ends_with("Marked 2, approved 1, rejected 0, skipped 0.\n"),
            "{out}"
        );
    }

    #[test]
    fn a_bad_key_asks_again_and_q_applies_what_was_answered() {
        let mut fake = Fake::default();
        let items = [priority(0), priority(1)];
        let (_, out, _) = go(&items, "x\nc\nn\nq\n", &mut fake);
        assert_eq!(out.matches("[a]ccept").count(), 3, "{out}");
        assert_eq!(fake.marks[0].len(), 1);
        assert_eq!(fake.marks[0][0].mark, Mark::Changed);
    }

    #[test]
    fn the_end_of_the_input_at_the_new_question_drops_that_mark() {
        let mut fake = Fake::default();
        let (code, out, _) = go(&[priority(0)], "a\n", &mut fake);
        assert_eq!(code, 0);
        assert!(fake.marks.is_empty());
        assert!(out.ends_with("Marked 0, approved 0, rejected 0, skipped 0.\n"));
    }

    #[test]
    fn nothing_pending_asks_nothing() {
        let mut fake = Fake::default();
        let (code, out, _) = go(&[], "", &mut fake);
        assert_eq!((code, out.as_str()), (0, "Nothing pending.\n"));
    }

    #[test]
    fn a_stale_decision_fails_the_run_but_the_others_go_on() {
        let mut fake = Fake {
            stale: vec![7],
            ..Fake::default()
        };
        let items = [decision(7, vec![]), decision(8, vec![])];
        let (code, out, err) = go(&items, "a\na\n", &mut fake);
        assert_eq!(code, 1);
        assert!(err.contains("Decision 7 not applied"), "{err}");
        assert!(out.contains("Decision 8 approved (m1)."), "{out}");
        assert!(
            out.ends_with("approved 1, rejected 0, skipped 0.\n"),
            "{out}"
        );
    }

    #[test]
    fn approving_marks_its_linked_priorities_but_never_one_marked_in_this_run() {
        let mut fake = Fake::default();
        let items = [priority(1), decision(7, vec![0, 1])];
        let (_, out, _) = go(&items, "r\n\na\n", &mut fake);
        assert_eq!(fake.marks.len(), 2);
        let linked = &fake.marks[1];
        assert_eq!(linked.len(), 1);
        assert_eq!((linked[0].idx, linked[0].mark), (0, Mark::Accepted));
        assert!(out.contains("Marked accepted: #0 (m1)."), "{out}");
    }

    #[test]
    fn rejecting_a_decision_marks_none_of_its_linked_priorities() {
        let mut fake = Fake::default();
        let (_, _, _) = go(&[decision(7, vec![0])], "r\n", &mut fake);
        assert!(fake.marks.is_empty());
        assert_eq!(fake.decided, [(7, Verdict::Rejected)]);
    }
}
