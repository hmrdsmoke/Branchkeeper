// One repository's status, read with libgit2 (the `git2` crate) rather than
// by shelling out to `git`. Everything the panel and popup show about a repo
// comes from here: its name, the checked-out branch, how far it is ahead of or
// behind its upstream, and whether the working tree has uncommitted changes.
//
// This module is deliberately UI-free. It returns numbers and names; app.rs
// decides how to word them. The one exception is `headline`, which is pure
// symbols (name, branch, arrows) and is shared by the panel and `--status`.

use std::path::{Path, PathBuf};

use git2::{BranchType, ErrorCode, Repository, StatusOptions};

/// How the checked-out branch relates to its upstream (tracking) branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tracking {
    /// Commits on the local branch that the upstream doesn't have yet (unpushed).
    pub ahead: usize,
    /// Commits on the upstream that the local branch doesn't have yet (unpulled).
    pub behind: usize,
}

/// A snapshot of one repository, as of the moment `read` ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoStatus {
    /// The working tree's directory name - "soulless-launcher", not the full
    /// path. What the panel calls the repo.
    pub name: String,
    /// The working tree root. Also the identity used to spot the same repo
    /// reached from two different subdirectories.
    pub workdir: PathBuf,
    /// The checked-out branch. For a detached HEAD this is "@" plus the short
    /// commit id, since there is no branch to name.
    pub branch: String,
    /// Ahead/behind counts against the upstream, or `None` when the branch has
    /// no upstream set (never pushed, or a detached HEAD).
    pub tracking: Option<Tracking>,
    /// How many entries `git status` would list: modified, staged, deleted,
    /// renamed, and untracked. 0 means clean.
    pub changed: usize,
}

impl RepoStatus {
    /// Read the repository that contains `path`. The path may be anywhere
    /// inside the working tree - a terminal sitting in `src/` resolves to the
    /// repo root the same way `git` itself does (discover walks upward).
    ///
    /// Errors are libgit2's own: not a repository, unreadable, and so on. A
    /// bare repository (no working tree) is reported as an error too, because
    /// nothing we show makes sense without a working tree.
    pub fn read(path: &Path) -> Result<Self, git2::Error> {
        let repo = Repository::discover(path)?;

        // libgit2 hands the workdir back with a trailing slash ("/home/me/repo/").
        // Rebuilding it from its components drops that, so the path displays
        // cleanly and compares equal however it was reached.
        let workdir: PathBuf = repo
            .workdir()
            .ok_or_else(|| git2::Error::from_str("bare repository (no working tree)"))?
            .components()
            .collect();
        let name = workdir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| workdir.display().to_string());

        let (branch, tracking) = head_info(&repo)?;
        let changed = changed_count(&repo)?;

        Ok(Self {
            name,
            workdir,
            branch,
            tracking,
            changed,
        })
    }

    /// True when the working tree has anything uncommitted.
    pub fn is_dirty(&self) -> bool {
        self.changed > 0
    }

    /// The compact ahead/behind marker: "↑2", "↓1", "↑2↓1", or empty when in
    /// sync or there's no upstream. Pure symbols, so the panel stays short.
    pub fn sync_label(&self) -> String {
        let mut label = String::new();
        if let Some(tracking) = self.tracking {
            if tracking.ahead > 0 {
                label.push_str(&format!("↑{}", tracking.ahead));
            }
            if tracking.behind > 0 {
                label.push_str(&format!("↓{}", tracking.behind));
            }
        }
        label
    }

    /// The panel's one-liner: `soulless main ↑2`. Name, branch, then the sync
    /// marker only when there's something to say.
    pub fn headline(&self) -> String {
        let sync = self.sync_label();
        if sync.is_empty() {
            format!("{} {}", self.name, self.branch)
        } else {
            format!("{} {} {}", self.name, self.branch, sync)
        }
    }
}

/// The branch name and its ahead/behind, handling the three states HEAD can be
/// in: on a branch (the normal case), detached, or unborn (a fresh `git init`
/// with no commits yet, where HEAD names a branch that doesn't exist).
fn head_info(repo: &Repository) -> Result<(String, Option<Tracking>), git2::Error> {
    let head = match repo.head() {
        Ok(head) => head,
        Err(e) if e.code() == ErrorCode::UnbornBranch => {
            // No commits yet. HEAD is still a symbolic ref to the branch that
            // the first commit will create, so show that name.
            let name = repo
                .find_reference("HEAD")?
                .symbolic_target()
                .and_then(|target| target.strip_prefix("refs/heads/"))
                .unwrap_or("(no commits)")
                .to_owned();
            return Ok((name, None));
        }
        Err(e) => return Err(e),
    };

    if !head.is_branch() {
        // Detached HEAD (checked out a tag or a commit): no branch to name, so
        // show where we are instead. No upstream either.
        let short = head
            .target()
            .map(|oid| oid.to_string()[..7].to_owned())
            .unwrap_or_default();
        return Ok((format!("@{short}"), None));
    }

    // `shorthand` strips "refs/heads/", leaving "main" or "feature/thing".
    let name = head.shorthand().unwrap_or("HEAD").to_owned();

    // Ahead/behind needs an upstream. Every step here can legitimately come up
    // empty (no upstream configured, upstream ref gone after a remote prune),
    // and in all of those the right answer is "no tracking", not an error.
    let tracking = (|| {
        let local = head.target()?;
        let branch = repo.find_branch(&name, BranchType::Local).ok()?;
        let upstream = branch.upstream().ok()?;
        let remote = upstream.get().target()?;
        let (ahead, behind) = repo.graph_ahead_behind(local, remote).ok()?;
        Some(Tracking { ahead, behind })
    })();

    Ok((name, tracking))
}

/// How many entries `git status` would show. Counts staged and unstaged
/// changes plus untracked files; ignored files are left out (libgit2's
/// default), so build output in target/ never makes a repo look dirty.
fn changed_count(repo: &Repository) -> Result<usize, git2::Error> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        // A brand-new directory counts once, not once per file inside it -
        // same as `git status` shows it, and much cheaper on a big drop.
        .recurse_untracked_dirs(false)
        // Submodule contents are their own repos' business.
        .exclude_submodules(true);
    Ok(repo.statuses(Some(&mut opts))?.len())
}
