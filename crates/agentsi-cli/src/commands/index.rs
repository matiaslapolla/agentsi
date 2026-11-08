//! `agentsi index`: updates the knowledge base of a product, or of each active product.

use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::agency_api::{ProductWhoami, WhoamiParams};
use agentsi_proto::knowledge_api::{KbIndexParams, KbIndexResult, KnowledgeIndex};

pub const USAGE: &str = "agentsi index [--product <id> | --all] [--force]";

/// `(ms / 1000).toFixed(1)`: a tie rounds up, as JavaScript rounds it.
fn seconds(ms: u64) -> String {
    let tenths = (ms + 50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// One line for a product, as the TypeScript CLI prints it.
pub fn index_line(r: &KbIndexResult) -> String {
    let skipped = r.repos.iter().filter(|x| x.skipped).count();
    let docs: u64 = r.repos.iter().map(|x| x.docs).sum();
    let commits: u64 = r.repos.iter().map(|x| x.commits).sum();
    let issues: u64 = r.repos.iter().map(|x| x.issues.unwrap_or(0)).sum();
    format!(
        "{}: {} repos ({skipped} unchanged), {docs} docs, {commits} commits, {issues} issues, {} minutes, {} memory, {}s",
        r.product,
        r.repos.len(),
        r.minutes,
        r.memory,
        seconds(r.ms)
    )
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let a = args::parse(
        argv,
        &[
            ("product", Kind::Text),
            ("all", Kind::Flag),
            ("force", Kind::Flag),
        ],
    )
    .map_err(Fail::Error)?;
    let mut core = Core::connect()?;
    let force = a.flag("force");
    let products: Vec<String> = if a.flag("all") {
        core.call::<ProductWhoami>(&WhoamiParams {
            list: true,
            ..WhoamiParams::default()
        })?
        .products
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.status == "active")
        .map(|p| p.id)
        .collect()
    } else {
        vec![resolve_product(&mut core, a.text("product"))?.product]
    };
    for product in products {
        let result = core.call::<KnowledgeIndex>(&KbIndexParams { product, force })?;
        println!("{}", index_line(&result));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::seconds;

    #[test]
    fn seconds_round_a_tie_up() {
        assert_eq!(seconds(1250), "1.3");
        assert_eq!(seconds(0), "0.0");
        assert_eq!(seconds(949), "0.9");
        assert_eq!(seconds(950), "1.0");
    }
}
