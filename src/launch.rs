// Opening a terminal in a repository - what a click on a board row does.
//
// One terminal, one flag. cosmic-term takes `--working-directory <dir>`
// (`-w`), and the child's own working directory is set to the repo as well,
// so a terminal that ignores the flag still lands in the right place. To use
// another terminal, change TERMINAL and TERMINAL_DIR_FLAG - alacritty and
// foot take `--working-directory` too, kitty wants `--directory`,
// gnome-terminal `--working-directory`.

use std::path::Path;
use std::process::{Command, Stdio};

/// The terminal emulator to start.
const TERMINAL: &str = "cosmic-term";

/// Its "start in this directory" flag.
const TERMINAL_DIR_FLAG: &str = "--working-directory";

/// Start a terminal in `dir`. Fire-and-forget: the applet doesn't wait on
/// it, and a failure (terminal not installed, say) is logged, not propagated.
pub fn terminal_in(dir: &Path) {
    let spawned = Command::new(TERMINAL)
        .arg(TERMINAL_DIR_FLAG)
        .arg(dir)
        .current_dir(dir)
        // The terminal has no business talking to the panel's stdio.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();

    match spawned {
        Ok(mut child) => {
            // Reap the process when it exits so it doesn't linger as a zombie
            // for as long as the applet runs. The thread sleeps in wait() and
            // costs nothing; the terminal itself keeps going if the applet
            // restarts, since it's a separate process and not a session child.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(why) => eprintln!("branchkeeper: couldn't start {TERMINAL}: {why}"),
    }
}
