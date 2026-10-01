// Fetching every repo on the board, so the behind (↓) counts tell the truth.
//
// Branchkeeper reads what's on disk and never touches the network on its own,
// which means ↓ is only as fresh as your last `git fetch`. This is the one
// place that reaches out: the "Fetch all" button runs a fetch on every repo's
// default remote, and the next board refresh shows the real ↓.
//
// Reads only. We fetch - update the remote-tracking refs - and stop there. No
// merge, no rebase, no fast-forward of your branch: nothing in your working
// tree moves, exactly like `git fetch` on its own. The worst a fetch can do
// is update `origin/*` and bump a ↓.
//
// Credentials come from the same places git itself uses, so whatever lets you
// push already works here with nothing to configure:
//   - SSH remotes (git@github.com:...): your ssh-agent.
//   - HTTPS remotes: the git credential helper (where a stored token lives).

use std::path::{Path, PathBuf};

use git2::{Cred, CredentialType, FetchOptions, RemoteCallbacks, Repository};

/// Fetch every repo in `dirs` on its default remote. Returns how many were
/// reached, out of how many were tried - so the button can say "fetched 11 of
/// 12" when one repo has no remote or the network's down. Blocking and
/// network-bound; the applet runs it on a background thread (see app.rs).
pub fn fetch_all(dirs: &[PathBuf]) -> FetchReport {
    let mut report = FetchReport {
        attempted: dirs.len(),
        succeeded: 0,
    };
    for dir in dirs {
        if fetch_one(dir).is_ok() {
            report.succeeded += 1;
        }
    }
    report
}

/// The outcome of a fetch-all: how many repos were reached, out of how many.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchReport {
    pub attempted: usize,
    pub succeeded: usize,
}

/// Fetch one repo on its default remote (usually `origin`). A repo with no
/// remote at all is not an error worth shouting about - it just can't be
/// fetched - so it returns `Ok` with nothing done; a real failure (auth, no
/// network) returns `Err` and is logged by the caller via the count.
fn fetch_one(dir: &Path) -> Result<(), git2::Error> {
    let repo = Repository::open(dir)?;

    // The default remote: whatever the current branch tracks, else "origin",
    // else the only remote there is. No remote => nothing to fetch, quietly.
    let remote_name = default_remote(&repo);
    let Some(remote_name) = remote_name else {
        return Ok(());
    };
    let mut remote = repo.find_remote(&remote_name)?;

    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(credentials);

    let mut opts = FetchOptions::new();
    opts.remote_callbacks(callbacks);
    // Prune remote-tracking refs for branches deleted on the remote, so a
    // merged-and-deleted branch stops lingering in the counts.
    opts.prune(git2::FetchPrune::On);

    // Empty refspecs => use the remote's configured fetch refspecs, the same
    // ones a bare `git fetch` uses. We only move remote-tracking refs.
    remote.fetch::<&str>(&[], Some(&mut opts), None)
}

/// Pick the remote to fetch: the current branch's upstream remote if it has
/// one, otherwise "origin" if it exists, otherwise the sole remote (if there's
/// exactly one). `None` when the repo has no remotes.
fn default_remote(repo: &Repository) -> Option<String> {
    // The branch's own upstream remote, if set.
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
    // Exactly one remote under some other name: use it. More than one and no
    // upstream and no "origin" is ambiguous, so leave it alone.
    match remotes.len() {
        1 => remotes.get(0).map(str::to_owned),
        _ => None,
    }
}

/// The credentials callback git2 calls when a remote needs auth. Mirrors what
/// the git command line does: for SSH, hand libgit2 a key from the running
/// ssh-agent; for HTTPS (USER_PASS_PLAINTEXT), ask the configured credential
/// helper. `_url` and the allowed types tell us which kind the server wants.
fn credentials(
    _url: &str,
    username: Option<&str>,
    allowed: CredentialType,
) -> Result<Cred, git2::Error> {
    if allowed.contains(CredentialType::SSH_KEY) {
        // The username is "git" for GitHub-style SSH remotes; fall back to it
        // if the URL somehow carried none.
        return Cred::ssh_key_from_agent(username.unwrap_or("git"));
    }
    if allowed.contains(CredentialType::USER_PASS_PLAINTEXT) {
        // Pulls a stored token/password from the user's git credential helper
        // (e.g. libsecret, the same store `git push` over HTTPS reads).
        let config = git2::Config::open_default()?;
        return Cred::credential_helper(&config, _url, username);
    }
    if allowed.contains(CredentialType::DEFAULT) {
        return Cred::default();
    }
    Err(git2::Error::from_str("no supported authentication method"))
}
