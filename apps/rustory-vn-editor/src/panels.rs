//! The panels.
//!
//! Each function lays out one region and pushes [`EditOp`]s into `app.pending` rather
//! than touching the story. That is the whole contract: panels describe intent, the
//! app applies it.
//!
//! Forms edit a *copy* of the value being changed, held in a local, and emit an
//! operation only when it differs. This keeps typing responsive — applying an
//! operation on every keystroke would push one undo step per character.

use rustory_vn_core::{Beat, Exit, NodeId, Slot};
use rustory_vn_edit::prelude::*;

use crate::app::{EditorApp, UiAction};
use crate::preview::{PreviewReport, Step};
use crate::tab::Tab;

/// The config as text for the box, or an empty string when there is none.
///
/// Delegated to the operation layer, which owns the text form: the editor should not know
/// what a `PrettyConfig` is, and one parser means the GUI and the CLI refuse the same text.
fn config_text(spec: &GameplaySpec) -> String {
    rustory_vn_edit::spec_to_ron(spec)
}

/// Whether a node id matches the filter text. Case-insensitive, trimmed, and an empty
/// filter matches everything.
///
/// Substring rather than prefix because authors think in the middle of a name as often
/// as at the start (`half_honest` when looking for `honest`).
fn matches_filter(id: &NodeId, filter: &str) -> bool {
    let needle = filter.trim().to_lowercase();
    needle.is_empty() || id.to_string().to_lowercase().contains(&needle)
}

/// Menu bar, title, undo/redo, preview controls.
pub fn top_bar(app: &mut EditorApp, ui: &mut egui::Ui) {
    egui::Panel::top("top").show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading("RuStory");
            ui.separator();

            let has_doc = !app.tabs.is_empty();

            if ui
                .button("New")
                .on_hover_text("New story in a new tab (Ctrl+N)")
                .clicked()
            {
                app.pending_actions.push(UiAction::New);
            }
            if ui
                .button("Open…")
                .on_hover_text("Open a story (Ctrl+O)")
                .clicked()
            {
                app.pending_actions.push(UiAction::Open);
            }

            // The recent list sits next to Open because it answers the same question —
            // "which story?" — with one click instead of a dialog.
            let recent = app.recent.paths().to_vec();
            ui.menu_button("Recent", |ui| {
                // Without a floor the menu shrinks to the longest button, which puts the
                // text hard against the edge and makes the whole list look like an
                // afterthought next to the other buttons.
                ui.set_min_width(240.0);
                if recent.is_empty() {
                    ui.weak("No recent stories yet");
                }
                for path in &recent {
                    let name = path.file_name().map_or_else(
                        || path.display().to_string(),
                        |name| name.to_string_lossy().into_owned(),
                    );
                    // Hover shows the directory: two chapters can share a file name.
                    if ui
                        .button(name)
                        .on_hover_text(path.display().to_string())
                        .clicked()
                    {
                        app.pending_actions.push(UiAction::OpenAt(path.clone()));
                        ui.close();
                    }
                }
                if !recent.is_empty() {
                    ui.separator();
                    if ui.button("Clear list").clicked() {
                        app.pending_actions.push(UiAction::ClearRecent);
                        ui.close();
                    }
                }
            });

            ui.add_enabled_ui(has_doc, |ui| {
                if ui.button("Save").on_hover_text("Save (Ctrl+S)").clicked() {
                    app.pending_actions.push(UiAction::Save);
                }
                if ui
                    .button("Save as…")
                    .on_hover_text("Save to a new file (Ctrl+Shift+S)")
                    .clicked()
                {
                    app.pending_actions.push(UiAction::SaveAs(app.active));
                }
            });

            ui.separator();

            let active = app.tabs.get(app.active);
            let (can_undo, can_redo, undo_label, redo_label) = active
                .map(|tab| {
                    (
                        tab.editor.can_undo(),
                        tab.editor.can_redo(),
                        tab.editor.undo_label().unwrap_or("").to_owned(),
                        tab.editor.redo_label().unwrap_or("").to_owned(),
                    )
                })
                .unwrap_or((false, false, String::new(), String::new()));

            if ui
                .add_enabled(can_undo, egui::Button::new("Undo"))
                .on_hover_text(if undo_label.is_empty() {
                    "Ctrl+Z".to_owned()
                } else {
                    format!("Undo {undo_label} (Ctrl+Z)")
                })
                .clicked()
            {
                app.undo_pending();
            }
            if ui
                .add_enabled(can_redo, egui::Button::new("Redo"))
                .on_hover_text(if redo_label.is_empty() {
                    "Ctrl+Y".to_owned()
                } else {
                    format!("Redo {redo_label} (Ctrl+Y)")
                })
                .clicked()
            {
                app.redo_pending();
            }

            // A shortcut nobody can find is not a shortcut. The list is short enough to show
            // in full, which beats a tooltip per button.
            if ui
                .button("?")
                .on_hover_text("Keys and controls (F1)")
                .clicked()
            {
                app.show_help = !app.show_help;
            }

            // Preview position, right-aligned so it does not shift with the toolbar.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let Some(tab) = app.tabs.get_mut(app.active) else {
                    ui.label(egui::RichText::new("No document open").weak());
                    return;
                };
                let position = tab.preview_steps.len();

                if ui.button(">>").on_hover_text("Step forward").clicked() {
                    tab.preview_steps.push(Step::Click);
                }
                // The slider rewinds along the path already taken; forward is the button,
                // because how long the story is only becomes known by playing it.
                let mut target = position;
                if ui
                    .add(egui::Slider::new(&mut target, 0..=position.max(1)).text("step"))
                    .changed()
                {
                    tab.preview_steps.truncate(target);
                }
                if ui.button("<<").on_hover_text("Reset preview").clicked() {
                    tab.preview_steps.clear();
                }
                ui.label("Preview:");
            });
        });
    });
}

