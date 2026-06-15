//! Cross-platform child container that survives parent death (R16).
//!
//! Unix: each child gets its own process group (`setpgid`) for `killpg` on
//! graceful exit, PLUS `prctl(PR_SET_PDEATHSIG, SIGKILL)` (Linux) and a
//! death-pipe (the parent holds the write end; the child watches the read end
//! and self-terminates on EOF — the macOS backstop). So a SIGKILL of
//! `poverty-mode` reaps the children. Windows: a Job Object with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; children are created SUSPENDED, assigned
//! to the job, then resumed (closing the spawn->assign race), and the OS kills
//! them when the job handle closes. Either way: no orphans.

use std::path::Path;

/// A child spawned into the group: its pid and (taken) stdout reader.
///
/// The reader is boxed (rather than `tokio::process::ChildStdout`) so both
/// platforms share one type: `ChildStdout` has no public constructor, and the
/// Windows path reads the parent end of a raw pipe via `tokio::fs::File`. The
/// READY reader (M6.5 `read_ready_line`) is generic over `AsyncBufRead`, so a
/// caller wraps this in `tokio::io::BufReader::new(...)` on both platforms.
pub struct GroupSpawn {
    pub pid: u32,
    pub stdout: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>,
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::process::Stdio;
    use tokio::process::{Child, Command};

    /// Owns the death-pipe ends (held open for the parent's life) and the spawned
    /// children (each its own process-group leader).
    pub struct ProxyGroup {
        /// Write end of the death-pipe, held SOLELY by the parent (R23h). When the
        /// parent dies the OS closes it -> children watching the read end see EOF
        /// and self-terminate. Dropped on graceful exit via RAII.
        _death_write: OwnedFd,
        /// Read end, kept open by the parent so each spawned child can inherit it.
        /// The parent never reads it; closing it on drop avoids an fd leak.
        _death_read: OwnedFd,
        /// Raw fd number of the read end, passed to children via env so they can
        /// watch it. Children inherit the fd across exec (CLOEXEC is cleared
        /// per-child in `pre_exec`).
        death_read_fd: i32,
        children: Vec<Child>,
        pgids: Vec<i32>,
    }

