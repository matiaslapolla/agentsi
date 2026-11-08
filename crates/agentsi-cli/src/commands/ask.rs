//! `agentsi ask`: asks one role one question. The answer goes to stdout, and the
//! usage line to stderr.

use super::meet::fixed;
use super::{Fail, interrupt, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::meeting_api::{MeetingAsk, MeetingAskParams, MeetingUsage};

pub const USAGE: &str = "agentsi ask [--product <id>] --role <role> --question \"<q>\"";

fn usage_line(u: &MeetingUsage) -> String {
    let total = u.input_tokens + u.cache_read + u.cache_write;
    format!(
        "\n[{} · {} · {}s · in {total} (fresh {}, cache read {}, cache write {}) · out {} · ${}]",
        u.role,
        u.model,
        fixed(u.duration_ms as f64 / 1000.0, 1),
        u.input_tokens,
        u.cache_read,
        u.cache_write,
        u.output_tokens,
        fixed(u.cost_usd, 4)
    )
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let a = args::parse(
        argv,
        &[
            ("product", Kind::Text),
            ("role", Kind::Text),
            ("question", Kind::Text),
        ],
    )
    .map_err(Fail::Error)?;
    let Some(role) = a.text("role").filter(|r| !r.is_empty()) else {
        return Err(Fail::Usage("Missing --role".into()));
    };
    let Some(question) = a.text("question").filter(|q| !q.is_empty()) else {
        return Err(Fail::Usage("Missing --question".into()));
    };
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let name = interrupt::name();
    interrupt::cancel_on_signal(&name);
    let reply = core.call::<MeetingAsk>(&MeetingAskParams {
        product: product.product,
        scope: product.scope,
        role: role.into(),
        question: question.into(),
        meeting: Some(name),
    });
    if interrupt::happened() {
        return Ok(interrupt::EXIT_CODE);
    }
    let r = reply?;
    for w in &r.warnings {
        eprintln!("{w}");
    }
    println!("{}", r.answer);
    eprintln!("{}", usage_line(&r.usage));
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usage_line_is_that_of_the_typescript_cli() {
        let u = MeetingUsage {
            role: "dev".into(),
            model: "sonnet".into(),
            input_tokens: 1,
            output_tokens: 2,
            cache_read: 3,
            cache_write: 4,
            cost_usd: 0.03125,
            duration_ms: 1250,
        };
        assert_eq!(
            usage_line(&u),
            "\n[dev · sonnet · 1.3s · in 8 (fresh 1, cache read 3, cache write 4) · out 2 · $0.0313]"
        );
    }
}