/// The tab strip.
///
/// Between the toolbar and the panels rather than inside either: it is about the window, and
/// putting it in a side panel would make it look like a property of the story.
pub fn tab_bar(app: &mut EditorApp, ui: &mut egui::Ui) {
    if app.tabs.is_empty() {
        return;
    }

    egui::Panel::top("tabs").show(ui, |ui| {
        egui::ScrollArea::horizontal()
            .id_salt("tab-strip")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Collected rather than acted on inside the loop: closing a tab while
                    // iterating over the tabs is the classic way to skip one.
                    let mut activate: Option<usize> = None;
                    let mut close: Option<usize> = None;

                    for (index, tab) in app.tabs.iter().enumerate() {
                        let selected = index == app.active;
                        let dirty = if tab.is_dirty() { " •" } else { "" };
                        let label = format!("{}{dirty}", tab.name());

                        // The close affordance sits inside the tab, the way a browser does
                        // it: a separate column of buttons would waste a row for every tab.
                        ui.horizontal(|ui| {
                            if ui
                                .selectable_label(selected, label)
                                .on_hover_text(tab.path().display().to_string())
                                .clicked()
                            {
                                activate = Some(index);
                            }
                            if ui
                                .small_button("×")
                                .on_hover_text("Close (Ctrl+W)")
                                .clicked()
                            {
                                close = Some(index);
                            }
                        });
                    }

                    if let Some(index) = activate {
                        app.active = index;
                        app.set_status(format!("Editing {}", app.tabs[index].name()), false);
                    }
                    if let Some(index) = close {
                        app.pending_actions.push(UiAction::Close(index));
                    }
                });
            });
    });
}

/// Bottom status strip.
pub fn status_bar(app: &mut EditorApp, ui: &mut egui::Ui) {
    egui::Panel::bottom("status").show(ui, |ui| {
        ui.horizontal(|ui| {
            let colour = if app.status_is_error {
                egui::Color32::from_rgb(240, 120, 120)
            } else {
                ui.style().visuals.text_color()
            };
            ui.colored_label(colour, &app.status);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let problems = app
                    .tabs
                    .get(app.active)
                    .map(|tab| tab.editor.problems().len());
                match problems {
                    Some(0) => {
                        ui.colored_label(egui::Color32::from_rgb(140, 200, 140), "valid");
                    }
                    Some(count) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(240, 180, 100),
                            format!("{count} problem(s)"),
                        );
                    }
                    // No document: say nothing rather than "valid", which would be a claim
                    // about a story that is not open.
                    None => {}
                }
            });
        });
    });
}

/// Node list: pick, create, rename, duplicate, delete.
pub fn node_list(app: &mut EditorApp, ui: &mut egui::Ui) {
    egui::Panel::left("nodes")
        .resizable(true)
        .default_size(240.0)
        .show(ui, |ui| {
            ui.heading("Nodes");
            ui.separator();

            let active = app.active;
            let Some(tab) = app.tabs.get_mut(active) else {
                ui.label("No story open.");
                ui.label(
                    egui::RichText::new(
                        "New or Open to start; several stories can be open at once.",
                    )
                    .small()
                    .weak(),
                );
                return;
            };

            // A story with thirty nodes is already a list you scan rather than read, and
            // scanning is what a filter is for. Typing here does not edit the document,
            // so it takes no undo step and cannot fail.
            ui.add(
                egui::TextEdit::singleline(&mut tab.node_filter)
                    .hint_text("filter nodes")
                    .desired_width(f32::INFINITY),
            );

            let story = tab.editor.story();
            let start = story.start.clone();
            let total = story.nodes.len();
            let ids: Vec<NodeId> = story
                .nodes
                .keys()
                .filter(|id| matches_filter(id, &tab.node_filter))
                .cloned()
                .collect();

            if ids.len() != total {
                ui.label(
                    egui::RichText::new(format!("{} of {total}", ids.len()))
                        .small()
                        .weak(),
                );
            }
            if ids.is_empty() && !tab.node_filter.trim().is_empty() {
                ui.label(egui::RichText::new("No node matches.").weak());
            }

            // Collected rather than applied inside the scroll area: setting the selection
            // resets the preview, and the closure still holds `story`.
            let mut select: Option<NodeId> = None;
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 130.0)
                .show(ui, |ui| {
                    for id in &ids {
                        let node = &story.nodes[id];
                        let is_start = id == &start;
                        let selected = tab.selected.as_ref() == Some(id);

                        let label = if is_start {
                            format!("▶ {id}  ({})", node.beats.len())
                        } else {
                            format!("   {id}  ({})", node.beats.len())
                        };

                        if ui
                            .selectable_label(selected, label)
                            .on_hover_text(describe_node(node))
                            .clicked()
                        {
                            select = Some(id.clone());
                            tab.rename_buffer = id.to_string();
                        }
                    }
                });

            ui.separator();

            // Create
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut tab.new_node_id)
                        .hint_text("new node id")
                        .desired_width(140.0),
                );
                let can_create = !tab.new_node_id.trim().is_empty();
                if ui.add_enabled(can_create, egui::Button::new("+")).clicked() {
                    let id = NodeId::from(tab.new_node_id.trim());
                    tab.pending.push(EditOp::CreateNode { id, at: None });
                    tab.new_node_id.clear();
                }
            });

            // Rename / duplicate / delete the selection
            if let Some(selected) = tab.selected.clone() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !is_start(&selected, &tab.editor),
                            egui::Button::new("Delete"),
                        )
                        .on_hover_text("Delete the selected node")
                        .clicked()
                    {
                        tab.pending.push(EditOp::DeleteNode {
                            id: selected.clone(),
                        });
                    }

                    if ui.button("Duplicate").clicked() {
                        let as_id = NodeId::from(format!("{}-copy", selected));
                        tab.pending.push(EditOp::DuplicateNode {
                            id: selected.clone(),
                            as_id,
                        });
                    }

                    if ui.button("Make start").clicked() {
                        tab.pending.push(EditOp::SetStartNode {
                            node: selected.clone(),
                        });
                    }
                });

                ui.horizontal(|ui| {
                    if tab.rename_buffer.is_empty() {
                        tab.rename_buffer = selected.to_string();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut tab.rename_buffer)
                            .hint_text("rename to")
                            .desired_width(140.0),
                    );
                    let target = tab.rename_buffer.trim().to_owned();
                    let can_rename = !target.is_empty() && target != selected.to_string();
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Rename"))
                        .clicked()
                    {
                        tab.pending.push(EditOp::RenameNode {
                            from: selected.clone(),
                            to: NodeId::from(target),
                        });
                    }
                });
            }

            // Applied last: the selection lives on the same tab the loop above borrowed from,
            // so it is written through two fields rather than a method that would take both.
            if let Some(id) = select {
                Tab::plan_selection(&mut tab.selected, &mut tab.preview_steps, id);
            }
        });
}

