//! Platform seam for killing the `az` subprocess tree.
//! `az` is a Python wrapper that can fork children holding the port, so on Unix
//! we kill the whole process group. Windows kill-by-port is a stub for the
//! later Windows port (see spec).

#[cfg(unix)]
pub fn kill_process_group(pid: u32) {
    use nix::sys::signal::{killpg, Signal};
    use nix::unistd::Pid;
    // Negative pgid semantics handled by killpg; ignore errors (process may be gone).
    let pgid = Pid::from_raw(pid as i32);
    let _ = killpg(pgid, Signal::SIGTERM);
    let _ = killpg(pgid, Signal::SIGKILL);
}

#[cfg(windows)]
pub fn kill_process_group(_pid: u32) {
    // TODO(windows port): replicate Go netstat/taskkill kill-by-port.
}
