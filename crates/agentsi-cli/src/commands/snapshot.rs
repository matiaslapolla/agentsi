use super::{Fail, product_only, resolve_product};
use crate::core::Core;
use agentsi_proto::agency_api::{DataSnapshot, SnapshotParams};
use std::io::Write;

pub const USAGE: &str = "agentsi snapshot [--product <id>]";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let flag = product_only(argv)?;
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, flag.as_deref())?;
    let r = core.call::<DataSnapshot>(&SnapshotParams {
        product: product.product,
        scope: product.scope,
    })?;
    let _ = std::io::stdout().write_all(r.markdown.as_bytes());
    Ok(0)
}