fn is_start(id: &NodeId, editor: &Editor) -> bool {
    &editor.story().start == id
}

fn describe_node(node: &rustory_vn_core::Node) -> String {
    let exit = match &node.exit {
        Exit::End { .. } => "ends".to_owned(),
        Exit::Goto { to } => format!("goto {to}"),
        Exit::Choose { choices, .. } => format!("{} choice(s)", choices.len()),
        Exit::Gameplay { spec } => format!("gameplay {}", spec.system),
    };
    format!("{} beat(s), {exit}", node.beats.len())
}

/// Document-level fields: title, author, and which node the story starts at.
///
/// Collapsible, and collapsed by default: authors spend their time in the beat list, and
/// a form they never touch should not take vertical space from the one they use.
/// Takes its own field borrows rather than `&mut EditorApp`: the caller holds an
/// immutable borrow of the document, and taking the whole struct would collide with it.
fn story_settings(pending: &mut Vec<EditOp>, show: &mut bool, story: &Story, ui: &mut egui::Ui) {
    let header = ui.horizontal(|ui| {
        let arrow = if *show { "v" } else { ">" };
        if ui
            .button(format!("{arrow} Story"))
            .on_hover_text("Title, author and start node")
            .clicked()
        {
            *show = !*show;
        }

        // A one-line summary, so the header says something even when collapsed.
        let title = story.meta.title.as_deref().unwrap_or("(untitled)");
        ui.label(egui::RichText::new(title).weak());
        ui.label(
            egui::RichText::new(format!("starts at {}", story.start))
                .small()
                .weak(),
        );
    });
    let _ = header;

    if !*show {
        ui.separator();
        return;
    }

    ui.group(|ui| {
        // `Some("")` and `None` are different states: the first is a field being typed
        // into, the second is "no title". A checkbox controls which, so clearing a field
        // does not silently delete the key from the file.
        let mut title = story.meta.title.clone().unwrap_or_default();
        let mut has_title = story.meta.title.is_some();
        ui.horizontal(|ui| {
            if ui.checkbox(&mut has_title, "title").changed() {
                pending.push(EditOp::SetTitle {
                    title: if has_title { Some(title.clone()) } else { None },
                });
            }
            if has_title
                && ui
                    .add(
                        egui::TextEdit::singleline(&mut title)
                            .desired_width(320.0)
                            .hint_text("story title"),
                    )
                    .changed()
            {
                pending.push(EditOp::SetTitle {
                    title: Some(title.clone()),
                });
            }
        });

        let mut author = story.meta.author.clone().unwrap_or_default();
        let mut has_author = story.meta.author.is_some();
        ui.horizontal(|ui| {
            if ui.checkbox(&mut has_author, "author").changed() {
                pending.push(EditOp::SetAuthor {
                    author: if has_author {
                        Some(author.clone())
                    } else {
                        None
                    },
                });
            }
            if has_author
                && ui
                    .add(
                        egui::TextEdit::singleline(&mut author)
                            .desired_width(320.0)
                            .hint_text("author"),
                    )
                    .changed()
            {
                pending.push(EditOp::SetAuthor {
                    author: Some(author.clone()),
                });
            }
        });

        ui.horizontal(|ui| {
            ui.label("start node");
            let mut start = story.start.to_string();
            let ids: Vec<String> = story.nodes.keys().map(|id| id.to_string()).collect();
            egui::ComboBox::from_id_salt("story start")
                .selected_text(&start)
                .show_ui(ui, |ui| {
                    for id in &ids {
                        if ui.selectable_value(&mut start, id.clone(), id).clicked()
                            && start != story.start.to_string()
                        {
                            pending.push(EditOp::SetStartNode {
                                node: NodeId::from(start.clone()),
                            });
                        }
                    }
                });
        });
    });

    ui.separator();
}

