// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// branchkeeper/src/actions.rs
// src/actions.rs

// Acting on a single repository - what the per-row Pull / Push / Fetch buttons
// do. The board (repo.rs) reads status; this writes. Everything here is
// libgit2 (the git2 crate), never a `git` process, same as the rest of the app.
//
// Three verbs, deliberately conservative:
//   fetch  update this repo's remote-tracking refs. Touches nothing in the
//          working tree. Identical to a bare `git fetch`.
//   pull   fetch, then FAST-FORWARD the current branch to its upstream. If the
//          branch can't fast-forward (it has diverged - local commits the
//          upstream doesn't have), we STOP and say so. We never create a merge
//          commit and never rebase; that's a decision for a real terminal, not
//          a panel button. So pull is "catch up when it's safe", nothing more.
//   push   push the current branch to its upstream remote.
//
// Credentials work the same way `git` itself does - ssh-agent for SSH remotes,
// the git credential helper for HTTPS - so whatever already lets you push from
// the command line works here with nothing to set up. The callback is shared
// with fetch.rs via `credentials`.
//
// Every function returns `Outcome`: a plain, already-worded result the UI can
// show as-is. Errors are turned into short human sentences here rather than
// leaking libgit2's wording into the popup.

use std::path::Path;

use git2::{
    AnnotatedCommit, AutotagOption, Cred, CredentialType, FetchOptions, PushOptions,
    RemoteCallbacks, Repository,
};

/// The result of one action, worded for a human. `ok` drives whether the UI
/// shows it as success or trouble; `msg` is the short line it shows either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub msg: String,
}

impl Outcome {
    fn ok(msg: impl Into<String>) -> Self {
        Self { ok: true, msg: msg.into() }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self { ok: false, msg: msg.into() }
    }
}

/// Which action to run. The UI sends one of these per button; `run` dispatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Fetch,
    Pull,
    Push,
}

/// Run `action` against the repo at `dir`, returning an already-worded Outcome.
/// Blocking and (for fetch/pull/push) network-bound: the applet runs it on a
/// background thread, never on the UI thread (see app.rs).
pub fn run(action: Action, dir: &Path) -> Outcome {
    let result = match action {
        Action::Fetch => fetch(dir),
        Action::Pull => pull(dir),
        Action::Push => push(dir),
    };
    // Any libgit2 error that escaped the inner functions becomes a short line.
    result.unwrap_or_else(|e| Outcome::err(short_error(&e)))
}

/// Fetch this repo's default remote. Remote-tracking refs move; the working
/// tree does not. "up to date" vs "fetched" we can't easily tell apart without
/// diffing refs before/after, so we just report success plainly.
fn fetch(dir: &Path) -> Result<Outcome, git2::Error> {
    let repo = Repository::open(dir)?;
    let Some(remote_name) = default_remote(&repo) else {
        return Ok(Outcome::err("no remote"));
    };
    do_fetch(&repo, &remote_name)?;
    Ok(Outcome::ok("fetched"))
}

