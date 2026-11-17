//! Where an auto-spawned daemon lives (docs/status.md § When a call auto-spawns, "Where an
//! auto-spawned daemon lives").
//!
//! `setsid` detaches the daemon from this client's session, not from its cgroup. On Linux the
//! daemon then lives in the caller's systemd unit and ends with it: on devstation-nyc-1 a unit
//! restart took the shared engine down because a client in that unit had spawned it. So for the
//! live drawer this client moves the daemon it spawned into a transient user scope of its own.
//! It is best effort: nothing here can fail the spawn.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::Path;

/// Place the daemon this call spawned (`pid`), serving `socket`, in its own scope — when it
/// won the flock, the drawer is the live one, and the account has a user bus. A call that
/// fails is said on stderr; the daemon keeps serving where it is.
pub(crate) fn place_spawned(socket: &Path, pid: u32) {
    #[cfg(target_os = "linux")]
    linux::place(socket, pid);
    #[cfg(not(target_os = "linux"))]
    let _ = (socket, pid);
}

/// The live drawer: the cache root is `$HOME/.cache/meridian`. Any other root is a sandbox
/// whose daemon stays where its caller's teardown can reap it.
fn is_live_drawer(cache_root: &Path, home: Option<&Path>) -> bool {
    let Some(home) = home else { return false };
    let live = home.join(".cache").join("meridian");
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    real(cache_root) == real(&live)
}

/// The transient unit the daemon is placed in: the socket stem says which daemon it is, the
/// pid keeps it unique while an earlier daemon's scope is still being collected.
fn scope_name(socket: &Path, pid: u32) -> String {
    let stem = socket
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    format!("mrd-engine-{stem}-{pid}.scope")
}

/// `busctl` argv for `StartTransientUnit`: the pid, the slice, the slice's memory floor (zero
/// values left unset), and `CollectMode=inactive-or-failed`, so a scope whose daemon was
/// OOM-killed is collected rather than left loaded as failed.
fn scope_call(bus: &Path, unit: &str, pid: u32, floor: (u64, u64)) -> Vec<String> {
    // Each property is its name, its signature, then its value's words (`au` takes its
    // length and its items as separate words).
    let mut props: Vec<Vec<String>> = vec![
        vec!["PIDs".into(), "au".into(), "1".into(), pid.to_string()],
        vec!["Slice".into(), "s".into(), SLICE.into()],
        vec![
            "CollectMode".into(),
            "s".into(),
            "inactive-or-failed".into(),
        ],
        vec![
            "Description".into(),
            "s".into(),
            "mrd engine (placed by the client that spawned it)".into(),
        ],
    ];
    for (name, bytes) in [("MemoryMin", floor.0), ("MemoryLow", floor.1)] {
        if bytes > 0 {
            props.push(vec![name.into(), "t".into(), bytes.to_string()]);
        }
    }
    let mut args: Vec<String> = vec![
        format!("--address=unix:path={}", bus.display()),
        format!("--timeout={}", TIMEOUT.as_secs()),
        "call".into(),
        "org.freedesktop.systemd1".into(),
        "/org/freedesktop/systemd1".into(),
        "org.freedesktop.systemd1.Manager".into(),
        "StartTransientUnit".into(),
        "ssa(sv)a(sa(sv))".into(),
        unit.into(),
        "fail".into(),
        props.len().to_string(),
    ];
    for prop in props {
        args.extend(prop);
    }
    args.push("0".into());
    args
}

/// The slice the scope goes in: the one the host's daemon unit sits in, whose levels carry the
/// engine's memory floor.
const SLICE: &str = "app.slice";

/// Bound on the one bus call a placement makes.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(target_os = "linux")]
mod linux {
    use std::io::Read as _;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    use super::{SLICE, TIMEOUT, is_live_drawer, scope_call, scope_name};

