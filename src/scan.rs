// Building the board: which repositories are "open" right now (by looking
// at where the user's shells are sitting), plus every project under the
// project roots whether it's open or not.
//
// The original design said "scan for terminals and file managers and read
// their cwd". The scan ended up one level down from that, for a reason worth
// keeping in mind: a terminal emulator never changes directory. `cd` moves
// the SHELL running inside it; cosmic-term's own /proc/<pid>/cwd stays
// wherever the panel launched it, forever. The same goes double for a file
// manager - cosmic-files browses without ever chdir-ing, so there is nothing
// to read from it. So the scan looks for interactive shells instead: a
// process named like a shell (bash, zsh, fish, ...) whose stdin is a
// pseudo-terminal (/dev/pts/N). That is every shell in every cosmic-term
// tab, every Zed terminal pane, every tmux window - with no list of
// emulators to keep up to date.
//
// Two facts about /proc that explain the behavior when something's missing:
//
//   * /proc/<pid>/cwd and /proc/<pid>/fd/0 are only readable for processes
//     under our own UID. Every shell the user opens qualifies, so it works;
//     a root shell (`sudo -i`) is invisible, and that's a permission wall,
//     not a bug. `branchkeeper --scan` lists exactly what the scan can see.
//
//   * "Most recently active" comes from the pseudo-terminal itself. The
//     kernel stamps /dev/pts/N's access time on input (keystrokes) and its
//     modification time on output - it's what `w` reads for its IDLE column -
//     so the shell whose pty was touched last is the one the user is in.
//     The kernel only moves those stamps in 8-second steps, on purpose
//     (keystroke-timing privacy), which is plenty for ordering a panel.

use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use git2::Repository;

use crate::repo::RepoStatus;
use crate::settings;

/// Process names (`/proc/<pid>/comm`) that count as a shell. comm is the
/// executable's base name, 15 characters max, so "bash" here matches a login
/// "-bash" too. Add to this if you use something else.
const SHELLS: &[&str] = &["bash", "zsh", "fish", "nu", "sh", "dash", "ksh", "tcsh"];

/// One interactive shell found in /proc.
#[derive(Debug, Clone)]
pub struct Shell {
    pub pid: u32,
    /// The process name, e.g. "bash".
    pub comm: String,
    /// The pseudo-terminal on its stdin, e.g. /dev/pts/3.
    pub pty: PathBuf,
    /// The shell's current directory - what the user `cd`'d to.
    pub cwd: PathBuf,
    /// When the pty was last read from or written to: the "last touched"
    /// signal. Input and output both count, so a terminal with a build
    /// running in it reads as active even if nobody is typing in it.
    pub active: SystemTime,
}

/// Every interactive shell this user has open, most recently active first.
/// Nothing here opens a repository; it's the cheap /proc walk only.
pub fn interactive_shells() -> Vec<Shell> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };

    let mut shells: Vec<Shell> = entries
        .flatten()
        // Only the numeric directories are processes.
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(read_shell)
        .collect();

    // Newest activity first. `Reverse` flips the natural oldest-first order.
    shells.sort_by_key(|shell| Reverse(shell.active));
    shells
}

/// Read one process out of /proc, if it's an interactive shell we can see.
/// Every `?` here is a quiet "no": not a shell, not on a terminal, not ours
/// to read, or gone since the directory listing.
fn read_shell(pid: u32) -> Option<Shell> {
    let proc_dir = PathBuf::from(format!("/proc/{pid}"));

    let comm = fs::read_to_string(proc_dir.join("comm")).ok()?;
    let comm = comm.trim_end().to_owned();
    if !SHELLS.contains(&comm.as_str()) {
        return None;
    }

    // Interactive means stdin is a pty. A shell running a script from cron, a
    // build, or a pipe has something else on fd 0, and isn't "open" in any
    // sense the panel cares about.
    let pty = fs::read_link(proc_dir.join("fd/0")).ok()?;
    if !pty.starts_with("/dev/pts") {
        return None;
    }

    let cwd = fs::read_link(proc_dir.join("cwd")).ok()?;

    let meta = fs::metadata(&pty).ok()?;
    let active = match (meta.accessed(), meta.modified()) {
        (Ok(read), Ok(written)) => read.max(written),
        (Ok(read), Err(_)) => read,
        (Err(_), Ok(written)) => written,
        (Err(_), Err(_)) => SystemTime::UNIX_EPOCH,
    };

    Some(Shell {
        pid,
        comm,
        pty,
        cwd,
        active,
    })
}

/// Everything the UI needs, already in the order the UI shows it.
#[derive(Debug, Clone, Default)]
pub struct Board {
    /// Repos with a shell sitting in them, most recently touched first.
    /// `open[0]` is the panel's headline.
    pub open: Vec<RepoStatus>,
    /// Every other project under the user's project folders (see
    /// settings.rs), alphabetical. A project that is open lives in `open`,
    /// not here, so nothing shows twice.
    pub projects: Vec<RepoStatus>,
}

impl Board {
    /// The repo the panel shows: the one touched last, if anything is open.
    pub fn headline(&self) -> Option<&RepoStatus> {
        self.open.first()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty() && self.projects.is_empty()
    }
}

/// The whole board: the open repos from the shell scan, then every project
/// under `roots` (the user's project folders, as stored in settings) that
/// isn't already open. The applet runs this on a background thread every
/// refresh (see app.rs); `--status` runs it directly.
pub fn snapshot(roots: &[String]) -> Board {
    let open = open_repos();

    let mut projects: Vec<RepoStatus> = project_dirs(roots)
        .into_iter()
        .filter(|dir| !open.iter().any(|repo| &repo.workdir == dir))
        .filter_map(|dir| RepoStatus::read(&dir).ok())
        .collect();
    projects.sort_by_key(|repo| repo.name.to_lowercase());

    Board { open, projects }
}

/// Project directories under the given roots: one level down, `.git`
/// present. Paths come back canonical (symlinks resolved) so they compare
/// equal to the real paths /proc reports for the shells.
pub fn project_dirs(roots: &[String]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    for root in roots {
        let root = settings::expand_root(root);
        // A root that doesn't exist is simply empty, not an error.
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // `.git` is a directory for a normal clone and a file for a
            // linked worktree; either way it marks a project.
            if !path.join(".git").exists() {
                continue;
            }
            dirs.push(fs::canonicalize(&path).unwrap_or(path));
        }
    }

    dirs
}

/// The repos that have a shell in them, most recently touched first.
fn open_repos() -> Vec<RepoStatus> {
    let mut seen_gitdirs: Vec<PathBuf> = Vec::new();
    let mut repos: Vec<RepoStatus> = Vec::new();

    // The shells come newest-first, so the first shell seen for any repo is
    // the one touched last, and the repo order falls out of the shell order.
    for shell in interactive_shells() {
        // Cheap identity check before the real read: discover_path walks up
        // from the cwd to the repository's .git without opening anything.
        // Not a repo (most shells sit in ~) is the normal case, not an error.
        // Five tabs in the same repo resolve to the same .git and cost one
        // status read, not five.
        let Ok(gitdir) = Repository::discover_path(&shell.cwd, &[] as &[&Path]) else {
            continue;
        };
        if seen_gitdirs.contains(&gitdir) {
            continue;
        }
        seen_gitdirs.push(gitdir);

        // Unreadable, bare, broken: skip it quietly, same as a non-repo.
        let Ok(status) = RepoStatus::read(&shell.cwd) else {
            continue;
        };
        repos.push(status);
    }

    repos
}
