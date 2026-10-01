# Branchkeeper

Git branch and sync status for the COSMIC panel.

Branchkeeper shows the git status of whatever repository you're working in. The panel shows one repo — the one you're in — as `soulless main ↑2 ↓0`: the repo name, the checked-out branch, and how many commits it is ahead of (`↑`) and behind (`↓`) its upstream. Click it for the full board: the repositories you currently have a terminal in, then every other project in your project folders, each with its branch, ahead/behind, and uncommitted changes. Click any row to open a terminal in that repo.

Panel is the headline, popup is the full board.

## How it finds your repos

Branchkeeper doesn't ask you to point it at anything. It looks at where your shells are sitting — every interactive bash, zsh, fish, or nu in any terminal, including Zed's terminal panes and tmux windows — reads each one's working directory, and keeps the ones that are inside a git repository. Open a terminal in a repo and it's on the board; close it and it's gone. The repo whose terminal you touched most recently is the one in the panel.

It reads this from `/proc`, which only shows your own processes: a root shell (`sudo -i`) is invisible to it. `branchkeeper --scan` lists exactly which shells it can see.

The rest of the board comes from your project folders: every folder one level down that has a `.git` is a project, open or not. Out of the box that's `~/Projects`. Settings, at the bottom of the popup, is where you add more — Add folder opens the system folder picker, and the repos in whatever you choose are on the board the next time you open it. Folders are saved with the rest of your COSMIC settings.

Everything git-related is read with libgit2 — no `git` process is ever started. `↓` is only as fresh as your last `git fetch` or `git pull`; Branchkeeper never touches the network.

## Status

Working. Git reads, the shell scan, the project list, terminal launching, and project-folder settings are in. The terminal it opens is `cosmic-term` (a constant in `src/launch.rs` if you use another).

## Building

You'll need Rust and the COSMIC development dependencies.

Clone the repo, then run `make` for a debug build, or `make release` for a release build. Install with `sudo make install`, which puts the binary, desktop file, metainfo, and icon in place and updates the icon cache. Then open COSMIC Panel settings and add Branchkeeper to your panel.

To remove it, run `sudo make uninstall`.

## Command line

The same binary answers a few flags instead of starting the applet, so you — or another program — can read the board and open repos from a script:

```
branchkeeper --status [PATH ...]   # the board, human-readable (no paths = the scan)
branchkeeper --scan                # every interactive shell it can see, newest first
branchkeeper --json                # the board as JSON, for other programs
branchkeeper --launch <name|path>  # open a terminal in a repo, by name or path
```

`--json` prints `{ "open": [...], "projects": [...] }`, each repo an object with `name`, `branch`, `ahead`, `behind` (both `null` when there's no upstream), `changed`, and `path`. `--launch soulless` opens a terminal in the repo named `soulless` on the board; it also takes a path, and exits non-zero if no such repo is found. These are what a desktop assistant can call to list your projects and open one on request.

## License

GPL-3.0