    pub(super) fn place(socket: &Path, pid: u32) {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from);
        let Ok(root) = cache::cache_root() else {
            return;
        };
        if !is_live_drawer(&root, home.as_deref()) || !won_flock(socket, pid) {
            return;
        }
        // SAFETY: getuid(2) cannot fail and touches no memory.
        let uid = unsafe { libc::getuid() };
        // The account's bus, never the caller's XDG_RUNTIME_DIR: a login with a private runtime
        // dir for its own socket shares the account's manager and has no bus of its own.
        let bus = PathBuf::from(format!("/run/user/{uid}/bus"));
        if !bus.exists() {
            return;
        }
        let unit = scope_name(socket, pid);
        if let Err(why) = start_scope(&bus, &unit, pid, slice_floor(uid)) {
            eprintln!(
                "mrd: the daemon this call started (pid {pid}) stays in this process's cgroup \
                 and stops when its unit stops: {why}"
            );
        }
    }

    /// The pidfile names `pid`: the daemon writes it after winning the flock and before its
    /// first answer, so a daemon that answered and is named there is the one this call spawned.
    fn won_flock(socket: &Path, pid: u32) -> bool {
        std::fs::read_to_string(socket.with_extension("pid"))
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok())
            == Some(pid)
    }

    fn start_scope(bus: &Path, unit: &str, pid: u32, floor: (u64, u64)) -> Result<(), String> {
        let mut child = Command::new("busctl")
            .args(scope_call(bus, unit, pid, floor))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("busctl: {e}"))?;
        // busctl's own --timeout bounds the method call; this bounds everything else.
        let deadline = Instant::now() + TIMEOUT + std::time::Duration::from_millis(500);
        loop {
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return await_moved(pid, unit, deadline),
                Ok(Some(status)) => {
                    let mut said = String::new();
                    if let Some(mut err) = child.stderr.take() {
                        let _ = err.read_to_string(&mut said);
                    }
                    return Err(format!(
                        "busctl StartTransientUnit {unit}: {status}: {}",
                        said.trim()
                    ));
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "busctl StartTransientUnit {unit}: no answer within {}s",
                        TIMEOUT.as_secs()
                    ));
                }
                Err(e) => return Err(format!("busctl: {e}")),
            }
        }
    }

    /// Wait until the kernel shows `pid` in `unit`: `StartTransientUnit` answers once the start
    /// job is queued, and the move is that job's, so only the pid's own cgroup says it happened.
    fn await_moved(pid: u32, unit: &str, deadline: Instant) -> Result<(), String> {
        let path = format!("/proc/{pid}/cgroup");
        let needle = format!("/{unit}");
        loop {
            let cgroup = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read the daemon's cgroup: {e}"))?;
            if cgroup.contains(&needle) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the user manager took {unit} but the daemon is still in {}",
                    cgroup.trim()
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// The `memory.min` and `memory.low` the slice carries in the account's user manager; zero
    /// when absent or not a number.
    fn slice_floor(uid: u32) -> (u64, u64) {
        let dir = PathBuf::from(format!(
            "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/{SLICE}"
        ));
        let read = |name: &str| {
            std::fs::read_to_string(dir.join(name))
                .ok()
                .and_then(|raw| raw.trim().parse::<u64>().ok())
                .unwrap_or(0)
        };
        (read("memory.min"), read("memory.low"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{is_live_drawer, scope_call, scope_name};

    #[test]
    fn only_the_home_cache_root_is_the_live_drawer() {
        let home = Path::new("/home/someone");
        assert!(is_live_drawer(
            Path::new("/home/someone/.cache/meridian"),
            Some(home)
        ));
        assert!(!is_live_drawer(
            Path::new("/tmp/sandbox/meridian"),
            Some(home)
        ));
        assert!(!is_live_drawer(
            Path::new("/home/someone/.cache/meridian"),
            None
        ));
    }

    #[test]
    fn the_scope_call_carries_the_pid_slice_floor_and_collect_mode() {
        let unit = scope_name(Path::new("/run/user/1000/mrd/8b3ea6d90c5d.sock"), 42);
        assert_eq!(unit, "mrd-engine-8b3ea6d90c5d-42.scope");
        let joined = scope_call(
            Path::new("/run/user/1000/bus"),
            &unit,
            42,
            (8 << 30, 8 << 30),
        )
        .join(" ");
        for want in [
            "--address=unix:path=/run/user/1000/bus",
            "--timeout=2",
            "StartTransientUnit ssa(sv)a(sa(sv)) mrd-engine-8b3ea6d90c5d-42.scope fail 6 ",
            "PIDs au 1 42",
            "Slice s app.slice",
            "CollectMode s inactive-or-failed",
            "MemoryMin t 8589934592",
            "MemoryLow t 8589934592",
        ] {
            assert!(
                joined.contains(want),
                "busctl argv lacks {want:?}: {joined}"
            );
        }
        let args = scope_call(Path::new("/run/user/1000/bus"), &unit, 42, (0, 0));
        assert!(
            args.iter()
                .any(|a| a == "mrd engine (placed by the client that spawned it)"),
            "a string value is one word: {args:?}"
        );
        assert!(
            joined.ends_with(" 0"),
            "the aux array must be empty: {joined}"
        );
        let bare = scope_call(Path::new("/run/user/1000/bus"), &unit, 42, (0, 0)).join(" ");
        assert!(
            !bare.contains("Memory") && bare.contains(" fail 4 "),
            "no floor sets none: {bare}"
        );
    }
}
