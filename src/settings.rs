// GPL-3.0-or-later - see LICENSE file for full terms
// Copyright 2026 Michael Van Auker (HMRDSmoke)
// Do not remove these comments.
// branchkeeper/src/settings.rs
// src/settings.rs

// Branchkeeper's own preferences, saved through cosmic-config in our own
// namespace (io.github.hmrdsmoke.Branchkeeper), so they land next to every
// other COSMIC app's settings under ~/.config/cosmic/ and survive restarts.
//
// One knob so far:
//   - project_roots: the folders whose immediate subdirectories are projects.
//     Every folder one level down with a `.git` in it goes on the board,
//     open or not. Absolute paths from the folder picker, or `~/...`.
//
// Field names ARE the on-disk keys - don't rename them without a version bump.

use std::path::{Path, PathBuf};

use cosmic::cosmic_config::{
    self, Config, ConfigSet, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};

/// cosmic-config namespace owned by Branchkeeper.
pub const CONFIG_ID: &str = "io.github.hmrdsmoke.Branchkeeper";

/// Branchkeeper's saved preferences.
#[derive(Debug, Clone, PartialEq, Eq, CosmicConfigEntry)]
#[version = 1]
pub struct Settings {
    /// Folders to look in for projects. Default: `~/Projects`.
    pub project_roots: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            project_roots: vec!["~/Projects".to_owned()],
        }
    }
}

impl Settings {
    /// Load saved preferences, falling back per-field to the defaults. Never
    /// fails: a missing namespace, missing keys, or a version mismatch all
    /// degrade to defaults rather than taking the applet down.
    pub fn load() -> Self {
        match Config::new(CONFIG_ID, Self::VERSION) {
            Ok(config) => Self::get_entry(&config).unwrap_or_else(|(_, partial)| partial),
            Err(_) => Self::default(),
        }
    }

    /// Add a folder to the roots and save. A folder already in the list is
    /// left alone, so picking the same one twice changes nothing.
    pub fn add_root(&mut self, dir: &Path) {
        let dir = dir.to_string_lossy().into_owned();
        if self.project_roots.contains(&dir) {
            return;
        }
        self.project_roots.push(dir);
        self.save_roots();
    }

    /// Remove the root at `index` (a position in `project_roots`) and save.
    pub fn remove_root(&mut self, index: usize) {
        if index < self.project_roots.len() {
            self.project_roots.remove(index);
            self.save_roots();
        }
    }

    /// Write `project_roots` to disk. The in-memory copy is already updated;
    /// a failed write is logged and the applet carries on with what it has.
    fn save_roots(&self) {
        match Config::new(CONFIG_ID, Self::VERSION) {
            Ok(config) => {
                if let Err(why) = config.set("project_roots", &self.project_roots) {
                    eprintln!("branchkeeper: couldn't save project folders: {why}");
                }
            }
            Err(why) => eprintln!("branchkeeper: couldn't open settings: {why}"),
        }
    }
}

/// Turn a root as stored ("~/Projects" or an absolute path) into a real path.
/// `~` means the home directory; anything else is taken as-is.
pub fn expand_root(root: &str) -> PathBuf {
    if let Some(rest) = root.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(root)
}

/// The reverse, for display only: an absolute path under the home directory
/// shows as `~/...` so the settings list stays short.
pub fn display_root(root: &str) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Some(rest) = Path::new(root).strip_prefix(&home).ok()
        && !rest.as_os_str().is_empty()
    {
        return format!("~/{}", rest.display());
    }
    root.to_owned()
}
