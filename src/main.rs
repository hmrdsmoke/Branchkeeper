// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// branchkeeper/src/main.rs
// src/main.rs

// Applet entry point - initializes localization and runs the applet.
//
// The panel starts the binary with no arguments and gets the applet. Every
// flag below instead prints (or acts) and exits without starting the event
// loop, so the pieces can be driven from a shell - or by another program like
// CosmicSprite - without the applet being installed in a panel:
//
//   branchkeeper --status [PATH ...]   the git reads, human-readable: one line
//                                      per repository (no paths = the full board)
//   branchkeeper --scan                the process scan: every interactive
//                                      shell it can see, newest first
//   branchkeeper --json                the full board as JSON, for other
//                                      programs to read (see json_report)
//   branchkeeper --launch <name|path>  open a terminal in a repo, named the way
//                                      --json reports it, or by path (see launch_repo)
//
// cosmic-panel hands an applet every word of its Exec line as-is, so these
// only trigger on the exact flags; anything else is ignored and the applet
// starts normally.

mod app;
mod fetch;
mod i18n;
mod launch;
mod repo;
mod scan;
mod settings;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::SystemTime;

use repo::RepoStatus;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(flag) if flag == "--status" => {
            status_report(args.map(PathBuf::from).collect());
            ExitCode::SUCCESS
        }
        Some(flag) if flag == "--scan" => {
            scan_report();
            ExitCode::SUCCESS
        }
        Some(flag) if flag == "--json" => {
            json_report();
            ExitCode::SUCCESS
        }
        Some(flag) if flag == "--launch" => {
            // The target is everything after the flag, joined back with spaces
            // so an unquoted repo name with spaces still works. Exit code says
            // whether a repo was actually found and opened, so a caller (or
            // Shuna) can tell success from "no such repo".
            let target = args
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ");
            launch_repo(&target)
        }
        _ => run_applet(),
    }
}

/// Start the panel applet's event loop. Returns FAILURE if libcosmic's run
/// returns an error, so the exit code is meaningful either way.
fn run_applet() -> ExitCode {
    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    match cosmic::applet::run::<app::AppModel>(()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("branchkeeper: {why}");
            ExitCode::FAILURE
        }
    }
}

/// Print the status of each given repository path, one per line, in the same
/// shape the panel shows it. With no paths, build the same board the applet
/// shows (see `scan::snapshot`) and print it section by section.
fn status_report(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        // The same folders the applet uses, from the saved settings.
        let board = scan::snapshot(&settings::Settings::load().project_roots);
        if board.is_empty() {
            println!("no repositories found");
            return;
        }
        println!("open:");
        print_rows(&board.open);
        println!("projects:");
        print_rows(&board.projects);
        return;
    }

    let repos: Vec<RepoStatus> = paths
        .iter()
        .filter_map(|path| match RepoStatus::read(path) {
            Ok(status) => Some(status),
            Err(why) => {
                eprintln!("branchkeeper: {}: {}", path.display(), why.message());
                None
            }
        })
        .collect();
    print_rows(&repos);
}

/// One line per repo. Plain numbers here, not the applet's wording: this is
/// the raw read-out for checking the git side, so nothing is hidden behind a
/// label. An empty section prints as a single "(none)".
fn print_rows(repos: &[RepoStatus]) {
    if repos.is_empty() {
        println!("  (none)");
        return;
    }
    for status in repos {
        let sync = match status.tracking {
            Some(tracking) => format!("↑{} ↓{}", tracking.ahead, tracking.behind),
            None => "no upstream".to_owned(),
        };
        println!(
            "  {:<40} {:<14} {} changed    {}",
            status.headline(),
            sync,
            status.changed,
            status.workdir.display()
        );
    }
}

