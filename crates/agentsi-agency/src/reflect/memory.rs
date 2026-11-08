//! The item operations of `memory.ts`, and the reads and the retire of the Memory
//! screen. The web page sorts the items as `newest_first` does.

use super::{Error, Result, typed_entry};
use crate::registry::Product;
use agentsi_proto::reflect_api::{AgentMemory, MemoryEntry, MemoryListResult};
use agentsi_store::meeting::{new_item_id, normalize_text};
use agentsi_store::read::{self, MemoryItem};
use agentsi_store::reflection::write_memory;
use rusqlite::{Connection, Transaction};
use std::collections::BTreeSet;

pub fn active(items: &[MemoryItem]) -> Vec<&MemoryItem> {
    items.iter().filter(|i| i.retired_at.is_none()).collect()
}

/// A new lesson with a free id. `items` gives the ids that are taken.
pub fn make_lesson(
    text: &str,
    source: &str,
    created_at: &str,
    items: &[MemoryItem],
) -> Result<MemoryItem> {
    let taken: BTreeSet<String> = items.iter().map(|i| i.id.clone()).collect();
    Ok(MemoryItem {
        id: new_item_id(&taken)?,
        text: normalize_text(text),
        kind: "lesson".into(),
        helpful: 0,
        harmful: 0,
        source: source.into(),
        created_at: created_at.into(),
        retired_at: None,
        retired_why: None,
    })
}

pub fn retire(items: &mut [MemoryItem], id: &str, why: &str, at: &str) {
    if let Some(item) = items.iter_mut().find(|i| i.id == id) {
        item.retired_at = Some(at.into());
        item.retired_why = Some(why.into());
    }
}

fn entry(i: MemoryItem) -> MemoryEntry {
    MemoryEntry {
        id: i.id,
        text: i.text,
        kind: i.kind,
        helpful: i.helpful,
        harmful: i.harmful,
        source: i.source,
        created_at: i.created_at,
        retired_at: i.retired_at,
        retired_why: i.retired_why,
    }
}

/// Newest first. Items of one time come in reverse order of position.
fn newest_first(items: Vec<MemoryItem>) -> Vec<MemoryEntry> {
    let mut indexed: Vec<(usize, MemoryItem)> = items.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.created_at
            .cmp(&a.1.created_at)
            .then_with(|| b.0.cmp(&a.0))
    });
    indexed.into_iter().map(|(_, i)| entry(i)).collect()
}

/// `memory.list`: the roster first, then each other agent that holds memory.
pub fn list(c: &Connection, product: &Product) -> Result<MemoryListResult> {
    let mut names = product.roles.clone();
    for agent in read::memory_agents(c, &product.id)? {
        if !names.contains(&agent) {
            names.push(agent);
        }
    }
    let mut agents = Vec::new();
    for agent in names {
        let items = newest_first(read::memory(c, &product.id, &agent)?);
        agents.push(AgentMemory { agent, items });
    }
    let mut reflections: Vec<_> = read::reflections(c, &product.id)?
        .iter()
        .filter_map(typed_entry)
        .collect();
    reflections.reverse();
    Ok(MemoryListResult {
        agents,
        reflections,
    })
}

/// `memory.retire`: retires one active item, with a reason.
pub fn retire_item(
    tx: &Transaction,
    product: &str,
    agent: &str,
    id: &str,
    why: &str,
    at: &str,
) -> Result<MemoryEntry> {
    let why = normalize_text(why);
    if why.is_empty() {
        return Err(Error::Conflict(format!("Item {id} needs a reason.")));
    }
    let mut items = read::memory(tx, product, agent)?;
    let Some(found) = items.iter().find(|i| i.id == id) else {
        return Err(Error::NotFound(format!(
            "No memory item \"{id}\" of {agent} in {product}."
        )));
    };
    if found.retired_at.is_some() {
        return Err(Error::Conflict(format!("Item {id} is already retired.")));
    }
    retire(&mut items, id, &why, at);
    write_memory(tx, product, agent, &items)?;
    let item = items.into_iter().find(|i| i.id == id);
    Ok(entry(item.expect("the item was found above")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_store::db::open_writer;

    fn item(id: &str, at: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            text: format!("text {id}"),
            kind: "fact".into(),
            helpful: 0,
            harmful: 0,
            source: "m1".into(),
            created_at: at.into(),
            retired_at: None,
            retired_why: None,
        }
    }

    fn product() -> Product {
        Product {
            id: "p".into(),
            name: "P".into(),
            paths: Vec::new(),
            status: "active".into(),
            roles: vec!["pm".into(), "dev".into()],
        }
    }

    #[test]
    fn the_list_has_the_roster_first_and_the_newest_item_first() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_memory(&tx, "p", "dev", &[item("m-1", "t1"), item("m-2", "t2")]).unwrap();
        write_memory(&tx, "p", "dev.api", &[item("m-3", "t1")]).unwrap();
        tx.commit().unwrap();
        let got = list(&c, &product()).unwrap();
        let names: Vec<&str> = got.agents.iter().map(|a| a.agent.as_str()).collect();
        assert_eq!(names, ["pm", "dev", "dev.api"]);
        assert!(got.agents[0].items.is_empty());
        assert_eq!(got.agents[1].items[0].id, "m-2");
    }

    #[test]
    fn retire_needs_an_active_item_and_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_memory(&tx, "p", "dev", &[item("m-1", "t1")]).unwrap();
        assert!(matches!(
            retire_item(&tx, "p", "dev", "m-9", "old", "t2"),
            Err(Error::NotFound(_))
        ));
        assert!(matches!(
            retire_item(&tx, "p", "dev", "m-1", "  ", "t2"),
            Err(Error::Conflict(_))
        ));
        let done = retire_item(&tx, "p", "dev", "m-1", " old  note ", "t2").unwrap();
        assert_eq!(done.retired_why.as_deref(), Some("old note"));
        assert_eq!(done.retired_at.as_deref(), Some("t2"));
        assert!(matches!(
            retire_item(&tx, "p", "dev", "m-1", "again", "t3"),
            Err(Error::Conflict(_))
        ));
    }
}
