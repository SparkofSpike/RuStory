//! The panels.
//!
//! Each function lays out one region and pushes [`EditOp`]s into `app.pending` rather
//! than touching the story. That is the whole contract: panels describe intent, the
//! app applies it.
//!
//! Forms edit a *copy* of the value being changed, held in a local, and emit an
//! operation only when it differs. This keeps typing responsive — applying an
//! operation on every keystroke would push one undo step per character.

use rustory_core::{Beat, Exit, NodeId, Slot};
use rustory_edit::prelude::*;

use crate::app::{EditorApp, UiAction};

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

            let has_doc = app.editor.is_some();

            if ui
                .button("New")
                .on_hover_text("New story (Ctrl+N)")
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
                    app.pending_actions.push(UiAction::SaveAs);
                }
            });

            ui.separator();

            let (can_undo, can_redo, undo_label, redo_label) = app
                .editor
                .as_ref()
                .map(|editor| {
                    (
                        editor.can_undo(),
                        editor.can_redo(),
                        editor.undo_label().unwrap_or("").to_owned(),
                        editor.redo_label().unwrap_or("").to_owned(),
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

            ui.separator();

            if let Some(editor) = &app.editor {
                let name = if editor.document().has_file() {
                    editor.path().file_name().map_or_else(
                        || "untitled".to_owned(),
                        |name| name.to_string_lossy().to_string(),
                    )
                } else {
                    // A story with no file yet must not look like one that has a home.
                    "untitled".to_owned()
                };
                let dirty = if editor.is_dirty() { " •" } else { "" };
                ui.label(format!("{name}{dirty}"));
            }

            // Preview position, right-aligned so it does not shift with the file name.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let total = app
                    .editor
                    .as_ref()
                    .map_or(0, |editor| editor.story().beat_count());
                let max = total.max(1);

                if ui.button(">>").on_hover_text("Step forward").clicked() {
                    app.preview_step += 1;
                }
                ui.add(
                    egui::Slider::new(&mut app.preview_step, 0..=max)
                        .text("step")
                        .clamping(egui::SliderClamping::Always),
                );
                if ui.button("<<").on_hover_text("Reset preview").clicked() {
                    app.preview_step = 0;
                }
                ui.label("Preview:");
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
                    .editor
                    .as_ref()
                    .map(|editor| editor.problems().len())
                    .unwrap_or(0);
                if problems > 0 {
                    ui.colored_label(
                        egui::Color32::from_rgb(240, 180, 100),
                        format!("{problems} problem(s)"),
                    );
                } else if app.editor.is_some() {
                    ui.colored_label(egui::Color32::from_rgb(140, 200, 140), "valid");
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

            let Some(editor) = app.editor.as_ref() else {
                ui.label("No story open.");
                return;
            };

            // A story with thirty nodes is already a list you scan rather than read, and
            // scanning is what a filter is for. Typing here does not edit the document,
            // so it takes no undo step and cannot fail.
            ui.add(
                egui::TextEdit::singleline(&mut app.node_filter)
                    .hint_text("filter nodes")
                    .desired_width(f32::INFINITY),
            );

            let story = editor.story();
            let start = story.start.clone();
            let total = story.nodes.len();
            let ids: Vec<NodeId> = story
                .nodes
                .keys()
                .filter(|id| matches_filter(id, &app.node_filter))
                .cloned()
                .collect();

            if ids.len() != total {
                ui.label(
                    egui::RichText::new(format!("{} of {total}", ids.len()))
                        .small()
                        .weak(),
                );
            }
            if ids.is_empty() && !app.node_filter.trim().is_empty() {
                ui.label(egui::RichText::new("No node matches.").weak());
            }

            // Collected rather than applied inside the scroll area: `select` is a method
            // on the app, which borrows all of it, and the closure still holds `editor`.
            let mut select: Option<NodeId> = None;
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 130.0)
                .show(ui, |ui| {
                    for id in &ids {
                        let node = &story.nodes[id];
                        let is_start = id == &start;
                        let selected = app.selected.as_ref() == Some(id);

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
                            app.rename_buffer = id.to_string();
                        }
                    }
                });

            ui.separator();

            // Create
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut app.new_node_id)
                        .hint_text("new node id")
                        .desired_width(140.0),
                );
                let can_create = !app.new_node_id.trim().is_empty();
                if ui.add_enabled(can_create, egui::Button::new("+")).clicked() {
                    let id = NodeId::from(app.new_node_id.trim());
                    app.pending.push(EditOp::CreateNode { id, at: None });
                    app.new_node_id.clear();
                }
            });

            // Rename / duplicate / delete the selection
            if let Some(selected) = app.selected.clone() {
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!is_start(&selected, editor), egui::Button::new("Delete"))
                        .on_hover_text("Delete the selected node")
                        .clicked()
                    {
                        app.pending.push(EditOp::DeleteNode {
                            id: selected.clone(),
                        });
                    }

                    if ui.button("Duplicate").clicked() {
                        let as_id = NodeId::from(format!("{}-copy", selected));
                        app.pending.push(EditOp::DuplicateNode {
                            id: selected.clone(),
                            as_id,
                        });
                    }

                    if ui.button("Make start").clicked() {
                        app.pending.push(EditOp::SetStartNode {
                            node: selected.clone(),
                        });
                    }
                });

                ui.horizontal(|ui| {
                    if app.rename_buffer.is_empty() {
                        app.rename_buffer = selected.to_string();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut app.rename_buffer)
                            .hint_text("rename to")
                            .desired_width(140.0),
                    );
                    let target = app.rename_buffer.trim().to_owned();
                    let can_rename = !target.is_empty() && target != selected.to_string();
                    if ui
                        .add_enabled(can_rename, egui::Button::new("Rename"))
                        .clicked()
                    {
                        app.pending.push(EditOp::RenameNode {
                            from: selected.clone(),
                            to: NodeId::from(target),
                        });
                    }
                });
            }

            // Applied last: `select` borrows the whole app, and `editor` is still in use
            // above. The early return that used to sit here is gone for the same reason
            // — skipping it must not skip the click the user just made.
            if let Some(id) = select {
                app.select(id);
            }
        });
}

