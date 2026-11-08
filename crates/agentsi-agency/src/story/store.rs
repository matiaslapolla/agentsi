//! The stories of a product in the store: `listStories`, `loadStory`,
//! `saveStory`, `nextStoryId` and the story log of `story.ts` and
//! `story-log.ts`. A story transaction holds the lock of its product, so the
//! writes of one product run one at a time.

use super::record::{self, is_story_id};
use super::{Error, Result};
use crate::clock;
use crate::registry::Product;
use agentsi_proto::story_api::Story;
use agentsi_store::{Store, read, stories};
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

/// What `session.log` holds: where to read the log of the story.
pub fn log_ref(product: &str, id: &str) -> String {
    format!("story_logs {product}/{id}")
}

fn parse(product: &str, value: serde_json::Value) -> Result<Story> {
    let mut story: Story = serde_json::from_value(value).map_err(|e| {
        Error::Agency(crate::Error::Unavailable(format!(
            "a story of {product} does not parse: {e}"
        )))
    })?;
    // Stories written before the owner config name the first owner; the row keeps it until the next save.
    if story.proposed_by == "matias" {
        story.proposed_by = "owner".into();
    }
    Ok(story)
}

/// The ids of the stories of `product` in the archive record.
pub fn archived_ids(c: &Connection, product: &str) -> Result<BTreeSet<String>> {
    Ok(read::archived(c, product)?
        .into_iter()
        .filter(|a| a.kind == "story")
        .map(|a| a.reference)
        .collect())
}

/// True when the product itself is in the archive record.
pub fn product_archived(c: &Connection, product: &str) -> Result<bool> {
    Ok(read::archived(c, product)?
        .iter()
        .any(|a| a.kind == "product"))
}

/// Each story of the product, oldest id first. Archived stories are left out
/// unless `include_archived` is set; then they carry `archived: true`.
pub fn list(c: &Connection, product: &str, include_archived: bool) -> Result<Vec<Story>> {
    let archived = archived_ids(c, product)?;
    let mut out = Vec::new();
    for value in read::stories(c, product)? {
        let mut story = parse(product, value)?;
        if archived.contains(&story.id) {
            if !include_archived {
                continue;
            }
            story.archived = Some(true);
        }
        out.push(story);
    }
    out.sort_by_key(|s| record::number(&s.id));
    Ok(out)
}

/// The story by product and id.
pub fn load(c: &Connection, product: &str, id: &str) -> Result<Story> {
    if !is_story_id(id) {
        return Err(Error::invalid(format!("Invalid story id \"{id}\"")));
    }
    match read::story(c, product, id)? {
        Some(value) => parse(product, value),
        None => Err(Error::NotFound(format!(
            "Unknown story \"{id}\" in {product}"
        ))),
    }
}

/// The highest number of either type plus 1, with the prefix of `kind`.
pub fn next_id(c: &Connection, product: &str, kind: &str) -> Result<String> {
    let highest = read::stories(c, product)?
        .iter()
        .filter_map(|v| v.get("id").and_then(|i| i.as_str()).map(record::number))
        .max()
        .unwrap_or(0);
    Ok(format!("{}{:03}", record::prefix(kind), highest + 1))
}

/// Validates the story against the roster, sets `updated_at` and writes it.
pub fn save(store: &Store, product: &Product, story: Story) -> Result<Story> {
    let saved = Story {
        archived: None,
        updated_at: clock::iso(clock::now_ms()),
        ..story
    };
    assert_valid(product, &saved)?;
    let data = serde_json::to_string(&saved).map_err(|e| Error::invalid(e.to_string()))?;
    let (pid, id) = (product.id.clone(), saved.id.clone());
    store.write(move |tx| stories::upsert_story(tx, &pid, &id, &data))?;
    Ok(saved)
}

/// Fails on a story that `save` would refuse. A step that changes git calls it first, so
/// a story it cannot save stops before the change.
pub fn assert_valid(product: &Product, story: &Story) -> Result<()> {
    let failures = record::validate(story, &product.roles);
    if failures.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = failures.iter().map(|f| format!("- {f}")).collect();
    Err(Error::invalid(format!(
        "Story {} is invalid:\n{}",
        story.id,
        list.join("\n")
    )))
}

pub fn append_log(store: &Store, product: &str, id: &str, text: &str) -> Result<()> {
    let (p, i, t) = (product.to_string(), id.to_string(), text.to_string());
    store.write(move |tx| stories::append_story_log(tx, &p, &i, &t))?;
    Ok(())
}

pub fn read_log(c: &Connection, product: &str, id: &str) -> Result<String> {
    Ok(read::story_log(c, product, id)?.unwrap_or_default())
}

static LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The lock of the story transactions of one product, in this process.
pub fn product_lock(product: &str) -> Arc<Mutex<()>> {
    let mut locks = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    locks.entry(product.to_string()).or_default().clone()
}

/// Runs `f` under the lock of the product.
pub fn transaction<T>(product: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let lock = product_lock(product);
    let _guard: MutexGuard<()> = lock.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Runs `f` on a reader connection.
pub fn read_with<T>(store: &Store, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    store.read(|c| Ok(f(c)))?
}
