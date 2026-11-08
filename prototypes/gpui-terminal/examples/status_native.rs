//! Does libghostty-vt parse OSC 7501 itself and answer the `?` query? Uses the raw FFI of
//! `ghostty-vt-sys` because the safe `ghostty-vt` 0.8.0 does not expose the program-status option.
use ghostty_vt::ffi;
use std::ffi::c_void;

#[derive(Default)]
struct Sink {
    pty: Vec<u8>,
    reports: Vec<String>,
}

unsafe extern "C" fn on_pty(_: ffi::Terminal, ud: *mut c_void, data: *const u8, len: usize) {
    let s = unsafe { &mut *ud.cast::<Sink>() };
    s.pty.extend_from_slice(unsafe { std::slice::from_raw_parts(data, len) });
}

unsafe extern "C" fn on_status(_: ffi::Terminal, ud: *mut c_void, r: *const ffi::TerminalProgramStatus) {
    let s = unsafe { &mut *ud.cast::<Sink>() };
    let r = unsafe { &*r };
    let text = |x: &ffi::String| String::from_utf8_lossy(unsafe { x.to_bytes() }).into_owned();
    s.reports.push(format!(
        "state={} kind={} progress={} id={:?} app={:?} title={:?} message={:?}",
        r.state, r.kind, r.progress, text(&r.id), text(&r.app), text(&r.title), text(&r.message)
    ));
}

fn main() {
    let mut sink = Box::<Sink>::default();
    let mut t: ffi::Terminal = std::ptr::null_mut();
    unsafe {
        assert_eq!(ffi::ghostty_terminal_new(std::ptr::null(), &mut t, 80, 24), 0);
        let ud = std::ptr::from_mut(&mut *sink).cast::<c_void>();
        ffi::ghostty_terminal_set(t, ffi::TerminalOption::USERDATA, ud);
        ffi::ghostty_terminal_set(t, ffi::TerminalOption::WRITE_PTY, on_pty as *const c_void);
        let w = |b: &[u8]| ffi::ghostty_terminal_vt_write(t, b.as_ptr(), b.len());

        println!("-- without the program-status option");
        w(b"\x1b]7501;?\x1b\\");
        println!("reply to the query: {:?}", String::from_utf8_lossy(&sink.pty));
        w(b"\x1b]7501;state=working:app=claude-code\x1b\\");
        println!("reports: {:?}", sink.reports);

        println!("-- with the program-status option");
        sink.pty.clear();
        ffi::ghostty_terminal_set(t, ffi::TerminalOption::PROGRAM_STATUS, on_status as *const c_void);
        w(b"\x1b]7501;?\x1b\\");
        println!("reply to the query: {:?}", String::from_utf8_lossy(&sink.pty));
        w(b"\x1b]7501;state=working:app=claude-code\x1b\\");
        w(b"\x1b]7501;state=blocked:app=claude-code:kind=permission:msg=YXBwcm92ZSBCYXNoOiB0b3VjaCAvdG1wL3AzLXByb2JlLWZpbGU=\x1b\\");
        w(b"\x1b]7501;state=idle:app=claude-code\x07");
        for r in &sink.reports {
            println!("report: {r}");
        }
        ffi::ghostty_terminal_free(t);
    }
}