/// Beat list and exit editor for the selected node.
pub fn beat_editor(app: &mut EditorApp, ui: &mut egui::Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        let active = app.active;
        let Some(tab) = app.tabs.get_mut(active) else {
            ui.heading("RuStory Editor");
            ui.label("Open a story to start editing.");
            ui.label(
                egui::RichText::new(
                    "New or Open in the toolbar; several stories can be open at once.",
                )
                .small()
                .weak(),
            );
            return;
        };

        let Some(selected) = tab.selected_node() else {
            ui.label("Select a node.");
            return;
        };

        let Some(node) = tab.editor.story().nodes.get(&selected) else {
            ui.label("That node no longer exists.");
            return;
        };

        ui.heading(format!("{selected}"));
        ui.separator();

        // Collected rather than applied inside the scroll area: changing the selection resets
        // the preview, and the closure still holds the story.
        let mut select: Option<NodeId> = None;
        // Same idea for a message that has to reach the status line: `set_status` takes the
        // whole app, which this closure has already borrowed the tab from.
        let mut status_after: Option<(String, bool)> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            // Who points at this node. Deletion refuses while anything does, and the
            // author is better served knowing that before the attempt than by reading it
            // in an error afterwards.
            let referrers = tab.editor.referrers(&selected);
            if !referrers.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Referenced by:");
                    for reference in &referrers {
                        match reference.source() {
                            Some(source) => {
                                if ui
                                    .small_button(source.to_string())
                                    .on_hover_text(reference.describe())
                                    .clicked()
                                {
                                    select = Some(source.clone());
                                }
                            }
                            // The start node is a reference from the story, not from
                            // another node, so there is nothing to click.
                            None => {
                                ui.label(egui::RichText::new(reference.describe()).weak());
                            }
                        }
                    }
                });
                ui.separator();
            }

            // --- story settings ---
            //
            // `SetTitle` and `SetAuthor` existed as operations before they had a control
            // here, which meant the CLI could do something the GUI could not — the exact
            // drift the operation layer is supposed to make impossible. A periodic audit
            // that greps for each `EditOp` variant in the editor source is what caught it;
            // doing that by eye would not have.
            story_settings(
                &mut tab.pending,
                &mut tab.show_story_settings,
                tab.editor.story(),
                ui,
            );

            // --- beats ---
            ui.label("Beats");
            let beat_count = node.beats.len();

            for index in 0..beat_count {
                let beat = &node.beats[index];
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{index}"));
                        ui.separator();

                        // Insert a copy right after this beat. The row of buttons below the
                        // list can only append, and moving a new beat up from the end is the
                        // kind of busywork that makes an editor feel like a form to fill in.
                        if ui
                            .button("+")
                            .on_hover_text("Insert a copy after this beat")
                            .clicked()
                        {
                            tab.pending.push(EditOp::InsertBeat {
                                node: selected.clone(),
                                index: index + 1,
                                beat: beat.clone(),
                            });
                        }

                        // Move up / down / delete
                        if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                            tab.pending.push(EditOp::MoveBeat {
                                node: selected.clone(),
                                from: index,
                                to: index - 1,
                            });
                        }
                        if ui
                            .add_enabled(index + 1 < beat_count, egui::Button::new("Dn"))
                            .clicked()
                        {
                            tab.pending.push(EditOp::MoveBeat {
                                node: selected.clone(),
                                from: index,
                                to: index + 2,
                            });
                        }
                        if ui.button("Del").on_hover_text("Delete beat").clicked() {
                            tab.pending.push(EditOp::DeleteBeat {
                                node: selected.clone(),
                                index,
                            });
                        }

                        ui.separator();
                        let mut edited = beat.clone();
                        if beat_form(ui, &mut edited, index) && edited != *beat {
                            tab.pending.push(EditOp::UpdateBeat {
                                node: selected.clone(),
                                index,
                                beat: edited,
                            });
                        }
                    });
                });
            }

            ui.horizontal(|ui| {
                if ui.button("+ Say").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::Say {
                            speaker: None,
                            text: String::new(),
                        },
                    });
                }
                if ui.button("+ Show actor").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::ShowActor {
                            actor: ActorId::from("actor"),
                            pose: "idle".into(),
                            slot: Slot::Center,
                        },
                    });
                }
                if ui.button("+ Hide actor").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::HideActor {
                            actor: ActorId::from("actor"),
                        },
                    });
                }
                if ui.button("+ Background").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::SetBackground { image: None },
                    });
                }
                if ui.button("+ Music").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::Music {
                            track: None,
                            fade_ms: Some(800),
                        },
                    });
                }
                if ui.button("+ Sound").clicked() {
                    tab.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::Sound {
                            clip: String::new(),
                            volume: None,
                        },
                    });
                }
            });

            ui.separator();

            // --- exit ---
            ui.label("Exit");
            ui.group(|ui| {
                let mut exit = node.exit.clone();
                let mut changed = false;
                // A reference the author cannot follow is a dead end in the editing
                // session even when the story is fine. `jump` collects the request;
                // acting on it here would fight the read borrow of `node`.
                let mut jump = None;
                exit_form(ui, &mut exit, &mut changed, &tab.editor, &mut jump);
                if changed && exit != node.exit {
                    tab.pending.push(EditOp::SetExit {
                        node: selected.clone(),
                        exit,
                    });
                }
                if let Some(target) = jump {
                    select = Some(target);
                }
            });

            // --- choices, when the exit is one ---
            if let Exit::Choose { choices, .. } = &node.exit {
                ui.separator();
                ui.label("Choices");
                for (index, choice) in choices.iter().enumerate() {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}", index + 1));
                            ui.separator();

                            // Order is what the player reads, so it is part of the writing.
                            let total = choices.len();
                            if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                                tab.pending.push(EditOp::MoveChoice {
                                    node: selected.clone(),
                                    from: index,
                                    to: index - 1,
                                });
                            }
                            if ui
                                .add_enabled(index + 1 < total, egui::Button::new("Dn"))
                                .clicked()
                            {
                                tab.pending.push(EditOp::MoveChoice {
                                    node: selected.clone(),
                                    from: index,
                                    to: index + 2,
                                });
                            }

                            let mut label = choice.label.clone();
                            if ui
                                .add(egui::TextEdit::singleline(&mut label).desired_width(200.0))
                                .lost_focus()
                                && label != choice.label
                            {
                                tab.pending.push(EditOp::UpdateChoice {
                                    node: selected.clone(),
                                    index,
                                    label: Some(label),
                                    to: None,
                                });
                            }

                            ui.label("to");

                            let mut target = choice.to.to_string();
                            let ids: Vec<String> = tab
                                .editor
                                .story()
                                .nodes
                                .keys()
                                .map(|id| id.to_string())
                                .collect();
                            egui::ComboBox::from_id_salt(("to", index))
                                .selected_text(&target)
                                .show_ui(ui, |ui| {
                                    for id in &ids {
                                        if ui
                                            .selectable_value(&mut target, id.clone(), id)
                                            .clicked()
                                            && target != choice.to.to_string()
                                        {
                                            tab.pending.push(EditOp::UpdateChoice {
                                                node: selected.clone(),
                                                index,
                                                label: None,
                                                to: Some(NodeId::from(target.clone())),
                                            });
                                        }
                                    }
                                });

                            // Following a reference by hand — find the node, scroll to
                            // it, hope the name was remembered correctly — is the most
                            // common way a writer loses the thread while editing.
                            if ui
                                .button("Go")
                                .on_hover_text("Select the target node")
                                .clicked()
                            {
                                select = Some(choice.to.clone());
                            }

                            if ui
                                .add_enabled(choices.len() > 1, egui::Button::new("Del"))
                                .on_hover_text("Delete choice")
                                .clicked()
                            {
                                tab.pending.push(EditOp::DeleteChoice {
                                    node: selected.clone(),
                                    index,
                                });
                            }
                        });
                    });
                }

                if ui.button("+ Choice").clicked() {
                    let target = tab.editor.story().start.clone();
                    tab.pending.push(EditOp::AddChoice {
                        node: selected.clone(),
                        label: "new choice".into(),
                        to: target,
                    });
                }
            }

            // --- outcomes, when the exit hands over to a gameplay system ---
            //
            // The keys are free text: the editor does not know which outcomes a system
            // reports, and a whitelist here would mean the editor knew about specific
            // systems. The runtime refuses an unmapped key with a clear error, so a typo
            // surfaces immediately rather than silently skipping a branch.
            if let Exit::Gameplay { spec } = &node.exit {
                ui.separator();
                ui.label("Config");
                ui.label(
                    egui::RichText::new(format!(
                        "RON handed to `{}` verbatim; the editor only checks that it parses",
                        spec.system
                    ))
                    .small()
                    .weak(),
                );

                // Refilled from the document whenever the selection or the document changes,
                // so an undo cannot leave the box describing a config that no longer exists.
                let key = (selected.clone(), tab.editor.story().checksum());
                if tab.config_key.as_ref() != Some(&key) {
                    tab.config_key = Some(key);
                    tab.config_buffer = config_text(spec);
                }

                ui.add(
                    egui::TextEdit::multiline(&mut tab.config_buffer)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .code_editor()
                        .hint_text("(duration_ms: 3000)"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button("Apply config")
                        .on_hover_text("Parse the text and store it on this node")
                        .clicked()
                    {
                        tab.pending.push(EditOp::SetGameplayConfig {
                            node: selected.clone(),
                            config: Some(tab.config_buffer.clone()),
                        });
                    }
                    if spec.config.is_some() && ui.button("Clear config").clicked() {
                        tab.pending.push(EditOp::SetGameplayConfig {
                            node: selected.clone(),
                            config: None,
                        });
                    }
                });

                ui.separator();
                ui.label("Outcomes");
                ui.label(
                    egui::RichText::new(format!(
                        "keys reported by `{}` and where each one leads",
                        spec.system
                    ))
                    .small()
                    .weak(),
                );

                let rows = crate::outcomes::rows(tab.editor.story(), &selected);
                let targets: Vec<String> = tab
                    .editor
                    .story()
                    .nodes
                    .keys()
                    .map(|id| id.to_string())
                    .collect();
                let mut retarget: Option<(String, NodeId)> = None;
                let mut outcome_error: Option<String> = None;
                let mut unmapped: Option<String> = None;

                for row in &rows {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(&row.outcome).monospace())
                                    .truncate(),
                            );
                            ui.label("to");

                            let mut target = row.to.to_string();
                            let original = target.clone();
                            egui::ComboBox::from_id_salt(("outcome", &row.outcome))
                                .selected_text(&target)
                                .show_ui(ui, |ui| {
                                    for id in &targets {
                                        ui.selectable_value(&mut target, id.clone(), id);
                                    }
                                });
                            if target != original {
                                retarget = Some((row.outcome.clone(), NodeId::from(target)));
                            }

                            if ui
                                .button("Go")
                                .on_hover_text("Select the target node")
                                .clicked()
                            {
                                select = Some(row.to.clone());
                            }

                            if ui
                                .button("Del")
                                .on_hover_text("Remove this mapping")
                                .clicked()
                            {
                                unmapped = Some(row.outcome.clone());
                            }
                        });
                    });
                }

                // Deferred: the loop above holds `editor`, and `set_status` borrows the
                // whole app rather than one field, so it cannot run here. Same discipline
                // as edits: collect during layout, act after.
                if let Some((outcome, to)) = retarget {
                    match crate::outcomes::set(&selected, &outcome, &to) {
                        Ok(op) => tab.pending.push(op),
                        Err(err) => outcome_error = Some(err),
                    }
                }
                if let Some(outcome) = unmapped {
                    tab.pending
                        .push(crate::outcomes::remove(&selected, &outcome));
                }

                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut tab.new_outcome)
                            .hint_text("outcome key")
                            .desired_width(140.0),
                    );
                    ui.label("to");

                    // A new mapping defaults to the story's start node: a valid
                    // placeholder the author will change, rather than an empty combo
                    // that silently does nothing when clicked.
                    if ui.button("+").clicked() {
                        let target = tab.editor.story().start.clone();
                        match crate::outcomes::set(&selected, &tab.new_outcome, &target) {
                            Ok(op) => {
                                tab.pending.push(op);
                                tab.new_outcome.clear();
                            }
                            Err(err) => outcome_error = Some(err),
                        }
                    }
                });

                // Field-level writes inside the closure, then reported once outside it:
                // `set_status` borrows the whole app, and this closure holds the tab.
                if let Some(message) = outcome_error {
                    status_after = Some((message, true));
                }
            }
        });

        if let Some(id) = select {
            Tab::plan_selection(&mut tab.selected, &mut tab.preview_steps, id);
        }
        if let Some((message, is_error)) = status_after {
            app.set_status(message, is_error);
        }
    });
}

