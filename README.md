# Branchkeeper

Git branch and sync status for the COSMIC panel.

Branchkeeper shows the git status of whatever repository you're working in. The panel shows one repo — the one you're in — as `soulless main ↑2`: the repo name, the checked-out branch, and how far ahead of or behind its upstream it is. Click it for the full board: every repository you currently have open, each with its branch, ahead/behind, and uncommitted changes.

Panel is the headline, popup is the full board.

## How it finds your repos

Branchkeeper doesn't ask you to point it at anything. It looks at where your shells are sitting — every interactive bash, zsh, fish, or nu in any terminal, including Zed's terminal panes and tmux windows — reads each one's working directory, and keeps the ones that are inside a git repository. Open a terminal in a repo and it's on the board; close it and it's gone. The repo whose terminal you touched most recently is the one in the panel.

It reads this from `/proc`, which only shows your own processes: a root shell (`sudo -i`) is invisible to it. `branchkeeper --scan` lists exactly which shells it can see.

Everything git-related is read with libgit2 — no `git` process is ever started.

## Status

Early. The git reads (branch, ahead/behind, change count) and the shell scan are in; the popup gets its polish next.

## Building

You'll need Rust and the COSMIC development dependencies.

Clone the repo, then run `make` for a debug build, or `make release` for a release build. Install with `sudo make install`, which puts the binary, desktop file, metainfo, and icon in place and updates the icon cache. Then open COSMIC Panel settings and add Branchkeeper to your panel.

To check the git reads from a terminal without installing anything:

```
cargo run -- --status ~/Projects/some-repo ~/Projects/another
```

prints one line per repository. With no paths it runs the same scan the applet runs.

To remove it, run `sudo make uninstall`.

## License

GPL-3.0
