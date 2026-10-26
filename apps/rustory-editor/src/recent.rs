//! The recent-files list.
//!
//! # Why this is not an operation
//!
//! `rustory-edit` owns the story, not the machine. Which files a person opened last
//! week is a property of the workstation, not of the document, and it has to survive a
//! crash mid-edit — so it lives in the app, beside the dialogs, and never enters the
//! undo history.
//!
//! # Where it is stored
//!
//! `%APPDATA%\rustory\recent.json` on Windows, `$XDG_CONFIG_HOME/rustory/recent.json`
//! (falling back to `~/.config`) elsewhere. When no such directory is known,
//! [`store_path`] returns `None` and the list still works for the session — memory
//! instead of a guess at a location.
//!
//! # What it tolerates
//!
//! A corrupt or hand-edited file loads as an empty list, because a broken convenience
//! feature must never stop the program from opening. Entries are compared
//! case-insensitively on Windows, where `Story.RON` and `story.ron` are the same file,
//! and are stored absolute so a list written from one working directory still resolves
//! from another.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many files are remembered. Long enough to be useful, short enough to scan.
const CAPACITY: usize = 10;

/// Format version, so a future change can tell old files from new ones.
const VERSION: u32 = 1;

/// The on-disk shape. Kept separate from [`RecentFiles`] so the file format can change
/// without changing how the rest of the editor holds the list.
#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    files: Vec<String>,
}

/// Files opened recently, newest first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

impl RecentFiles {
    /// Load the list, or an empty one when there is nowhere to read from.
    #[must_use]
    pub fn load() -> Self {
        match store_path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    /// Load from a specific file. Missing or malformed reads as empty.
    #[must_use]
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(stored) = serde_json::from_str::<Stored>(&text) else {
            // Not an error worth a dialog: the user's problem is "the menu is empty",
            // and the fix is to open a file, which rewrites this file anyway.
            return Self::default();
        };

        let mut recent = Self::default();
        for file in stored.files {
            let file = PathBuf::from(file);
            // A hand-edited file can repeat an entry; the menu must not.
            if recent
                .paths
                .iter()
                .any(|existing| same_path(existing, &file))
            {
                continue;
            }
            if recent.paths.len() == CAPACITY {
                break;
            }
            recent.paths.push(file);
        }
        recent
    }

    /// Remember a file as the most recent. Returns whether the list changed, so a
    /// caller can skip a pointless write.
    pub fn record(&mut self, path: &Path) -> bool {
        let path = absolute(path);
        let before = self.paths.clone();
        // Remove first, then insert: reopening the fifth file must move it to the top,
        // not leave a second copy of it behind.
        self.paths.retain(|existing| !same_path(existing, &path));
        self.paths.insert(0, path);
        self.paths.truncate(CAPACITY);
        self.paths != before
    }

    /// Drop one entry, for a file the user has since deleted.
    pub fn forget(&mut self, path: &Path) -> bool {
        let before = self.paths.len();
        self.paths.retain(|existing| !same_path(existing, path));
        self.paths.len() != before
    }

    /// Keep only the entries `exists` accepts.
    ///
    /// Separated from [`prune`](Self::prune) so the decision can be tested without
    /// touching a filesystem.
    pub fn prune_where(&mut self, exists: impl Fn(&Path) -> bool) -> bool {
        let before = self.paths.len();
        self.paths.retain(|path| exists(path));
        self.paths.len() != before
    }

    /// Drop entries whose files are gone.
    pub fn prune(&mut self) -> bool {
        self.prune_where(Path::exists)
    }

    /// Empty the list. Returns whether there was anything to clear.
    pub fn clear(&mut self) -> bool {
        let had = !self.paths.is_empty();
        self.paths.clear();
        had
    }

    /// The entries, newest first.
    #[must_use]
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Whether the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// Write the list where [`store_path`] says it belongs.
    ///
    /// No configured location is not an error — the list simply is not persisted.
    /// A real write failure is returned, because a status line can say so once without
    /// interrupting the work.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = store_path() else {
            return Ok(());
        };
        self.save_to(&path)
    }

    /// Write to a specific file, creating its directory.
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }

        let stored = Stored {
            version: VERSION,
            files: self
                .paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        };
        let text = serde_json::to_string_pretty(&stored).map_err(io::Error::other)?;
        std::fs::write(path, text)
    }
}

/// Where the list is stored, if this machine has a configuration directory.
#[must_use]
pub fn store_path() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    Some(base.join("rustory").join("recent.json"))
}