/// Right panel: the live preview and any problems.
/// Right panel: the live preview and any problems.
pub fn inspector(app: &mut EditorApp, ui: &mut egui::Ui) {
    egui::Panel::right("preview")
        .resizable(true)
        .default_size(560.0)
        .show(ui, |ui| {
            ui.heading("Preview");
            if let Some(reason) = &app.preview.unavailable {
                ui.colored_label(egui::Color32::from_rgb(240, 180, 100), reason);
                return;
            }

            let active = app.active;

            // The frame is rendered while borrowing the tab immutably and the renderer
            // mutably. Those are two different fields of the app, which is the only reason
            // this is possible without cloning the story every frame.
            let rendered = {
                let Some(tab) = app.tabs.get(active) else {
                    return;
                };
                let steps = tab.preview_steps.clone();
                let assets = tab.assets.clone();
                let problems = tab.editor.problems();
                // Preview what the author is looking at, not the beginning of the story. A
                // preview of node A while node B is selected teaches people to distrust the
                // panel.
                let start = tab
                    .selected_node()
                    .unwrap_or_else(|| tab.story().start.clone());

                let outcome = app
                    .preview
                    .render(ui.ctx(), tab.story(), &start, &steps, &assets);
                (outcome, problems)
            };
            let (outcome, problems) = rendered;

            let (report, failed) = match outcome {
                Ok(report) => (report, None),
                Err(err) => (
                    PreviewReport {
                        status: err.clone(),
                        choices: Vec::new(),
                        finished: false,
                    },
                    Some(err),
                ),
            };

            let mut advance: Option<Step> = None;

            if let Some(handle) = app.preview.texture() {
                let available = ui.available_width();
                let height =
                    available * (crate::preview::HEIGHT as f32 / crate::preview::WIDTH as f32);
                // Clicking the frame advances the story: the preview is a playthrough, not
                // a still, and the alternative is finding the >> button every time.
                let response = ui.add(
                    egui::Image::new(handle)
                        .fit_to_exact_size(egui::vec2(available, height))
                        .sense(egui::Sense::click()),
                );
                if response.clicked() {
                    advance = Some(Step::Click);
                }
            }

            ui.separator();
            ui.label(format!("at {}", report.status));
            if !app
                .tabs
                .get(active)
                .is_some_and(|tab| tab.preview_steps.is_empty())
                && ui.button("Reset preview").clicked()
            {
                advance = None;
                if let Some(tab) = app.tabs.get_mut(active) {
                    tab.preview_steps.clear();
                }
            }

            // The branches on offer, as buttons pressed in the playthrough. Without these
            // a choice node shows a menu nobody can click.
            if !report.choices.is_empty() {
                ui.separator();
                for (index, label) in report.choices.iter().enumerate() {
                    if ui.button(format!("{}. {label}", index + 1)).clicked() {
                        advance = Some(Step::Choose(index));
                    }
                }
            } else if report.finished {
                ui.label(egui::RichText::new("The story has ended.").weak());
            }

            if let Some(step) = advance
                && let Some(tab) = app.tabs.get_mut(active)
            {
                tab.preview_steps.push(step);
            }

            if !problems.is_empty() {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(240, 180, 100),
                    format!("{} problem(s)", problems.len()),
                );
                for problem in problems {
                    ui.label(format!("• {problem}"));
                }
            }

            // Written after the panel's own borrows end: `set_status` takes the whole app.
            if let Some(tab) = app.tabs.get_mut(active) {
                tab.preview_status = report.status.clone();
            }
            if let Some(err) = failed {
                app.set_status(format!("preview: {err}"), true);
            }
        });
}

