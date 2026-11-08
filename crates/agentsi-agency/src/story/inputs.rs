//! The definition of an agent, as the prompts of stories mode read it. The other
//! inputs of a prompt are in `prompt_inputs`.

use super::{Error, Result};
use crate::roles::files::RoleFiles;
use crate::roles::load::load_role;
use crate::roster;
use agentsi_proto::roles_api::RoleDef;
use rusqlite::Connection;
use std::path::Path;

pub use crate::prompt_inputs::{brief_xml, compact_snapshot, memory_for_prompt, objective_ids};

/// The resolved definition of one agent of the product.
pub fn role(c: &Connection, root: &Path, product: &str, agent: &str) -> Result<RoleDef> {
    let overlays = roster::read_overlays(c, product)?;
    let mut warnings = Vec::new();
    load_role(
        agent,
        product,
        &RoleFiles::at(root),
        &overlays,
        &mut warnings,
    )
    .map_err(Error::invalid)
}