/// Render the list for a terminal, one path per line, or as JSON with `json`.
///
/// Kept here rather than in `main` so the agent-facing output has a test.
#[must_use]
pub fn render(paths: &[PathBuf], json: bool) -> String {
    if json {
        let stored = Stored {
            version: VERSION,
            files: paths
                .iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect(),
        };
        // `to_string_pretty` cannot fail for this shape; an empty object is a better
        // answer than a panic if it somehow did.
        return serde_json::to_string_pretty(&stored).unwrap_or_else(|_| "{}".to_owned());
    }

    let mut out = String::new();
    for path in paths {
        out.push_str(&path.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Make a path absolute without resolving links: the file may not exist any more, and
/// `canonicalize` fails on those.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    let key = |path: &Path| {
        absolute(path)
            .to_string_lossy()
            .replace('/', "\\")
            .to_lowercase()
    };
    key(a) == key(b)
}

#[cfg(not(windows))]
fn same_path(a: &Path, b: &Path) -> bool {
    absolute(a) == absolute(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory for one test, removed when it drops.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("rustory-recent-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("scratch directory");
            Self { path }
        }

        fn join(&self, name: &str) -> PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn a_recorded_file_comes_first() {
        let mut recent = RecentFiles::default();
        recent.record(Path::new("a.ron"));
        recent.record(Path::new("b.ron"));
        assert_eq!(recent.paths()[0], absolute(Path::new("b.ron")));
        assert_eq!(recent.paths()[1], absolute(Path::new("a.ron")));
    }

    #[test]
    fn reopening_a_file_moves_it_up_without_duplicating_it() {
        let mut recent = RecentFiles::default();
        recent.record(Path::new("a.ron"));
        recent.record(Path::new("b.ron"));
        recent.record(Path::new("a.ron"));
        assert_eq!(recent.paths().len(), 2);
        assert_eq!(recent.paths()[0], absolute(Path::new("a.ron")));
    }

    #[test]
    fn the_same_path_written_differently_is_one_entry() {
        // The two spellings differ only in separator and case.
        let mut recent = RecentFiles::default();
        recent.record(Path::new("stories/chapter.ron"));
        recent.record(Path::new("stories\\CHAPTER.ron"));
        let expected = if cfg!(windows) { 1 } else { 2 };
        assert_eq!(recent.paths().len(), expected);
    }

    #[test]
    fn the_list_stops_at_capacity() {
        let mut recent = RecentFiles::default();
        for index in 0..CAPACITY + 5 {
            recent.record(&PathBuf::from(format!("story-{index}.ron")));
        }
        assert_eq!(recent.paths().len(), CAPACITY);
        assert_eq!(
            recent.paths()[0],
            absolute(Path::new(&format!("story-{}.ron", CAPACITY + 4)))
        );
    }

    #[test]
    fn recording_the_first_entry_again_reports_no_change() {
        let mut recent = RecentFiles::default();
        assert!(recent.record(Path::new("a.ron")));
        assert!(!recent.record(Path::new("a.ron")));
    }

    #[test]
    fn a_missing_file_loads_empty() {
        let scratch = Scratch::new("missing");
        assert!(RecentFiles::load_from(&scratch.join("nope.json")).is_empty());
    }

    #[test]
    fn a_corrupt_file_loads_empty_instead_of_failing() {
        let scratch = Scratch::new("corrupt");
        let path = scratch.join("recent.json");
        std::fs::write(&path, "{ this is not json").expect("write");
        assert!(RecentFiles::load_from(&path).is_empty());
    }

    #[test]
    fn saving_and_loading_round_trips_the_order() {
        let scratch = Scratch::new("round-trip");
        let path = scratch.join("nested/recent.json");

        let mut recent = RecentFiles::default();
        recent.record(Path::new("a.ron"));
        recent.record(Path::new("b.ron"));
        recent.save_to(&path).expect("save");

        let loaded = RecentFiles::load_from(&path);
        assert_eq!(loaded.paths(), recent.paths());
    }

    #[test]
    fn a_hand_edited_file_cannot_exceed_capacity() {
        let scratch = Scratch::new("overflow");
        let path = scratch.join("recent.json");
        let files: Vec<String> = (0..CAPACITY + 3)
            .map(|index| format!("story-{index}.ron"))
            .collect();
        std::fs::write(
            &path,
            serde_json::to_string(&Stored {
                version: VERSION,
                files,
            })
            .expect("serialise"),
        )
        .expect("write");

        assert_eq!(RecentFiles::load_from(&path).paths().len(), CAPACITY);
    }

    #[test]
    fn pruning_keeps_only_what_exists() {
        let mut recent = RecentFiles::default();
        recent.record(Path::new("keep.ron"));
        recent.record(Path::new("gone.ron"));

        let kept = recent.prune_where(|path| path.ends_with("keep.ron"));
        assert!(kept);
        assert_eq!(recent.paths(), [absolute(Path::new("keep.ron"))]);
    }

    #[test]
    fn forgetting_removes_one_entry() {
        let mut recent = RecentFiles::default();
        recent.record(Path::new("a.ron"));
        recent.record(Path::new("b.ron"));

        assert!(recent.forget(Path::new("a.ron")));
        assert_eq!(recent.paths().len(), 1);
        assert!(!recent.forget(Path::new("a.ron")));
    }

    #[test]
    fn clearing_reports_whether_anything_went() {
        let mut recent = RecentFiles::default();
        recent.record(Path::new("a.ron"));
        assert!(recent.clear());
        assert!(!recent.clear());
    }

    #[test]
    fn terminal_rendering_is_one_path_per_line() {
        let paths = [PathBuf::from("a.ron"), PathBuf::from("b.ron")];
        assert_eq!(render(&paths, false), "a.ron\nb.ron\n");
    }

    #[test]
    fn json_rendering_round_trips_through_the_loader() {
        let paths = [PathBuf::from("a.ron")];
        let text = render(&paths, true);
        let scratch = Scratch::new("json");
        let file = scratch.join("recent.json");
        std::fs::write(&file, text).expect("write");

        assert_eq!(RecentFiles::load_from(&file).paths(), paths);
    }
}
