// Applet entry point - initializes localization and runs the applet.
//
// Three ways in. The panel starts the binary with no arguments and gets the
// applet. The other two are terminal checks that print and exit without ever
// starting the event loop, so the pieces can be tested from a shell before
// (and without) the applet being installed in a panel:
//
//   branchkeeper --status [PATH ...]   the git reads: one line per repository
//                                      (no paths = the same scan the applet runs)
//   branchkeeper --scan                the process scan: every interactive
//                                      shell it can see, newest first
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
use std::time::SystemTime;

use repo::RepoStatus;

fn main() -> cosmic::iced::Result {
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(flag) if flag == "--status" => {
            status_report(args.map(PathBuf::from).collect());
            return Ok(());
        }
        Some(flag) if flag == "--scan" => {
            scan_report();
            return Ok(());
        }
        _ => {}
    }

    // Get the system's preferred languages.
    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();

    // Enable localizations to be applied.
    i18n::init(&requested_languages);

    // Starts the applet's event loop with `()` as the application's flags.
    cosmic::applet::run::<app::AppModel>(())
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
