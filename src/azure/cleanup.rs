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

/// Bind a freshly-spawned tunnel child to OS-managed cleanup so it (and its
/// descendants) are killed if az-burrow itself dies — including a crash or a
/// force-kill, which the graceful `kill_process_group` path can't catch.
///
/// No-op on Unix: there the child already lives in its own process group, and
/// `kill_process_group` reaps it on shutdown.
#[cfg(unix)]
pub fn register_child(_child: &tokio::process::Child) {}

/// Windows: assign the child to a process-wide Job Object created with
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The job handle is intentionally never
/// closed, so when az-burrow exits for ANY reason the OS closes the last handle
/// and the kernel terminates every process still in the job — guaranteeing the
/// port is freed even on a panic or Task-Manager kill. Children that `cmd.exe`
/// spawns inherit job membership, so the whole `az` tree is covered.
#[cfg(windows)]
pub fn register_child(child: &tokio::process::Child) {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let Some(raw) = child.raw_handle() else {
        return;
    };
    let job = job_handle();
    if job == 0 {
        return;
    }
    // SAFETY: `job` is a valid job handle (or 0, handled above) and `raw` is the
    // live handle of a child we just spawned. Failure (e.g. the child already
    // exited) is non-fatal — the graceful `taskkill` path still covers shutdown.
    unsafe {
        AssignProcessToJobObject(job as HANDLE, raw as HANDLE);
    }
}

/// Lazily create the process-wide kill-on-close Job Object, returning its handle
/// as an `isize` (so it can live in a `Sync` static). Returns 0 on failure.
#[cfg(windows)]
fn job_handle() -> isize {
    use std::ffi::c_void;
    use std::ptr;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<isize> = OnceLock::new();
    *JOB.get_or_init(|| unsafe {
        let job: HANDLE = CreateJobObjectW(ptr::null(), ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job as isize
    })
}
