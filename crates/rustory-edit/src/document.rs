//! The document being edited.
//!
//! A [`Story`] plus the file it came from. The path matters because an editor's most
//! important verb is "save", and a save that writes somewhere other than where the
//! file was read is how work gets lost.

use std::path::{Path, PathBuf};

use rustory_core::Story;

use crate::error::EditError;

/// A story and where it lives.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    story: Story,
    path: PathBuf,
    /// Whether this document has ever been written.
    ///
    /// A new story has no file yet, and `save` must not invent one: writing to a
    /// guessed path is how work ends up somewhere the author never looks. Until a
    /// path is chosen, `save` refuses and the caller asks.
    saved: bool,
}

impl Document {
    /// Wrap a story that already has a home on disk.
    ///
    /// Used by [`Document::open`] and by tests. A document created from scratch
    /// should use [`Document::untitled`], which knows it has never been written.
    #[must_use]
    pub fn new(story: Story, path: impl Into<PathBuf>) -> Self {
        Self {
            story,
            path: path.into(),
            saved: false,
        }
    }

    /// A story that has never been written anywhere.
    ///
    /// The path is a placeholder for display only; `save` refuses until
    /// [`Document::save_as`] gives it a real one.
    #[must_use]
    pub fn untitled(story: Story) -> Self {
        Self {
            story,
            path: PathBuf::from("untitled.ron"),
            saved: false,
        }
    }

    /// Whether the document has ever been written to disk.
    #[must_use]
    pub fn has_file(&self) -> bool {
        self.saved
    }

    /// Read a story from disk.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EditError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|err| EditError::Io {
            path: path.display().to_string(),
            message: err.to_string(),
        })?;

        let story = rustory_script::parse_story(&text, &path.display().to_string()).map_err(
            |err| match err {
                rustory_script::ScriptError::Parse {
                    path,
                    line,
                    column,
                    message,
                } => EditError::Parse {
                    path,
                    line,
                    column,
                    message,
                },
                other => EditError::Io {
                    path: path.display().to_string(),
                    message: other.to_string(),
                },
            },
        )?;

        Ok(Self {
            story,
            path: path.to_path_buf(),
            saved: true,
        })
    }

    #[must_use]
    pub fn story(&self) -> &Story {
        &self.story
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Point the document at a different file, for "save as".
    pub fn set_path(&mut self, path: impl Into<PathBuf>) {
        self.path = path.into();
        self.saved = true;
    }

    /// Replace the story wholesale. Used by the editor after a successful operation
    /// and by undo.
    pub(crate) fn replace_story(&mut self, story: Story) {
        self.story = story;
    }

    /// Write the story back in the format it was read from.
    ///
    /// Written through the same RON serializer the loader accepts, and re-parsed
    /// before returning: a save that produces a file the engine cannot open would be
    /// the worst possible outcome, and re-parsing costs microseconds.
    pub fn save(&self) -> Result<(), EditError> {
        if !self.saved {
            return Err(EditError::Write {
                path: self.path.display().to_string(),
                message: "this story has no file yet; choose where to save it".to_owned(),
            });
        }

        // Write, then confirm the file loads back. A save that produces a file the
        // engine cannot open would be the worst possible outcome; re-parsing costs
        // microseconds.
        self.write_to(&self.path)
    }

    fn write_to(&self, path: &Path) -> Result<(), EditError> {
        let text = ron::ser::to_string_pretty(&self.story, ron::ser::PrettyConfig::default())
            .map_err(|err| EditError::Write {
                path: path.display().to_string(),
                message: err.to_string(),
            })?;

        if let Err(err) = rustory_script::parse_story(&text, "memory") {
            return Err(EditError::Write {
                path: path.display().to_string(),
                message: format!("refusing to write a file that would not load back: {err}"),
            });
        }

        std::fs::write(path, text).map_err(|err| EditError::Write {
            path: path.display().to_string(),
            message: err.to_string(),
        })
    }

    /// Write to a specific path and adopt it.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), EditError> {
        let path = path.as_ref().to_path_buf();
        self.write_to(&path)?;
        // Adopted only after the write succeeded, so a failed save-as leaves the
        // document pointing at whatever it was pointing at before.
        self.path = path;
        self.saved = true;
        Ok(())
    }

    /// The RON text this document would write, without writing it.
    ///
    /// The GUI uses it for a live "what will be saved" view; the CLI uses it for
    /// `--dry-run`.
    pub fn to_ron(&self) -> Result<String, EditError> {
        ron::ser::to_string_pretty(&self.story, ron::ser::PrettyConfig::default()).map_err(|err| {
            EditError::Write {
                path: self.path.display().to_string(),
                message: err.to_string(),
            }
        })
    }
}