/// Fetch, then fast-forward the current branch to its upstream if it can.
/// Three honest endings: already current, fast-forwarded by N, or "diverged"
/// (can't fast-forward without a merge/rebase, which we won't do from here).
fn pull(dir: &Path) -> Result<Outcome, git2::Error> {
    let repo = Repository::open(dir)?;
    let Some(remote_name) = default_remote(&repo) else {
        return Ok(Outcome::err("no remote"));
    };

    // Must be on a branch to pull into one.
    let head = repo.head()?;
    if !head.is_branch() {
        return Ok(Outcome::err("detached HEAD"));
    }
    let branch_name = head.shorthand().unwrap_or("HEAD").to_owned();

    do_fetch(&repo, &remote_name)?;

    // Where the upstream is now, after the fetch.
    let upstream_ref = format!("refs/remotes/{remote_name}/{branch_name}");
    let upstream_oid = match repo.refname_to_id(&upstream_ref) {
        Ok(oid) => oid,
        // No upstream ref even after fetching: the branch has no tracking
        // counterpart on the remote. Nothing to pull into it.
        Err(_) => return Ok(Outcome::err("no upstream")),
    };
    let fetched: AnnotatedCommit = repo.find_annotated_commit(upstream_oid)?;

    // What kind of merge would git do here?
    let (analysis, _pref) = repo.merge_analysis(&[&fetched])?;

    if analysis.is_up_to_date() {
        return Ok(Outcome::ok("up to date"));
    }
    if !analysis.is_fast_forward() {
        // Diverged (local has commits the upstream doesn't) or unborn in a way
        // we won't handle. A real merge/rebase is a terminal decision.
        return Ok(Outcome::err("diverged - pull in a terminal"));
    }

    // Fast-forward: move the branch ref to the upstream commit and check out.
    let count = ahead_count(&repo, upstream_oid, &head)?;
    let mut branch_ref = repo.find_reference(&format!("refs/heads/{branch_name}"))?;
    branch_ref.set_target(upstream_oid, "branchkeeper: fast-forward")?;
    repo.set_head(&format!("refs/heads/{branch_name}"))?;
    // Force so the working tree and index match the new HEAD; safe because a
    // fast-forward only adds commits the tree didn't have, never discards work.
    repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;

    Ok(Outcome::ok(format!("pulled {count}")))
}

/// Push the current branch to its upstream remote. Reports "pushed",
/// "up to date" (nothing to push), or a short failure (auth, rejected).
fn push(dir: &Path) -> Result<Outcome, git2::Error> {
    let repo = Repository::open(dir)?;
    let Some(remote_name) = default_remote(&repo) else {
        return Ok(Outcome::err("no remote"));
    };

    let head = repo.head()?;
    if !head.is_branch() {
        return Ok(Outcome::err("detached HEAD"));
    }
    let branch_name = head.shorthand().unwrap_or("HEAD").to_owned();

    // Nothing to push if the branch isn't ahead of its upstream. (If there's no
    // upstream yet, we still try the push below, which sets it.)
    if let Some((ahead, _behind)) = ahead_behind(&repo, &branch_name, &remote_name) {
        if ahead == 0 {
            return Ok(Outcome::ok("up to date"));
        }
    }

    let mut remote = repo.find_remote(&remote_name)?;

    // Capture a rejection from the server (non-fast-forward, hook refusal) so
    // we can word it, instead of the push silently "succeeding" at the transport
    // level while the ref was rejected.
    let rejected = std::cell::RefCell::new(None::<String>);

    // The push runs inside this block so `opts` (which borrows `rejected` via
    // the callback) is dropped at the closing brace, freeing the borrow before
    // we read `rejected` below. Otherwise `opts` lives to end-of-function and
    // the borrow outlives the read.
    {
        let mut callbacks = RemoteCallbacks::new();
        callbacks.credentials(credentials);
        callbacks.push_update_reference(|refname, status| {
            if let Some(reason) = status {
                *rejected.borrow_mut() = Some(format!("{refname}: {reason}"));
            }
            Ok(())
        });

        let mut opts = PushOptions::new();
        opts.remote_callbacks(callbacks);

        // Push the branch to the same-named branch on the remote. No leading
        // '+' - no force - so a non-fast-forward is rejected by the server and
        // surfaces via push_update_reference above.
        let refspec = format!("refs/heads/{branch_name}:refs/heads/{branch_name}");
        remote.push(&[refspec.as_str()], Some(&mut opts))?;
    }

    if let Some(reason) = rejected.into_inner() {
        return Ok(Outcome::err(format!("rejected: {reason}")));
    }
    Ok(Outcome::ok("pushed"))
}

// --- shared helpers ---------------------------------------------------------

