//! The desk session: the one `claude -p` process of the core for the front desk of a product.
//! This module builds its arguments, runs it and reads its stream. The core keeps one process
//! for each product. Spec: `docs/specs/agentsi-desktop-conversation.md`.

pub mod args;
pub mod auth;
pub mod process;
pub mod stream;

use std::io::Read;

/// A new random session uuid (version 4), as `--session-id` takes it.
pub fn new_session_id() -> std::io::Result<String> {
    let mut b = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// True when the stderr of a process that stopped says that Claude Code lost the stored
/// session, or that its id is in use.
pub fn session_lost(stderr: &str) -> bool {
    let t = stderr.to_lowercase();
    t.contains("no conversation found") || t.contains("already in use")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_id_is_a_version_4_uuid() {
        let a = new_session_id().unwrap();
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
        assert_eq!(&a[14..15], "4");
        assert!("89ab".contains(&a[19..20]), "{a}");
        assert_ne!(a, new_session_id().unwrap());
    }

    #[test]
    fn a_lost_session_reads_from_stderr() {
        assert!(session_lost(
            "Error: No conversation found with session ID: 1b4e"
        ));
        assert!(session_lost("Error: Session ID 1b4e is already in use."));
        assert!(!session_lost("boom"));
    }
}
