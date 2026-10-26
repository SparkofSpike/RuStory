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
}

impl Document {
    /// Wrap a story that has not been saved anywhere yet.
    #[must_use]
    pub fn new(story: Story, path: impl Into<PathBuf>) -> Self {
        Self {
            story,
            path: path.into(),
        }
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
        let text = ron::ser::to_string_pretty(&self.story, ron::ser::PrettyConfig::default())
            .map_err(|err| EditError::Write {
                path: self.path.display().to_string(),
                message: err.to_string(),
            })?;

        // Round-trip check before touching the file, so a serializer bug cannot
        // destroy a script that was fine.
        if let Err(err) = rustory_script::parse_story(&text, "memory") {
            return Err(EditError::Write {
                path: self.path.display().to_string(),
                message: format!("refusing to write a file that would not load back: {err}"),
            });
        }

        std::fs::write(&self.path, text).map_err(|err| EditError::Write {
            path: self.path.display().to_string(),
            message: err.to_string(),
        })
    }

    /// Write to a specific path and adopt it.
    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), EditError> {
        self.path = path.as_ref().to_path_buf();
        self.save()
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