    impl ProxyGroup {
        pub fn new() -> anyhow::Result<ProxyGroup> {
            // Portable `pipe()` (macOS has no `pipe2`), then set FD_CLOEXEC on BOTH
            // ends so neither leaks into unrelated children; per-child we clear
            // CLOEXEC on the read end in `pre_exec` so only the spawned child
            // inherits it. Both ends are wrapped in `OwnedFd` immediately so they
            // are closed on any early return / drop (RAII, no fd leak).
            let mut fds = [0i32; 2];
            // SAFETY: fds is a valid 2-int array; pipe writes both fds.
            let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
            if rc != 0 {
                return Err(anyhow::anyhow!(
                    "creating death-pipe: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let death_read = OwnedFd::from_raw_fd_checked(fds[0])?;
            let death_write = OwnedFd::from_raw_fd_checked(fds[1])?;
            set_cloexec(&death_read)?;
            set_cloexec(&death_write)?;
            let death_read_fd = death_read.as_raw_fd();
            Ok(ProxyGroup {
                _death_read: death_read,
                _death_write: death_write,
                death_read_fd,
                children: Vec::new(),
                pgids: Vec::new(),
            })
        }

        pub fn spawn(
            &mut self,
            exe: &Path,
            args: &[String],
            extra_env: &[(String, String)],
        ) -> anyhow::Result<GroupSpawn> {
            let mut cmd = Command::new(exe);
            cmd.args(args);
            cmd.stdout(Stdio::piped());
            // Tell the child which fd is the death-pipe read end to watch.
            cmd.env("PM_DEATH_PIPE_FD", self.death_read_fd.to_string());
            // Per-child env overrides (e.g. test-only fault injection): applied to
            // THIS child's Command only, never the parent's global env (no UB).
            for (k, v) in extra_env {
                cmd.env(k, v);
            }

            let read_fd = self.death_read_fd;
            // Test-only knob: when PM_DISABLE_PDEATHSIG=1 is in THIS process's env,
            // skip arming PR_SET_PDEATHSIG so the os-reaps test can force the
            // death-pipe-only backstop path (the macOS path) on a Linux/CI host.
            // Inert in production (never set outside tests). Read once on the parent
            // side and captured by the closure (env reads are not async-signal-safe).
            let disable_pdeathsig = std::env::var("PM_DISABLE_PDEATHSIG").as_deref() == Ok("1");
            // SAFETY: pre_exec runs in the forked child before exec. It calls only
            // async-signal-safe libc functions (setpgid/prctl/getppid/fcntl/_exit).
            // The captured `disable_pdeathsig` bool was computed in the parent.
            unsafe {
                cmd.pre_exec(move || {
                    // `disable_pdeathsig` is only read inside the Linux-only block
                    // below; mark it used on non-Linux unix (e.g. macOS) so the
                    // captured binding does not trip `unused_variables`.
                    #[cfg(not(target_os = "linux"))]
                    let _ = disable_pdeathsig;
                    // Own process group for killpg (child side; the parent ALSO
                    // calls setpgid post-spawn to close the fork race — see below).
                    if libc::setpgid(0, 0) != 0 {
                        // EACCES means the child already exec'd; treat as benign.
                        let err = std::io::Error::last_os_error();
                        if err.raw_os_error() != Some(libc::EACCES) {
                            return Err(err);
                        }
                    }
                    // Linux: die when the parent thread dies (unless disabled for the
                    // death-pipe-only test).
                    #[cfg(target_os = "linux")]
                    if !disable_pdeathsig {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        // Race: if the parent died between fork and prctl, we are
                        // already orphaned (ppid == 1) and PDEATHSIG will never
                        // fire. Detect and exit.
                        if libc::getppid() == 1 {
                            libc::_exit(0);
                        }
                    }
                    // Clear CLOEXEC on the inherited death-pipe read end so the
                    // watcher (spawned after exec) can read it. The exec'd binary
                    // reads PM_DEATH_PIPE_FD and watches it.
                    let flags = libc::fcntl(read_fd, libc::F_GETFD);
                    if flags >= 0 {
                        let _ = libc::fcntl(read_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
                    }
                    Ok(())
                });
            }

            let mut child = cmd
                .spawn()
                .map_err(|e| anyhow::anyhow!("spawning grouped child: {e}"))?;
            let pid = child.id().ok_or_else(|| anyhow::anyhow!("spawned child has no pid"))?;
            let stdout = child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>);
            // Close the setpgid fork race from the PARENT side too (POSIX: both the
            // parent and child should call setpgid). If the child has not yet run
            // its own setpgid, this establishes the group; if it already did (or
            // already exec'd), the call returns EACCES/ESRCH which we ignore. After
            // this the child's pgid == its pid, so `killpg(-pid)` always targets it.
            // SAFETY: setpgid is always safe to call; benign errnos are ignored.
            unsafe {
                libc::setpgid(pid as libc::pid_t, pid as libc::pid_t);
            }
            self.pgids.push(pid as i32);
            self.children.push(child);
            Ok(GroupSpawn { pid, stdout })
        }

        /// killpg(SIGKILL) every child process group. Idempotent / best-effort.
        pub fn kill_all(&mut self) -> anyhow::Result<()> {
            for pgid in &self.pgids {
                // SAFETY: kill is always safe to call; ESRCH (already gone) is
                // ignored. A negative pid in `kill` targets the process group.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
            Ok(())
        }

        pub async fn wait_all_exited(&mut self) -> anyhow::Result<()> {
            for child in self.children.iter_mut() {
                let _ = child.wait().await;
            }
            Ok(())
        }

        pub async fn drop_and_wait(mut self) -> anyhow::Result<()> {
            self.kill_all()?;
            self.wait_all_exited().await
        }
    }

    impl Drop for ProxyGroup {
        fn drop(&mut self) {
            let _ = self.kill_all();
            // Closing `_death_write` here (on normal drop) also signals any child
            // we did not retain; the explicit killpg above is the primary path.
        }
    }

    /// Set `FD_CLOEXEC` on an fd so it is NOT inherited by children (the read end
    /// re-clears this per-child in `pre_exec`; the write end keeps it forever so
    /// no child can hold the write side open and defeat the EOF signal, R23h).
    fn set_cloexec(fd: &OwnedFd) -> anyhow::Result<()> {
        let raw = fd.as_raw_fd();
        // SAFETY: raw is a live, owned fd for the duration of these calls.
        let flags = unsafe { libc::fcntl(raw, libc::F_GETFD) };
        if flags < 0 {
            return Err(anyhow::anyhow!(
                "fcntl(F_GETFD) on death-pipe fd: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: raw is a live, owned fd.
        if unsafe { libc::fcntl(raw, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(anyhow::anyhow!(
                "fcntl(F_SETFD, FD_CLOEXEC) on death-pipe fd: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(())
    }

    /// `OwnedFd::from_raw_fd` is unsafe and infallible; wrap it with a sanity
    /// check so an invalid fd is a clear error rather than UB later.
    trait OwnedFdExt: Sized {
        fn from_raw_fd_checked(fd: i32) -> anyhow::Result<Self>;
    }
    impl OwnedFdExt for OwnedFd {
        fn from_raw_fd_checked(fd: i32) -> anyhow::Result<Self> {
            if fd < 0 {
                return Err(anyhow::anyhow!("invalid fd {fd} for a death-pipe end"));
            }
            // SAFETY: fd came from `pipe()` and is owned exclusively by us.
            Ok(unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) })
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{
        CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, TRUE,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Pipes::CreatePipe;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, ResumeThread, TerminateProcess, WaitForSingleObject, CREATE_SUSPENDED,
        CREATE_UNICODE_ENVIRONMENT, INFINITE, PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW,
    };

    struct OwnedChild {
        process: OwnedHandle,
        /// Kept alive only so the suspended primary thread handle is owned and
        /// closed on drop (it is no longer used after `ResumeThread`).
        _thread: OwnedHandle,
        #[allow(dead_code)]
        pid: u32,
    }

    /// Owns the kill-on-job-close Job Object + the suspended-then-resumed
    /// children. Dropping the Job handle terminates every assigned process.
    pub struct ProxyGroup {
        job: OwnedHandle,
        children: Vec<OwnedChild>,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    /// Build a double-null-terminated UTF-16 environment block: the parent env
    /// (via `std::env::vars()`) with `extra` overrides applied. Used only when a
    /// per-child env override is requested (else `spawn` passes `null` to inherit).
    fn build_env_block(extra: &[(String, String)]) -> Vec<u16> {
        use std::collections::BTreeMap;
        let mut map: BTreeMap<String, String> = std::env::vars().collect();
        for (k, v) in extra {
            map.insert(k.clone(), v.clone());
        }
        let mut block: Vec<u16> = Vec::new();
        for (k, v) in map {
            let entry = format!("{k}={v}");
            block.extend(entry.encode_utf16());
            block.push(0);
        }
        block.push(0); // final (double-null) terminator
        block
    }

    /// Terminate a CREATE_SUSPENDED child that never made it into the running set
    /// (job-assign or resume failed), then wait for the OS to finish reaping it so
    /// the caller's early `return Err(...)` cannot leave a suspended orphan. The
    /// child is suspended (runs no code), so termination resolves promptly; the
    /// wait awaits an OS-completed event we just requested, not a chosen timeout.
    fn terminate_suspended_orphan(proc: HANDLE) {
        // SAFETY: `proc` is a live process handle still owned by the caller's
        // `OwnedHandle` for the duration of this call; terminating a suspended
        // child is well-defined. Best-effort — we are already on an error path.
        unsafe {
            TerminateProcess(proc, 1);
            // Block until the process is actually gone before the caller drops
            // (closes) its handle, so no suspended orphan survives the return.
            WaitForSingleObject(proc, INFINITE);
        }
    }

    impl ProxyGroup {
        pub fn new() -> anyhow::Result<ProxyGroup> {
            // SAFETY: CreateJobObjectW with null attrs/name returns a job handle.
            let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if raw.is_null() {
                return Err(anyhow::anyhow!(
                    "CreateJobObjectW failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: raw is a freshly created, owned job handle.
            let job = unsafe { OwnedHandle::from_raw_handle(raw as *mut _) };

            // SAFETY: zeroed is a valid initial state for this C struct.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: raw is a valid job handle; info is a valid, sized struct.
            let ok = unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if ok == 0 {
                return Err(anyhow::anyhow!(
                    "SetInformationJobObject(KILL_ON_JOB_CLOSE) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(ProxyGroup {
                job,
                children: Vec::new(),
            })
        }

        pub fn spawn(
            &mut self,
            exe: &Path,
            args: &[String],
            extra_env: &[(String, String)],
        ) -> anyhow::Result<GroupSpawn> {
            // Build an inheritable stdout pipe for the READY read.
            // SAFETY: zeroed is a valid initial state for SECURITY_ATTRIBUTES.
            let mut sa: SECURITY_ATTRIBUTES = unsafe { std::mem::zeroed() };
            sa.nLength = std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32;
            sa.bInheritHandle = TRUE;
            let mut read_h: HANDLE = std::ptr::null_mut();
            let mut write_h: HANDLE = std::ptr::null_mut();
            // SAFETY: CreatePipe writes two handles; sa makes them inheritable.
            if unsafe { CreatePipe(&mut read_h, &mut write_h, &sa, 0) } == 0 {
                return Err(anyhow::anyhow!(
                    "CreatePipe failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // The READ end must NOT be inheritable (parent keeps it).
            // SAFETY: read_h is a valid handle from CreatePipe.
            unsafe {
                SetHandleInformation(read_h, HANDLE_FLAG_INHERIT, 0);
            }
            // Own the read end now; the write end is given to the child and closed
            // in the parent after CreateProcessW.
            // SAFETY: read_h is a freshly created, owned handle.
            let read_owned = unsafe { OwnedHandle::from_raw_handle(read_h as *mut _) };

            // Command line: "exe" "arg1" "arg2" ... (quote each token).
            let mut cmdline = String::new();
            cmdline.push('"');
            cmdline.push_str(&exe.to_string_lossy());
            cmdline.push('"');
            for a in args {
                cmdline.push(' ');
                cmdline.push('"');
                cmdline.push_str(a);
                cmdline.push('"');
            }
            let mut cmdline_w = to_wide(&cmdline);

            // SAFETY: zeroed is a valid initial state for STARTUPINFOW.
            let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
            si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
            si.dwFlags = STARTF_USESTDHANDLES;
            si.hStdOutput = write_h;
            // Inherit stderr so the parent terminal stays informative.
            // SAFETY: GetStdHandle returns a process-owned handle.
            si.hStdError = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
            si.hStdInput = INVALID_HANDLE_VALUE;

            // SAFETY: zeroed is a valid initial state for PROCESS_INFORMATION.
            let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

            // Conditional env block: empty -> inherit parent env (null); non-empty ->
            // a custom UTF-16 block with the overrides applied. `env_block` is kept
            // alive for the whole CreateProcessW call (env_ptr borrows it).
            let env_block: Vec<u16>;
            let (creation_flags, env_ptr): (u32, *const std::ffi::c_void) = if extra_env.is_empty() {
                (CREATE_SUSPENDED, std::ptr::null())
            } else {
                env_block = build_env_block(extra_env);
                (
                    CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT,
                    env_block.as_ptr() as *const std::ffi::c_void,
                )
            };

            // SAFETY: standard CreateProcessW call; bInheritHandles=TRUE so the child
            // inherits the write end of the pipe. CREATE_SUSPENDED means the child runs
            // no code until we ResumeThread (after assigning the job). `env_ptr` is
            // either null (inherit) or points into the live `env_block`.
            let ok = unsafe {
                CreateProcessW(
                    std::ptr::null(),
                    cmdline_w.as_mut_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    TRUE,
                    creation_flags,
                    env_ptr,
                    std::ptr::null(),
                    &si,
                    &mut pi,
                )
            };
            // Parent no longer needs the child's write end.
            // SAFETY: write_h is a valid handle we own; closing it once is correct.
            unsafe {
                CloseHandle(write_h);
            }
            if ok == 0 {
                return Err(anyhow::anyhow!(
                    "CreateProcessW(CREATE_SUSPENDED) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: pi.hProcess / pi.hThread are freshly created, owned handles.
            let process = unsafe { OwnedHandle::from_raw_handle(pi.hProcess as *mut _) };
            let thread = unsafe { OwnedHandle::from_raw_handle(pi.hThread as *mut _) };
            let pid = pi.dwProcessId;

            // Assign to the job BEFORE the child runs (race closed). If this fails
            // the child is CREATE_SUSPENDED and NOT in any job: dropping `process`
            // only closes the handle, leaving a suspended ORPHAN that never runs and
            // never dies. Terminate it before returning so it cannot leak.
            let job_raw = self.job.as_raw_handle();
            let proc_raw = process.as_raw_handle();
            // Test-only fault injection: force the assign step to be treated as
            // failed so the orphan-prevention path is exercised hermetically. The
            // knob rides the per-child `extra_env` slice (NOT the process's global
            // env), so a test sets it without any env-mutation race. Inert in
            // production (never set outside tests).
            let force_assign_fail = extra_env
                .iter()
                .any(|(k, v)| k == "PM_TEST_FORCE_ASSIGN_FAIL" && v == "1");
            // SAFETY: both are valid handles owned by us for this call.
            let assigned =
                !force_assign_fail && unsafe { AssignProcessToJobObject(job_raw as HANDLE, proc_raw as HANDLE) } != 0;
            if !assigned {
                let err = if force_assign_fail {
                    std::io::Error::other("forced via extra_env PM_TEST_FORCE_ASSIGN_FAIL=1")
                } else {
                    std::io::Error::last_os_error()
                };
                terminate_suspended_orphan(proc_raw as HANDLE);
                return Err(anyhow::anyhow!(
                    "AssignProcessToJobObject failed (terminated suspended child pid {pid}): {err}"
                ));
            }
            // Now let it run. If ResumeThread fails the child is still suspended
            // (it is in the job, but dropping the job handle only triggers
            // kill-on-job-close once EVERY handle to the job closes — and `self`
            // still holds one, so this child would hang suspended until full
            // teardown). Terminate it before returning so it cannot leak.
            // SAFETY: pi.hThread is the suspended primary thread of the child.
            if unsafe { ResumeThread(pi.hThread) } == u32::MAX {
                let err = std::io::Error::last_os_error();
                terminate_suspended_orphan(proc_raw as HANDLE);
                return Err(anyhow::anyhow!(
                    "ResumeThread failed (terminated suspended child pid {pid}): {err}"
                ));
            }

            // Wrap the parent's read end as an async reader for the READY read.
            let std_file = std::fs::File::from(read_owned);
            let tokio_file = tokio::fs::File::from_std(std_file);
            let stdout: Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>> = Some(Box::new(tokio_file));

            self.children.push(OwnedChild {
                process,
                _thread: thread,
                pid,
            });
            Ok(GroupSpawn { pid, stdout })
        }

        pub fn kill_all(&mut self) -> anyhow::Result<()> {
            // Terminating the job kills every assigned process immediately.
            let job_raw = self.job.as_raw_handle();
            // SAFETY: job is a valid job handle owned by us. Exit code 1.
            let _ = unsafe { TerminateJobObject(job_raw as HANDLE, 1) };
            Ok(())
        }

        pub async fn wait_all_exited(&mut self) -> anyhow::Result<()> {
            // Wait for each process handle on a blocking task (real exit, no timer).
            for c in self.children.iter() {
                // HANDLE is a raw pointer (!Send), so carry it as isize across the
                // blocking task boundary and rebuild it inside.
                let h = c.process.as_raw_handle() as isize;
                let _ = tokio::task::spawn_blocking(move || {
                    let handle = h as HANDLE;
                    // SAFETY: handle is a live process handle for the wait duration
                    // (the OwnedHandle stays alive in `self.children` until after
                    // this whole function returns).
                    unsafe {
                        let _ = WaitForSingleObject(handle, INFINITE);
                        let mut code = 0u32;
                        let _ = GetExitCodeProcess(handle, &mut code);
                    }
                })
                .await;
            }
            Ok(())
        }

        pub async fn drop_and_wait(mut self) -> anyhow::Result<()> {
            self.kill_all()?;
            self.wait_all_exited().await
            // Dropping `self.job` after this closes the handle -> kill-on-close.
        }
    }

    impl Drop for ProxyGroup {
        fn drop(&mut self) {
            // Best-effort terminate now; dropping `self.job` (kill-on-job-close)
            // is the backstop if the process is dying with handles still open.
            let _ = self.kill_all();
        }
    }
}

pub use imp::ProxyGroup;

/// Spawn a background thread that self-terminates the process when the
/// death-pipe write end (held by the parent) closes — the macOS backstop and a
/// harmless redundancy on Linux (where `PR_SET_PDEATHSIG` is the primary path).
///
/// Reads `PM_DEATH_PIPE_FD` (set by [`ProxyGroup::spawn`]); a missing/invalid
/// value means this child was not spawned into a group, so this is a no-op.
#[cfg(unix)]
pub fn spawn_death_watcher_from_env() {
    let fd: i32 = match std::env::var("PM_DEATH_PIPE_FD").ok().and_then(|s| s.parse().ok()) {
        Some(fd) => fd,
        None => return,
    };
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        loop {
            // SAFETY: fd is the inherited death-pipe read end.
            let n = unsafe { libc::read(fd, byte.as_mut_ptr() as *mut _, 1) };
            if n == 0 {
                // EOF: the parent (write end holder) died -> self-terminate.
                // SAFETY: _exit is always safe; we deliberately bypass cleanup.
                unsafe { libc::_exit(0) };
            }
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                // SAFETY: _exit is always safe.
                unsafe { libc::_exit(0) };
            }
            // A stray byte (we never write one) — ignore and keep watching.
        }
    });
}

/// Windows no-op: the Job Object kills children when the parent's job handle
/// closes, so there is no death-pipe to watch.
#[cfg(windows)]
pub fn spawn_death_watcher_from_env() {}

#[cfg(test)]
#[path = "teardown_tests.rs"]
mod teardown_tests;
