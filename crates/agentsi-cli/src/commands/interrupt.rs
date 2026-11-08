//! `Ctrl+C` during `meet`, `ask` and `reflect`. The runs of `claude` are children of the core,
//! so the signal of the terminal does not reach them. The command tells the core
//! to stop them, because each run costs money.

use crate::core::Core;
use agentsi_proto::meeting_api::{MeetingCancel, MeetingCancelParams};
use agentsi_proto::reflect_api::{ReflectCancel, ReflectCancelParams};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The exit code of a process that SIGINT stopped.
pub const EXIT_CODE: i32 = 130;

static SIGNALED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    SIGNALED.store(true, Ordering::SeqCst);
}

/// A name for the events of one run of this command.
pub fn name() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("cli-{}-{now}", std::process::id())
}

pub fn happened() -> bool {
    SIGNALED.load(Ordering::SeqCst)
}

/// From now on, SIGINT and SIGTERM run `cancel` on a new connection to the core.
fn on_signal_call(cancel: impl FnOnce(&mut Core) + Send + 'static) {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    // SAFETY: the handler only stores into an atomic, which a signal handler may do.
    unsafe {
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
    std::thread::spawn(move || {
        while !happened() {
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Ok(mut core) = Core::connect() {
            cancel(&mut core);
        }
    });
}

/// From now on, SIGINT and SIGTERM cancel the meeting `name`.
pub fn cancel_on_signal(name: &str) {
    let meeting = name.to_string();
    on_signal_call(move |core| {
        let _ = core.call::<MeetingCancel>(&MeetingCancelParams { meeting });
    });
}

/// From now on, SIGINT and SIGTERM cancel the reflection of `product`.
pub fn cancel_reflection_on_signal(product: &str) {
    let product = product.to_string();
    on_signal_call(move |core| {
        let _ = core.call::<ReflectCancel>(&ReflectCancelParams { product });
    });
}
