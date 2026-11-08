//! `agentsi up` and `agentsi migrate`. The core applies the migrations when it
//! opens the database, so both commands start the core (if none runs) and
//! print what it reports. They keep the names of the TypeScript commands.

use super::Fail;
use crate::args;
use crate::core::Core;
use agentsi_proto::core_api::Empty;
use agentsi_proto::ops_api::{CoreStatus, CoreStatusResult};

pub const UP_USAGE: &str = "agentsi up";
pub const MIGRATE_USAGE: &str = "agentsi migrate";

fn status() -> Result<CoreStatusResult, Fail> {
    Ok(Core::connect()?.call::<CoreStatus>(&Empty {})?)
}

/// The line about the database.
pub fn database_line(s: &CoreStatusResult) -> String {
    match &s.database {
        None => "No database yet. The core creates it when it first needs it.".into(),
        Some(d) if d.schema_version == d.latest => {
            format!(
                "Database up to date, at schema version {}.",
                d.schema_version
            )
        }
        Some(d) => format!(
            "Database at schema version {}, the core knows {}.",
            d.schema_version, d.latest
        ),
    }
}

pub fn up(argv: &[String]) -> Result<i32, Fail> {
    args::parse(argv, &[]).map_err(Fail::Error)?;
    let s = status()?;
    println!("Core running: agentsi-core {}.", s.version);
    println!("Data root: {}", s.data_root);
    println!("{}", database_line(&s));
    Ok(0)
}

pub fn migrate(argv: &[String]) -> Result<i32, Fail> {
    args::parse(argv, &[]).map_err(Fail::Error)?;
    println!("{}", database_line(&status()?));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::ops_api::DatabaseStatus;

    fn s(db: Option<(u32, u32)>) -> CoreStatusResult {
        CoreStatusResult {
            version: "0.1.0".into(),
            data_root: "/data".into(),
            database: db.map(|(schema_version, latest)| DatabaseStatus {
                path: "/data/agentsi.db".into(),
                schema_version,
                latest,
            }),
            pill: false,
        }
    }

    #[test]
    fn the_line_names_the_schema_version() {
        assert_eq!(
            database_line(&s(Some((3, 3)))),
            "Database up to date, at schema version 3."
        );
        assert!(database_line(&s(Some((2, 3)))).contains("version 2, the core knows 3"));
        assert!(database_line(&s(None)).starts_with("No database yet"));
    }
}
