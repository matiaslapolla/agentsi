use super::Fail;
use crate::args::{self, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::agency_api::{ProductWhoami, WhoamiParams};
use std::path::PathBuf;

pub const USAGE: &str = "agentsi whoami [--cwd <dir>] [--list] [--home]";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let a = args::parse(
        argv,
        &[
            ("cwd", Kind::Text),
            ("list", Kind::Flag),
            ("home", Kind::Flag),
        ],
    )
    .map_err(Fail::Error)?;
    let mut core = Core::connect()?;
    if a.flag("home") {
        let r = core.call::<ProductWhoami>(&WhoamiParams {
            home: true,
            ..WhoamiParams::default()
        })?;
        let home = r
            .home
            .ok_or_else(|| "the core gave no data root".to_string())?;
        println!("{}  ({})", home.root, home.from);
        return Ok(0);
    }
    if a.flag("list") {
        let r = core.call::<ProductWhoami>(&WhoamiParams {
            list: true,
            ..WhoamiParams::default()
        })?;
        for p in r.products.unwrap_or_default() {
            println!(
                "{}  {}  {}  {}{}",
                p.id,
                p.name,
                p.status,
                p.paths.join(","),
                if p.archived { "  (archived)" } else { "" }
            );
        }
        return Ok(0);
    }
    let cwd = match a.text("cwd") {
        Some(dir) => PathBuf::from(dir),
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    let cwd = std::path::absolute(&cwd).map_err(|e| e.to_string())?;
    let r = core.call::<ProductWhoami>(&WhoamiParams {
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..WhoamiParams::default()
    })?;
    match r.product {
        Some(identity) => {
            println!("{}", json::pretty(&identity));
            Ok(0)
        }
        None => Ok(2),
    }
}
