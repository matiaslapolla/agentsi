//! Helpers for `tests/golden/make-golden.sh`, which writes the golden files with
//! the TypeScript CLI. They run the code that the golden test also runs.
//!
//!   golden_tool prepare <work>            make the work folder
//!   golden_tool cases <work>              print one line for each step of each case
//!   golden_tool harness <work> <step>     run a harness step (`@registry`, `@commits`, `@tree`)
//!   golden_tool rows <work> <json file>   print the `.rows` file of the rows that Postgres gave
//!   golden_tool render <work> <exit> <stdout file> <stderr file>   print a golden file
//!   golden_tool mcp-cases <work>          print `name<TAB>cwd<TAB>requests file` for each MCP case
//!   golden_tool render-mcp <work> <stdout file>   print the golden file of an MCP session
//!   golden_tool documents <work> <json lines file>   print the golden file of the documents table
//!   golden_tool meeting-rows <work> <json file>   print the rows of `meeting-rows.sql`, as `@meeting-rows` prints them
//!   golden_tool reflect-rows <work> <json file>   print the rows of `reflect-rows.sql`, as `@reflect-rows` prints them
//!   golden_tool empty-db <file>           make an empty database file with the schema

#[path = "../tests/golden/support.rs"]
mod support;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let work = PathBuf::from(args.get(1).expect("the work folder"));
    match args[0].as_str() {
        "empty-db" => {
            agentsi_store::db::open_writer(&work).expect("the database file");
        }
        "prepare" => support::prepare(&work),
        "cases" => {
            // One line for each step: name, shared, fresh or rows, step number, step count, cwd, arguments.
            for c in support::cases() {
                let steps = c.steps(&work);
                for (i, step) in steps.iter().enumerate() {
                    println!(
                        "{}\t{}\t{}\t{}\t{}\t{}",
                        c.name,
                        match (c.fresh, c.rows, c.reflect_rows) {
                            (false, ..) => "shared",
                            (true, _, true) => "reflect-rows",
                            (true, false, _) => "fresh",
                            (true, true, _) => "rows",
                        },
                        i,
                        steps.len(),
                        support::cwd_of(&work, &c.cwd).display(),
                        step.join("\t")
                    );
                }
            }
        }
        "harness" => {
            // A harness step has exit 0 and no stderr, so only its stdout is printed.
            print!("{}", support::harness(&work, &args[2]).1);
        }
        "render" => {
            // golden_tool render <work> <exit> <stdout file> <stderr file> [step text]
            let read = |i: usize| std::fs::read_to_string(&args[i]).unwrap();
            let text = support::golden_text(&work, args[2].parse().unwrap(), &read(3), &read(4));
            match args.get(5) {
                Some(step) => print!("{}", support::step_text(&work, step, &text)),
                None => print!("{text}"),
            }
        }
        "rows" => {
            let json = std::fs::read_to_string(&args[2]).unwrap();
            print!(
                "{}",
                support::rows_text(serde_json::from_str(json.trim()).unwrap())
            );
        }
        "meeting-rows" => {
            let text = std::fs::read_to_string(&args[2]).unwrap();
            print!(
                "{}",
                support::meeting_rows_text(&work, serde_json::from_str(&text).unwrap())
            );
        }
        "reflect-rows" => {
            let text = std::fs::read_to_string(&args[2]).unwrap();
            print!(
                "{}",
                support::reflect_rows_text(&work, serde_json::from_str(&text).unwrap())
            );
        }
        "mcp-cases" => {
            for c in support::mcp_cases() {
                println!(
                    "{}\t{}\t{}",
                    c.name,
                    support::cwd_of(&work, &c.cwd).display(),
                    c.requests.display()
                );
            }
        }
        "render-mcp" => {
            let stdout = std::fs::read_to_string(&args[2]).unwrap();
            print!("{}", support::mcp_golden(&work, &stdout));
        }
        "documents" => {
            let text = std::fs::read_to_string(&args[2]).unwrap();
            let rows = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    let v: serde_json::Value = serde_json::from_str(l).unwrap();
                    let text = |k: &str| v[k].as_str().map(String::from);
                    support::DocRow {
                        product: text("product").unwrap(),
                        scope: text("scope"),
                        kind: text("kind").unwrap(),
                        reference: text("ref").unwrap(),
                        title: text("title"),
                        body: text("body"),
                        updated_at: text("updated_at"),
                    }
                })
                .collect();
            print!("{}", support::documents_golden(&work, rows));
        }
        other => panic!("unknown command {other}"),
    }
}