/// Run a fetch on `remote_name` with prune, the same way fetch.rs does for
/// "Fetch all" - credentials callback, the remote's configured refspecs.
fn do_fetch(repo: &Repository, remote_name: &str) -> Result<(), git2::Error> {
    let mut remote = repo.find_remote(remote_name)?;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(credentials);

    let mut opts = FetchOptions::new();
    opts.remote_callbacks(callbacks);
    opts.prune(git2::FetchPrune::On);
    // Don't drag down tags we don't track; match a plain fetch.
    opts.download_tags(AutotagOption::Auto);

    remote.fetch::<&str>(&[], Some(&mut opts), None)
}

/// How many commits the upstream is ahead of our current HEAD (i.e. how many a
/// fast-forward will bring in). Best-effort; 0 on any hiccup.
fn ahead_count(
    repo: &Repository,
    upstream: git2::Oid,
    head: &git2::Reference,
) -> Result<usize, git2::Error> {
    let local = head.target().unwrap_or(upstream);
    let (behind_us, _) = repo.graph_ahead_behind(upstream, local)?;
    Ok(behind_us)
}

/// Ahead/behind of the local branch vs `remote_name/branch`, or None if either
/// side can't be resolved (no upstream ref, etc.).
fn ahead_behind(repo: &Repository, branch: &str, remote_name: &str) -> Option<(usize, usize)> {
    let local = repo.refname_to_id(&format!("refs/heads/{branch}")).ok()?;
    let upstream = repo
        .refname_to_id(&format!("refs/remotes/{remote_name}/{branch}"))
        .ok()?;
    repo.graph_ahead_behind(local, upstream).ok()
}

/// Pick the remote to act on: the current branch's upstream remote if set, else
/// "origin", else the sole remote. None when the repo has no remotes. (Same
/// rule as fetch.rs's default_remote - kept in step with it deliberately.)
fn default_remote(repo: &Repository) -> Option<String> {
    if let Ok(head) = repo.head()
        && head.is_branch()
        && let Some(branch) = head.shorthand()
        && let Ok(name) = repo.branch_upstream_remote(&format!("refs/heads/{branch}"))
        && let Some(name) = name.as_str()
    {
        return Some(name.to_owned());
    }

    let remotes = repo.remotes().ok()?;
    if remotes.iter().flatten().any(|r| r == "origin") {
        return Some("origin".to_owned());
    }
    match remotes.len() {
        1 => remotes.get(0).map(str::to_owned),
        _ => None,
    }
}

/// The credentials callback git2 calls when a remote needs auth. Same behavior
/// as fetch.rs: ssh-agent for SSH, the git credential helper for HTTPS. Kept
/// identical on purpose so push and fetch authenticate the same way.
fn credentials(
    url: &str,
    username: Option<&str>,
    allowed: CredentialType,
) -> Result<Cred, git2::Error> {
    if allowed.contains(CredentialType::SSH_KEY) {
        return Cred::ssh_key_from_agent(username.unwrap_or("git"));
    }
    if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
        let config = git2::Config::open_default()?;
        return Cred::credential_helper(&config, url, username);
    }
    if allowed.contains(CredentialType::DEFAULT) {
        return Cred::default();
    }
    Err(git2::Error::from_str("no supported authentication method"))
}

/// Turn a libgit2 error into a short line fit for the popup. We keep it terse -
/// the popup is narrow - and special-case the auth one since it's the common
/// real-world failure (expired token, no ssh-agent key).
fn short_error(e: &git2::Error) -> String {
    use git2::ErrorClass;
    match e.class() {
        ErrorClass::Ssh | ErrorClass::Http | ErrorClass::Net => "network or auth failed".to_owned(),
        ErrorClass::Callback => "auth failed".to_owned(),
        _ => {
            // Collapse libgit2's (sometimes long) message to its first line.
            let first = e.message().lines().next().unwrap_or("failed");
            first.to_owned()
        }
    }
}
