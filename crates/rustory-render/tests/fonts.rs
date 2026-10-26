//! Fonts are the project's choice, not the engine's.
//!
//! RuStory ships no typefaces. A game that must render identically on machines it
//! has never seen brings its own; a game that is happy with platform defaults does
//! nothing. These tests cover the seam: can a project font be loaded, selected, and
//! does a wrong name fail loudly instead of silently rendering in the wrong face.

use rustory_render::{FontChoice, Renderer, TextItem};

fn renderer_or_skip() -> Option<Renderer> {
    match Renderer::new_headless(64, 64) {
        Ok(renderer) => Some(renderer),
        Err(err) => {
            eprintln!("skipping font test: {err}");
            None
        }
    }
}

/// Some font file that exists on this machine, if any.
///
/// Copied out of the system directory rather than embedded: the engine must not
/// carry a typeface, and a test that shipped one would quietly contradict that.
fn some_system_font() -> Option<std::path::PathBuf> {
    let dirs = [
        "C:/Windows/Fonts",
        "/usr/share/fonts",
        "/System/Library/Fonts",
        "/Library/Fonts",
    ];
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_ascii_lowercase);
            if matches!(extension.as_deref(), Some("ttf" | "otf")) {
                return Some(path);
            }
        }
    }
    None
}

#[test]
fn a_project_font_can_be_loaded_and_named() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };
    let Some(font) = some_system_font() else {
        eprintln!("skipping: no font file on this machine to load");
        return;
    };

    let families = renderer.load_font(&font).expect("loads");
    assert!(
        !families.is_empty(),
        "a loaded font must report at least one family name"
    );

    // The point of returning families: a creator gets the *internal* name, which is
    // what `FontChoice::Named` matches, not the filename they happened to have.
    for family in &families {
        assert!(
            renderer.has_family(family),
            "`{family}` should be available after loading its file"
        );
    }
}

#[test]
fn loading_a_missing_file_reports_the_path() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    let err = renderer
        .load_font("definitely/not/here.ttf")
        .expect_err("must fail");
    let message = err.to_string();
    assert!(
        message.contains("definitely/not/here.ttf"),
        "the message must name the file: {message}"
    );
}

#[test]
fn loading_a_file_that_is_not_a_font_is_refused() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    let dir = std::env::temp_dir().join("rustory-font-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("not-a-font.ttf");
    std::fs::write(&path, b"this is not a typeface").expect("write");

    let err = renderer.load_font(&path).expect_err("must refuse");
    assert!(
        err.to_string().contains("no usable face"),
        "unexpected error: {err}"
    );

    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_unknown_family_is_reported_rather_than_silently_ignored() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    assert!(
        !renderer.set_sans_serif_family("A Family That Does Not Exist"),
        "selecting a missing family must report failure"
    );
}

#[test]
fn an_installed_family_can_be_selected() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };

    // Whatever the platform offers first is by definition installed.
    let Some(family) = renderer.families().into_iter().next() else {
        eprintln!("skipping: this machine reports no fonts at all");
        return;
    };

    assert!(
        renderer.set_sans_serif_family(&family),
        "`{family}` came from the database, so selecting it must succeed"
    );
}

#[test]
fn a_directory_load_skips_junk_and_keeps_going() {
    let Some(mut renderer) = renderer_or_skip() else {
        return;
    };
    let Some(font) = some_system_font() else {
        eprintln!("skipping: no font file to copy");
        return;
    };

    let dir = std::env::temp_dir().join("rustory-font-dir-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");

    // One real font, one unreadable file with a font extension, one irrelevant file.
    std::fs::copy(&font, dir.join("real.ttf")).expect("copy font");
    std::fs::write(dir.join("broken.otf"), b"nope").expect("write junk");
    std::fs::write(dir.join("readme.txt"), b"ignore me").expect("write txt");

    let families = renderer.load_fonts_dir(&dir).expect("directory loads");
    assert!(
        !families.is_empty(),
        "the real font should still load despite the broken neighbour"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_text_item_defaults_to_the_platform_sans_serif() {
    // The engine must not pick a typeface on a project's behalf. Defaulting to the
    // generic family means a project without fonts renders with whatever the system
    // considers normal, rather than with something Hardcoded and wrong.
    assert_eq!(FontChoice::default(), FontChoice::SansSerif);
    let item = TextItem::new("hello", 0.0, 0.0, 20.0);
    assert_eq!(item.font, FontChoice::SansSerif);
}

#[test]
fn a_text_item_can_ask_for_a_named_family() {
    let item = TextItem::new("hello", 0.0, 0.0, 20.0).with_font(FontChoice::named("Agency FB"));
    assert_eq!(item.font, FontChoice::Named("Agency FB".into()));
}
