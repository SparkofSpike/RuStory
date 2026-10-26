//! Native file dialogs.
//!
//! # Why this is its own module
//!
//! `rfd` blocks the calling thread while a dialog is open, and that is correct: a
//! person choosing a file is not doing anything else. But blocking the UI thread is
//! exactly the thing you must not do casually, so every call site goes through here and
//! the blocking is confined to three functions with obvious names.
//!
//! It also means the dependency is replaceable. If `rfd` ever misbehaves on a platform,
//! swapping it out touches this file and nothing else — the rest of the editor only
//! knows "ask the user for a path".
//!
//! # Why filters
//!
//! A story is one file. Offering "all files" as the default invites picking a PNG and
//! then wondering why the parse error mentions angle brackets.

use std::path::{Path, PathBuf};

/// The extension stories use.
pub const STORY_EXTENSION: &str = "ron";

/// Ask for a story to open.
///
/// Returns `None` when the user cancels, which is not an error — cancelling a dialog
/// must never surface as a message.
#[must_use]
pub fn open_story() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open story")
        .add_filter("RuStory script", &[STORY_EXTENSION])
        .pick_file()
}

/// Ask where to write a story.
///
/// `suggested` seeds the file name, so "save as" on `chapter-2.ron` proposes
/// `chapter-2.ron` in the new directory rather than a blank field.
#[must_use]
pub fn save_story_as(suggested: Option<&Path>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save story")
        .add_filter("RuStory script", &[STORY_EXTENSION]);

    if let Some(path) = suggested {
        if let Some(dir) = path.parent()
            && dir.is_dir()
        {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            dialog = dialog.set_file_name(name);
        }
    }

    let picked = dialog.save_file()?;

    // A file dialog that does not append the filter extension leaves the user with a
    // file their own engine will refuse to open by default. Adding it is friendlier
    // than complaining later.
    Some(ensure_extension(picked))
}

/// Ask the user to confirm losing unsaved work.
///
/// Native confirm dialogs are synchronous and modal, so this can simply return a bool.
#[must_use]
pub fn confirm_discard(story_name: &str) -> bool {
    rfd::MessageDialog::new()
        .set_title("Unsaved changes")
        .set_description(format!(
            "`{story_name}` has changes that have not been saved.\n\nDiscard them?"
        ))
        .set_level(rfd::MessageLevel::Warning)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show()
        == rfd::MessageDialogResult::Yes
}

/// Append the story extension when the dialog did not.
#[must_use]
pub fn ensure_extension(mut path: PathBuf) -> PathBuf {
    let has_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(STORY_EXTENSION));

    if !has_extension {
        // `set_extension` replaces, so only touch the name when there is nothing
        // there — `chapter-1.draft` should become `chapter-1.draft.ron`, not
        // `chapter-1.ron`.
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("story")
            .to_owned();
        path.set_file_name(format!("{name}.{STORY_EXTENSION}"));
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extension_is_added_when_missing() {
        assert_eq!(
            ensure_extension(PathBuf::from("story")),
            PathBuf::from("story.ron")
        );
    }

    #[test]
    fn an_existing_story_extension_is_left_alone() {
        assert_eq!(
            ensure_extension(PathBuf::from("story.ron")),
            PathBuf::from("story.ron")
        );
        // Case-insensitively: Windows will hand back `.RON` from a rename.
        assert_eq!(
            ensure_extension(PathBuf::from("story.RON")),
            PathBuf::from("story.RON")
        );
    }

    #[test]
    fn a_different_extension_is_kept_and_the_story_one_appended() {
        // `chapter.draft` must not become `chapter.ron`: the author named it that.
        assert_eq!(
            ensure_extension(PathBuf::from("chapter.draft")),
            PathBuf::from("chapter.draft.ron")
        );
    }

    #[test]
    fn a_path_in_a_directory_survives_the_rewrite() {
        let path = ensure_extension(PathBuf::from("stories/chapter-1"));
        assert_eq!(path, PathBuf::from("stories/chapter-1.ron"));
    }
}