/// Print the whole board as JSON, for another program to read - this is the
/// interface CosmicSprite (Shuna) calls to know the projects and their state.
/// Shape (stable; add fields, don't rename):
///
///   {
///     "open":     [ <repo>, ... ],   // repos with a terminal in them, newest first
///     "projects": [ <repo>, ... ]    // every other project in the folders, A-Z
///   }
///   <repo> = {
///     "name":    "soulless",
///     "branch":  "main",             // or "@<short-sha>" when detached
///     "ahead":   2,                  // null when there's no upstream
///     "behind":  0,                  // null when there's no upstream
///     "changed": 3,                  // uncommitted entries; 0 = clean
///     "path":    "/home/you/Projects/soulless"
///   }
///
/// Built from the same scan the applet uses, so `--json` and the panel always
/// agree. Hand-rolled rather than pulling in a JSON crate: the shape is tiny
/// and fixed, and the one sharp edge - strings with quotes or backslashes - is
/// handled by `json_escape`.
fn json_report() {
    let board = scan::snapshot(&settings::Settings::load().project_roots);

    let mut out = String::from("{\n  \"open\": ");
    out.push_str(&repos_to_json(&board.open));
    out.push_str(",\n  \"projects\": ");
    out.push_str(&repos_to_json(&board.projects));
    out.push_str("\n}");
    println!("{out}");
}

/// A JSON array of repo objects, pretty-printed two levels in.
fn repos_to_json(repos: &[RepoStatus]) -> String {
    if repos.is_empty() {
        return "[]".to_owned();
    }
    let items: Vec<String> = repos
        .iter()
        .map(|r| {
            let (ahead, behind) = match r.tracking {
                Some(t) => (t.ahead.to_string(), t.behind.to_string()),
                None => ("null".to_owned(), "null".to_owned()),
            };
            format!(
                "    {{ \"name\": \"{}\", \"branch\": \"{}\", \"ahead\": {}, \"behind\": {}, \"changed\": {}, \"path\": \"{}\" }}",
                json_escape(&r.name),
                json_escape(&r.branch),
                ahead,
                behind,
                r.changed,
                json_escape(&r.workdir.to_string_lossy()),
            )
        })
        .collect();
    format!("[\n{}\n  ]", items.join(",\n"))
}

/// Escape the characters JSON strings can't carry literally. Enough for repo
/// names, branches, and paths (control chars below space are rare in those but
/// handled anyway).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Open a terminal in a repo chosen by `target`, which is either a repo name
/// as `--json` reports it ("soulless") or a path. Returns SUCCESS when a repo
/// was found and the terminal started, FAILURE otherwise - so a caller can act
/// on the result.
///
/// Matching, in order: an exact path to a repo; then a repo on the board whose
/// name equals `target` (case-insensitive); then, if nothing's on the board by
/// that name, a direct path that happens to be a repo. Board-first so "open
/// soulless" works without the full path, path-fallback so a repo that isn't in
/// any project folder can still be opened by its path.
fn launch_repo(target: &str) -> ExitCode {
    if target.is_empty() {
        eprintln!("branchkeeper --launch: needs a repo name or path");
        return ExitCode::FAILURE;
    }

    // By name on the board (the common case: `--launch soulless`).
    let board = scan::snapshot(&settings::Settings::load().project_roots);
    let on_board = board
        .open
        .iter()
        .chain(board.projects.iter())
        .find(|r| r.name.eq_ignore_ascii_case(target));
    if let Some(repo) = on_board {
        launch::terminal_in(&repo.workdir);
        println!("opening {} ({})", repo.name, repo.workdir.display());
        return ExitCode::SUCCESS;
    }

    // By path: a directory that is itself a repo, even one in no project folder.
    let path = settings::expand_root(target);
    if let Ok(status) = RepoStatus::read(&path) {
        launch::terminal_in(&status.workdir);
        println!("opening {} ({})", status.name, status.workdir.display());
        return ExitCode::SUCCESS;
    }

    eprintln!("branchkeeper --launch: no repo named or found at {target:?}");
    ExitCode::FAILURE
}

/// Print every interactive shell the scan can see, newest activity first,
/// whether or not it's in a repo. This is the "why doesn't it see my
/// terminal?" tool: if a shell isn't in this list, the applet can't know
/// about it either (see the notes at the top of scan.rs for the usual
/// reasons - not a listed shell, no pty on stdin, or not our UID).
fn scan_report() {
    let shells = scan::interactive_shells();
    if shells.is_empty() {
        println!("no interactive shells found");
        return;
    }
    let now = SystemTime::now();
    for shell in &shells {
        let idle = now
            .duration_since(shell.active)
            .map(|age| age.as_secs())
            .unwrap_or(0);
        println!(
            "{:>7}  {:<6} {:<12} idle {:>5}s  {}",
            shell.pid,
            shell.comm,
            shell.pty.display(),
            idle,
            shell.cwd.display()
        );
    }
}