/// Edit a beat in place. Returns whether anything changed.
fn beat_form(ui: &mut egui::Ui, beat: &mut Beat, index: usize) -> bool {
    let before = beat.clone();

    match beat {
        Beat::Say { speaker, text } => {
            ui.horizontal(|ui| {
                let mut is_narration = speaker.is_none();
                if ui.checkbox(&mut is_narration, "narration").changed() {
                    *speaker = if is_narration {
                        None
                    } else {
                        Some(ActorId::from("actor"))
                    };
                }
                if let Some(who) = speaker {
                    let mut name = who.to_string();
                    if ui
                        .add(egui::TextEdit::singleline(&mut name).desired_width(100.0))
                        .changed()
                    {
                        *who = ActorId::from(name);
                    }
                }
            });
            ui.add(
                egui::TextEdit::multiline(text)
                    .desired_width(f32::INFINITY)
                    .desired_rows(2)
                    .hint_text("line of dialogue"),
            );
        }
        Beat::ShowActor { actor, pose, slot } => {
            ui.horizontal(|ui| {
                ui.label("show");
                let mut name = actor.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(90.0))
                    .changed()
                {
                    *actor = ActorId::from(name);
                }
                ui.label("as");
                ui.add(egui::TextEdit::singleline(pose).desired_width(120.0));
                ui.label("at");
                egui::ComboBox::from_id_salt(format!("slot-{index}"))
                    .selected_text(format!("{slot:?}"))
                    .show_ui(ui, |ui| {
                        for (label, value) in [
                            ("Left", Slot::Left),
                            ("Center", Slot::Center),
                            ("Right", Slot::Right),
                        ] {
                            ui.selectable_value(slot, value, label);
                        }
                    });
            });
        }
        Beat::HideActor { actor } => {
            ui.horizontal(|ui| {
                ui.label("hide");
                let mut name = actor.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(120.0))
                    .changed()
                {
                    *actor = ActorId::from(name);
                }
            });
        }
        Beat::SetBackground { image } => {
            ui.horizontal(|ui| {
                ui.label("background");
                let mut is_set = image.is_some();
                if ui.checkbox(&mut is_set, "set").changed() {
                    *image = if is_set { Some(String::new()) } else { None };
                }
                if let Some(path) = image {
                    ui.add(
                        egui::TextEdit::singleline(path)
                            .desired_width(180.0)
                            .hint_text("bg.png"),
                    );
                }
            });
        }
        Beat::Music { track, fade_ms } => {
            ui.horizontal(|ui| {
                ui.label("music");
                let mut playing = track.is_some();
                if ui.checkbox(&mut playing, "play").changed() {
                    *track = if playing { Some(String::new()) } else { None };
                }
                if let Some(name) = track {
                    ui.add(
                        egui::TextEdit::singleline(name)
                            .desired_width(160.0)
                            .hint_text("track"),
                    );
                }
                ui.label("fade");
                // Milliseconds rather than seconds: the value goes into the story file,
                // and a unit that survives a round trip without float formatting is one
                // fewer way for the file to churn.
                let mut fade = fade_ms.unwrap_or(0);
                if ui
                    .add(egui::DragValue::new(&mut fade).speed(10).range(0..=10_000))
                    .changed()
                {
                    *fade_ms = Some(fade);
                }
                ui.label("ms");
            });
        }
        Beat::Sound { clip, volume } => {
            ui.horizontal(|ui| {
                ui.label("sound");
                ui.add(
                    egui::TextEdit::singleline(clip)
                        .desired_width(180.0)
                        .hint_text("clip"),
                );
                ui.label("volume");
                let mut level = volume.unwrap_or(1.0);
                if ui
                    .add(egui::Slider::new(&mut level, 0.0..=1.0).fixed_decimals(2))
                    .changed()
                {
                    // `None` means "the default", which is 1.0. Writing 1.0 back as an
                    // explicit value would make every sound beat longer for no reason.
                    *volume = if (level - 1.0).abs() < f32::EPSILON {
                        None
                    } else {
                        Some(level)
                    };
                }
            });
        }
    }

    *beat != before
}

