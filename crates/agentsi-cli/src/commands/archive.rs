//! `agentsi archive` and `agentsi restore`: hide a minutes, a decision, a
//! story or a whole product from the lists. Nothing is deleted.

use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::marks_api::{ArchiveArchive, ArchiveKind, ArchiveParams, ArchiveRestore};

pub const ARCHIVE_USAGE: &str = "agentsi archive [minutes-id|decision-id|story-id] [--product <id>]  (without an id: the whole product)";
pub const RESTORE_USAGE: &str = "agentsi restore [minutes-id|decision-id|story-id] [--product <id>]  (without an id: the whole product)";

pub fn archive(argv: &[String]) -> Result<i32, Fail> {
    run(argv, true)
}

pub fn restore(argv: &[String]) -> Result<i32, Fail> {
    run(argv, false)
}

/// A decision id is a plain number, a story id is `S001` or `T012`, and any
/// other id names a minutes.
fn kind_of(id: &str) -> ArchiveKind {
    let story = id.len() >= 4
        && matches!(id.as_bytes()[0], b'S' | b'T')
        && id[1..].bytes().all(|b| b.is_ascii_digit());
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        ArchiveKind::Decision
    } else if story {
        ArchiveKind::Story
    } else {
        ArchiveKind::Minutes
    }
}

fn run(argv: &[String], archive: bool) -> Result<i32, Fail> {
    let usage = if archive {
        ARCHIVE_USAGE
    } else {
        RESTORE_USAGE
    };
    let (a, positionals) =
        args::parse_positional(argv, &[("product", Kind::Text)]).map_err(Fail::Error)?;
    if positionals.len() > 1 {
        return Err(Fail::Usage(usage.into()));
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?.product;
    let past = if archive { "Archived" } else { "Restored" };
    let params = |kind, id| ArchiveParams {
        product: product.clone(),
        kind,
        id,
    };
    let Some(id) = positionals.first() else {
        let p = params(ArchiveKind::Product, None);
        if archive {
            core.call::<ArchiveArchive>(&p)?;
        } else {
            core.call::<ArchiveRestore>(&p)?;
        }
        println!("{past} product {product}.");
        return Ok(0);
    };
    let kind = kind_of(id);
    // `Number(id)` of TypeScript: leading zeros go away.
    let sent = if kind == ArchiveKind::Decision {
        id.parse::<u64>()
            .map_or_else(|_| id.clone(), |n| n.to_string())
    } else {
        id.clone()
    };
    let p = params(kind, Some(sent));
    if archive {
        core.call::<ArchiveArchive>(&p)?;
    } else {
        core.call::<ArchiveRestore>(&p)?;
    }
    let label = match kind {
        ArchiveKind::Decision => "decision ",
        ArchiveKind::Story => "story ",
        _ => "",
    };
    println!("{past} {label}{id}.");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_of_an_id_follows_its_shape() {
        assert_eq!(kind_of("12"), ArchiveKind::Decision);
        assert_eq!(kind_of("S001"), ArchiveKind::Story);
        assert_eq!(kind_of("T1234"), ArchiveKind::Story);
        assert_eq!(kind_of("2026-10-09-meeting"), ArchiveKind::Minutes);
        assert_eq!(kind_of("S01"), ArchiveKind::Minutes);
        assert_eq!(kind_of("S0a1"), ArchiveKind::Minutes);
    }
}