fn is_start(id: &NodeId, editor: &Editor) -> bool {
    &editor.story().start == id
}

fn describe_node(node: &rustory_core::Node) -> String {
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
        let Some(editor) = app.editor.as_ref() else {
            ui.heading("RuStory Editor");
            ui.label("Open a story to start editing.");
            ui.label("Usage: rustory-editor <story.ron>");
            return;
        };

        let Some(selected) = app.selected.clone() else {
            ui.label("Select a node.");
            return;
        };

        let Some(node) = editor.story().nodes.get(&selected) else {
            ui.label("That node no longer exists.");
            return;
        };

        ui.heading(format!("{selected}"));
        ui.separator();

        // Collected rather than applied inside the scroll area: `select` is a method on
        // the app, which borrows all of it, and the closure still holds `editor`.
        let mut select: Option<NodeId> = None;

        egui::ScrollArea::vertical().show(ui, |ui| {
            // --- story settings ---
            //
            // `SetTitle` and `SetAuthor` existed as operations before they had a control
            // here, which meant the CLI could do something the GUI could not — the exact
            // drift the operation layer is supposed to make impossible. A periodic audit
            // that greps for each `EditOp` variant in the editor source is what caught it;
            // doing that by eye would not have.
            story_settings(
                &mut app.pending,
                &mut app.show_story_settings,
                editor.story(),
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

                        // Move up / down / delete
                        if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                            app.pending.push(EditOp::MoveBeat {
                                node: selected.clone(),
                                from: index,
                                to: index - 1,
                            });
                        }
                        if ui
                            .add_enabled(index + 1 < beat_count, egui::Button::new("Dn"))
                            .clicked()
                        {
                            app.pending.push(EditOp::MoveBeat {
                                node: selected.clone(),
                                from: index,
                                to: index + 2,
                            });
                        }
                        if ui.button("Del").on_hover_text("Delete beat").clicked() {
                            app.pending.push(EditOp::DeleteBeat {
                                node: selected.clone(),
                                index,
                            });
                        }

                        ui.separator();
                        let mut edited = beat.clone();
                        if beat_form(ui, &mut edited, index) && edited != *beat {
                            app.pending.push(EditOp::UpdateBeat {
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
                    app.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::Say {
                            speaker: None,
                            text: String::new(),
                        },
                    });
                }
                if ui.button("+ Show actor").clicked() {
                    app.pending.push(EditOp::InsertBeat {
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
                    app.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::HideActor {
                            actor: ActorId::from("actor"),
                        },
                    });
                }
                if ui.button("+ Background").clicked() {
                    app.pending.push(EditOp::InsertBeat {
                        node: selected.clone(),
                        index: beat_count,
                        beat: Beat::SetBackground { image: None },
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
                exit_form(ui, &mut exit, &mut changed, editor, &mut jump);
                if changed && exit != node.exit {
                    app.pending.push(EditOp::SetExit {
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

                            let mut label = choice.label.clone();
                            if ui
                                .add(egui::TextEdit::singleline(&mut label).desired_width(200.0))
                                .lost_focus()
                                && label != choice.label
                            {
                                app.pending.push(EditOp::UpdateChoice {
                                    node: selected.clone(),
                                    index,
                                    label: Some(label),
                                    to: None,
                                });
                            }

                            ui.label("to");

                            let mut target = choice.to.to_string();
                            let ids: Vec<String> = editor
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
                                            app.pending.push(EditOp::UpdateChoice {
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
                                app.pending.push(EditOp::DeleteChoice {
                                    node: selected.clone(),
                                    index,
                                });
                            }
                        });
                    });
                }

                if ui.button("+ Choice").clicked() {
                    let target = editor.story().start.clone();
                    app.pending.push(EditOp::AddChoice {
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
                ui.label("Outcomes");
                ui.label(
                    egui::RichText::new(format!(
                        "keys reported by `{}` and where each one leads",
                        spec.system
                    ))
                    .small()
                    .weak(),
                );

                let rows = crate::outcomes::rows(editor.story(), &selected);
                let targets: Vec<String> = editor
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
                        Ok(op) => app.pending.push(op),
                        Err(err) => outcome_error = Some(err),
                    }
                }
                if let Some(outcome) = unmapped {
                    app.pending
                        .push(crate::outcomes::remove(&selected, &outcome));
                }

                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut app.new_outcome)
                            .hint_text("outcome key")
                            .desired_width(140.0),
                    );
                    ui.label("to");

                    // A new mapping defaults to the story's start node: a valid
                    // placeholder the author will change, rather than an empty combo
                    // that silently does nothing when clicked.
                    if ui.button("+").clicked() {
                        let target = editor.story().start.clone();
                        match crate::outcomes::set(&selected, &app.new_outcome, &target) {
                            Ok(op) => {
                                app.pending.push(op);
                                app.new_outcome.clear();
                            }
                            Err(err) => outcome_error = Some(err),
                        }
                    }
                });

                // Field-level writes, not `set_status`: that method borrows all of
                // `app`, and `editor` still holds a borrow of `app.editor` here.
                if let Some(message) = outcome_error {
                    app.status = message;
                    app.status_is_error = true;
                }
            }
        });

        if let Some(id) = select {
            app.select(id);
        }
    });
}

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

            let Some(editor) = app.editor.as_ref() else {
                return;
            };

            // Render into a texture, read it back, and hand it to egui. This is the
            // engine, not a mock-up — see preview.rs for why the image goes through the
            // CPU.
            //
            // Problems and the story are captured before any mutation: `set_status`
            // borrows all of `app`, which would conflict with holding `editor`.
            let step = app.preview_step;
            let assets = app.assets.clone();
            let problems = editor.problems();
            // Preview what the author is looking at, not the beginning of the story. A
            // preview of node A while node B is selected teaches people to distrust the
            // panel.
            let start = app
                .selected
                .clone()
                .unwrap_or_else(|| editor.story().start.clone());
            let outcome = app
                .preview
                .render(ui.ctx(), editor.story(), &start, step, &assets);
            let (status_line, failed) = match outcome {
                Ok(status) => (status, None),
                Err(err) => (err.clone(), Some(err)),
            };

            app.preview_status = status_line;
            app.status_is_error = failed.is_some();
            if let Some(err) = failed {
                app.set_status(format!("preview: {err}"), true);
            }

            if let Some(handle) = app.preview.texture() {
                let available = ui.available_width();
                let height =
                    available * (crate::preview::HEIGHT as f32 / crate::preview::WIDTH as f32);
                ui.add(egui::Image::new(handle).fit_to_exact_size(egui::vec2(available, height)));
            }

            ui.separator();
            ui.label(format!("at {}", app.preview_status));
            if app.preview_step > 0 && ui.button("Reset preview").clicked() {
                app.preview_step = 0;
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
                                choices: vec![rustory_core::Choice {
                                    label: "option".into(),
                                    to: NodeId::from(node_ids.first().cloned().unwrap_or_default()),
                                }],
                            },
                            _ => Exit::Gameplay {
                                spec: rustory_core::GameplaySpec {
                                    system: rustory_core::SystemId::from("timer.timed_choice"),
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
                    spec.system = rustory_core::SystemId::from(system);
                    *changed = true;
                }
                // The mappings themselves are drawn below the exit form, where they get
                // room for a table. Saying so beats an empty row here.
                ui.label(format!("{} outcome(s), mapped below", spec.outcomes.len()));
            }
        }
    });
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