/// Edit an exit. Sets `changed` when the user asked for something different.
///
/// `jump` receives a node the author asked to look at. The form cannot select it
/// itself: it is handed a story, not the app, and reaching back into the app from here
/// would make the panel that draws a form also decide what is selected.
fn exit_form(
    ui: &mut egui::Ui,
    exit: &mut Exit,
    changed: &mut bool,
    editor: &Editor,
    jump: &mut Option<NodeId>,
) {
    let node_ids: Vec<String> = editor
        .story()
        .nodes
        .keys()
        .map(|id| id.to_string())
        .collect();

    ui.horizontal(|ui| {
        let current = match exit {
            Exit::End { .. } => "End",
            Exit::Goto { .. } => "Goto",
            Exit::Choose { .. } => "Choose",
            Exit::Gameplay { .. } => "Gameplay",
        };
        egui::ComboBox::from_id_salt("exit kind")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for kind in ["End", "Goto", "Choose", "Gameplay"] {
                    if ui.selectable_label(current == kind, kind).clicked() && current != kind {
                        *exit = match kind {
                            "End" => Exit::End { ending: None },
                            "Goto" => Exit::Goto {
                                to: NodeId::from(node_ids.first().cloned().unwrap_or_default()),
                            },
                            "Choose" => Exit::Choose {
                                prompt: None,
                                choices: vec![rustory_vn_core::Choice {
                                    label: "option".into(),
                                    to: NodeId::from(node_ids.first().cloned().unwrap_or_default()),
                                }],
                            },
                            _ => Exit::Gameplay {
                                spec: rustory_vn_core::GameplaySpec {
                                    system: rustory_vn_core::SystemId::from("timer.timed_choice"),
                                    config: None,
                                    outcomes: std::collections::BTreeMap::new(),
                                },
                            },
                        };
                        *changed = true;
                    }
                }
            });

        match exit {
            Exit::End { ending } => {
                let mut has = ending.is_some();
                if ui.checkbox(&mut has, "named ending").changed() {
                    *ending = if has { Some(String::new()) } else { None };
                    *changed = true;
                }
                if let Some(name) = ending
                    && ui
                        .add(egui::TextEdit::singleline(name).desired_width(160.0))
                        .changed()
                {
                    *changed = true;
                }
            }
            Exit::Goto { to } => {
                let mut target = to.to_string();
                egui::ComboBox::from_id_salt("goto target")
                    .selected_text(&target)
                    .show_ui(ui, |ui| {
                        for id in &node_ids {
                            if ui.selectable_value(&mut target, id.clone(), id).clicked()
                                && target != to.to_string()
                            {
                                *to = NodeId::from(target.clone());
                                *changed = true;
                            }
                        }
                    });
                if ui
                    .button("Go")
                    .on_hover_text("Select the target node")
                    .clicked()
                {
                    *jump = Some(to.clone());
                }
            }
            Exit::Choose { prompt, .. } => {
                let mut has = prompt.is_some();
                if ui.checkbox(&mut has, "prompt").changed() {
                    *prompt = if has { Some(String::new()) } else { None };
                    *changed = true;
                }
                if let Some(text) = prompt
                    && ui
                        .add(egui::TextEdit::singleline(text).desired_width(240.0))
                        .changed()
                {
                    *changed = true;
                }
            }
            Exit::Gameplay { spec } => {
                let mut system = spec.system.to_string();
                if ui
                    .add(egui::TextEdit::singleline(&mut system).desired_width(180.0))
                    .changed()
                {
                    spec.system = rustory_vn_core::SystemId::from(system);
                    *changed = true;
                }
                // The mappings themselves are drawn below the exit form, where they get
                // room for a table. Saying so beats an empty row here.
                ui.label(format!("{} outcome(s), mapped below", spec.outcomes.len()));
            }
        }
    });
}

