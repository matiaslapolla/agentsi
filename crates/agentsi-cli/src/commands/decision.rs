//! `agentsi decision add`: a pending decision outside a meeting.

use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::marks_api::{DecisionAdd, DecisionAddParams};

pub const USAGE: &str = "agentsi decision add --text \"<text>\" --why \"<why>\" [--action \"<what to run once approved>\"] [--product <id>]";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (sub, rest) = argv
        .split_first()
        .map_or((None, &[][..]), |(s, r)| (Some(s), r));
    let (a, positionals) = args::parse_positional(
        rest,
        &[
            ("product", Kind::Text),
            ("text", Kind::Text),
            ("why", Kind::Text),
            ("action", Kind::Text),
        ],
    )
    .map_err(Fail::Error)?;
    let text = a.text("text").map(str::trim).unwrap_or_default();
    let why = a.text("why").map(str::trim).unwrap_or_default();
    if sub.map(String::as_str) != Some("add")
        || !positionals.is_empty()
        || text.is_empty()
        || why.is_empty()
    {
        return Err(Fail::Usage(USAGE.into()));
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?.product;
    let added = core.call::<DecisionAdd>(&DecisionAddParams {
        product: product.clone(),
        text: text.into(),
        why: why.into(),
        action: a.text("action").map(|s| s.trim().to_string()),
    })?;
    println!("Decision {} pending in {product}.", added.id);
    Ok(0)
}
