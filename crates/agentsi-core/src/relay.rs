//! `agentsi-core connect`: joins stdin and stdout to the socket, for SSH.
//! The bytes pass unchanged, so line mode and framed mode both work.

use agentsi_proto::client::connect_or_start;
use agentsi_proto::socket::SocketPath;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::path::Path;

/// Returns when the core closes the connection. At the end of stdin, the
/// relay closes its half of the socket and still prints the replies.
pub fn relay(socket: &SocketPath, core_bin: &Path) -> io::Result<()> {
    let stream = connect_or_start(socket, core_bin)?;
    let mut to_core = stream.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut to_core);
        let _ = to_core.shutdown(Shutdown::Write);
    });
    let mut from_core = stream;
    let mut out = io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = from_core.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n])?;
        out.flush()?;
    }
}
