//! Platform seam for killing the `az` subprocess tree.
//! `az` is a Python wrapper that can fork children holding the port, so we must
//! kill the whole tree — not just the direct child — to free the local port.
//! On Unix we kill the process group; on Windows we kill the PID tree.

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
pub fn kill_process_group(pid: u32) {
    // `pid` is the `cmd.exe` we spawned (see `az_command`); `/T` kills its whole
    // tree (cmd → az → python → tunnel) and `/F` forces it, freeing the port.
    // Runs synchronously in `stop()` before the monitor task drops the Child, so
    // the tree is still intact when taskkill walks it. Errors are ignored — the
    // process may already be gone.
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output();
}
