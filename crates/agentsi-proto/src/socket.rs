//! Where the core listens.

use std::path::PathBuf;

/// The variable that names the socket. It overrides the default path.
pub const SOCKET_ENV: &str = "AGENTSI_CORE_SOCKET";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketPath {
    pub path: PathBuf,
    /// true: the default path. Then the core owns the folder and sets its
    /// mode to 0700. For a path from `AGENTSI_CORE_SOCKET`, the core makes a
    /// missing folder with mode 0700, but never changes a folder that exists.
    pub default: bool,
}

impl SocketPath {
    /// The lock file of the core, next to the socket. It holds the pid.
    pub fn lock(&self) -> PathBuf {
        self.path.with_extension("lock")
    }

    /// The log of a core that a client started, next to the socket.
    pub fn log(&self) -> PathBuf {
        self.path.with_extension("log")
    }
}

/// The socket path of this user. Linux: `$XDG_RUNTIME_DIR/agentsi/core.sock`.
/// macOS: `$TMPDIR/agentsi/core.sock`.
pub fn socket_path() -> Result<SocketPath, String> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base_var = if cfg!(target_os = "macos") {
        "TMPDIR"
    } else {
        "XDG_RUNTIME_DIR"
    };
    resolve(var(SOCKET_ENV), var(base_var), base_var)
}

fn resolve(
    over: Option<PathBuf>,
    base: Option<PathBuf>,
    base_var: &str,
) -> Result<SocketPath, String> {
    if let Some(path) = over {
        return Ok(SocketPath {
            path,
            default: false,
        });
    }
    match base {
        Some(base) => Ok(SocketPath {
            path: base.join("agentsi").join("core.sock"),
            default: true,
        }),
        None => Err(format!(
            "{base_var} is not set. Set {SOCKET_ENV} to the socket path of the core."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_variable_overrides_the_default_path() {
        let p = resolve(
            Some("/x/s.sock".into()),
            Some("/run/user/1".into()),
            "XDG_RUNTIME_DIR",
        )
        .unwrap();
        assert_eq!(
            p,
            SocketPath {
                path: "/x/s.sock".into(),
                default: false
            }
        );
        assert_eq!(p.lock(), PathBuf::from("/x/s.lock"));
        let p = resolve(None, Some("/run/user/1".into()), "XDG_RUNTIME_DIR").unwrap();
        assert_eq!(
            p,
            SocketPath {
                path: "/run/user/1/agentsi/core.sock".into(),
                default: true
            }
        );
        assert!(
            resolve(None, None, "XDG_RUNTIME_DIR")
                .unwrap_err()
                .contains("AGENTSI_CORE_SOCKET")
        );
    }
}