/// The shortcut sheet.
///
/// A window rather than a panel: the list is short, and covering the story while it is open
/// would mean closing it to look something up. F1 toggles it, and so does the `?` button.
pub fn help_window(app: &mut EditorApp, ui: &mut egui::Ui) {
    if !app.show_help {
        return;
    }

    let mut open = true;
    egui::Window::new("Keys and controls")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        // Centred by default: opening on top of the node list hides the thing the reader was
        // about to look at, and a help window that has to be dragged before it is useful is
        // a help window people stop opening.
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ui.ctx(), |ui| {
            let mut row = |keys: &str, what: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        [120.0, 18.0],
                        egui::Label::new(egui::RichText::new(keys).monospace()),
                    );
                    ui.label(what);
                });
            };

            row("Ctrl+N", "New story in a new tab");
            row("Ctrl+O", "Open a story in a new tab");
            row("Ctrl+S", "Save");
            row("Ctrl+Shift+S", "Save as");
            row("Ctrl+W", "Close the tab");
            row("Ctrl+Z", "Undo");
            row("Ctrl+Y", "Redo");
            row("F1", "This list");

            ui.separator();
            ui.label(egui::RichText::new("Tabs").strong());
            ui.label("Several stories can be open at once; click a tab to switch.");
            ui.label("A dot after the name means unsaved changes.");
            ui.label("Closing a tab with unsaved changes asks first, and so does closing");
            ui.label("the window — every dirty tab is asked about, not just the visible one.");

            ui.separator();
            ui.label(egui::RichText::new("Preview").strong());
            ui.label("Click the picture to advance.");
            ui.label("A branch appears as buttons under it.");
            ui.label("The slider rewinds along the path already taken.");

            ui.separator();
            ui.label(egui::RichText::new("Editing").strong());
            ui.label("Each beat row has + (insert a copy after it), Up, Dn and Del.");
            ui.label("Text fields are applied as you type; every edit is one undo step.");
            ui.label("Saving rewrites the file from its data, so comments are not kept.");

            ui.separator();
            ui.label(egui::RichText::new("Recovery").strong());
            ui.label("Unsaved work is also written beside the story, as .draft.ron.");
            ui.label("The next open offers it back; nothing is restored silently.");
        });

    // The window's own close button has to be honoured as well as the toggle.
    if !open {
        app.show_help = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_filter_matches_every_node() {
        assert!(matches_filter(&NodeId::from("prologue"), ""));
        assert!(matches_filter(&NodeId::from("prologue"), "   "));
    }

    #[test]
    fn the_filter_matches_anywhere_in_the_name_ignoring_case() {
        // Searching `honest` has to find `half_honest`: authors look for the word, not
        // for the prefix someone else chose.
        assert!(matches_filter(&NodeId::from("half_honest"), "HONEST"));
        assert!(matches_filter(&NodeId::from("Prologue"), "pro"));
    }

    #[test]
    fn a_filter_that_appears_nowhere_excludes_the_node() {
        assert!(!matches_filter(&NodeId::from("prologue"), "epilogue"));
    }
}
