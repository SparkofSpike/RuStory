//! Translates narrative state into a draw list.
//!
//! This is the only place that knows both what a story is and what a pixel is.
//! Keeping the translation in one module means layout can change without the
//! state machine or the renderer noticing, and it makes layout testable: the
//! output is a [`Frame`], plain data with no GPU attached.

use std::collections::HashMap;

use rustory_vn_core::{LineView, Mode, Narrative, Slot};
use rustory_vn_render::{Frame, Quad, Rect, Rgba, TextItem, TextureId};

/// Logical resolution. All layout constants below are in these units.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

const DIALOGUE_BOX: Rect = Rect::new(60.0, 510.0, 1160.0, 180.0);
const NAME_ORIGIN: (f32, f32) = (92.0, 522.0);
const LINE_ORIGIN: (f32, f32) = (92.0, 566.0);
const LINE_SIZE: f32 = 30.0;
const NAME_SIZE: f32 = 24.0;

const CHOICE_WIDTH: f32 = 620.0;
const CHOICE_HEIGHT: f32 = 66.0;
const CHOICE_TOP: f32 = 210.0;
const CHOICE_GAP: f32 = 18.0;

const SPRITE_SCALE: f32 = 0.75;

const COL_BACKGROUND: Rgba = Rgba::rgb(12, 14, 20);
const COL_BOX: Rgba = Rgba::rgba(8, 10, 16, 236);
const COL_BOX_BORDER: Rgba = Rgba::rgba(150, 165, 200, 90);
const COL_TEXT: Rgba = Rgba::rgb(238, 240, 246);
const COL_NAME: Rgba = Rgba::rgb(178, 196, 240);
const COL_CHOICE: Rgba = Rgba::rgba(26, 31, 46, 248);
const COL_CHOICE_EDGE: Rgba = Rgba::rgba(120, 150, 210, 200);
const COL_HINT: Rgba = Rgba::rgba(200, 210, 230, 200);

/// Resolves asset names to uploaded textures.
#[derive(Default)]
pub struct TextureBank {
    textures: HashMap<String, TextureId>,
}

impl TextureBank {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, id: TextureId) {
        self.textures.insert(name.into(), id);
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<TextureId> {
        self.textures.get(name).copied()
    }

    /// Accepts either a bare name (`school`) or a filename (`school.png`).
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<TextureId> {
        self.get(name).or_else(|| {
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            self.get(stem)
        })
    }
}

/// Builds the draw list for one frame.
pub struct Scene<'a> {
    pub narrative: &'a Narrative,
    pub textures: &'a TextureBank,
    /// Prompt from the gameplay system that currently has control, if any.
    pub gameplay_prompt: Option<&'a str>,
    /// Keyboard hint line, shown at the bottom.
    pub hint: Option<&'a str>,
}

impl Scene<'_> {
    #[must_use]
    pub fn build(&self) -> Frame {
        let mut frame = Frame::new(WIDTH, HEIGHT).with_clear(COL_BACKGROUND);

        self.push_background(&mut frame);
        self.push_actors(&mut frame);
        self.push_dialogue(&mut frame);
        self.push_choice(&mut frame);
        self.push_gameplay_prompt(&mut frame);
        self.push_hint(&mut frame);

        frame
    }

    fn push_background(&self, frame: &mut Frame) {
        let Some(name) = self.narrative.stage().background() else {
            return;
        };
        let Some(id) = self.textures.resolve(name) else {
            return;
        };
        frame.push_quad(Quad::textured(
            Rect::new(0.0, 0.0, WIDTH as f32, HEIGHT as f32),
            id,
            Rgba::rgb(255, 255, 255),
        ));
    }

    fn push_actors(&self, frame: &mut Frame) {
        // The actor id is not needed here: the pose names the texture. The stage
        // keeps the ids so a later pass can still tell who is on screen.
        for state in self.narrative.stage().actors().values() {
            // Actors without art are skipped rather than drawn as a placeholder:
            // a missing sprite should look like a missing sprite, not a bug.
            let Some(id) = self.textures.resolve(&state.pose) else {
                continue;
            };

            let height = 620.0 * SPRITE_SCALE;
            let width = 420.0 * SPRITE_SCALE;
            let x = match state.slot {
                Slot::Left => 80.0,
                Slot::Center => (WIDTH as f32 - width) / 2.0,
                Slot::Right => WIDTH as f32 - 80.0 - width,
            };
            // Feet rest on the dialogue box so a sprite never floats mid-air.
            let y = DIALOGUE_BOX.y - height;

            frame.push_quad(Quad::textured(
                Rect::new(x, y, width, height),
                id,
                Rgba::rgb(255, 255, 255),
            ));
        }
    }

    fn push_dialogue(&self, frame: &mut Frame) {
        let Some(line) = self.narrative.line() else {
            return;
        };

        // The box is drawn for any visible line, including one still revealing.
        frame.push_quad(Quad::solid(DIALOGUE_BOX, COL_BOX));
        // A thin top border reads as a panel edge without needing rounded corners.
        frame.push_quad(Quad::solid(
            Rect::new(DIALOGUE_BOX.x, DIALOGUE_BOX.y, DIALOGUE_BOX.width, 2.0),
            COL_BOX_BORDER,
        ));

        if let Some(speaker) = line.speaker {
            frame.push_text(
                TextItem::new(
                    display_name(speaker.as_str()),
                    NAME_ORIGIN.0,
                    NAME_ORIGIN.1,
                    NAME_SIZE,
                )
                .with_color(COL_NAME)
                .with_wrap(DIALOGUE_BOX.width - 64.0),
            );
        }

        frame.push_text(
            TextItem::new(
                line.visible.to_owned(),
                LINE_ORIGIN.0,
                LINE_ORIGIN.1,
                LINE_SIZE,
            )
            .with_color(COL_TEXT)
            .with_wrap(DIALOGUE_BOX.width - 64.0),
        );
    }

    fn push_choice(&self, frame: &mut Frame) {
        let Mode::AwaitingChoice { prompt, choices } = self.narrative.mode() else {
            return;
        };

        if let Some(prompt) = prompt {
            // The prompt gets its own opaque plate: drawn straight onto the
            // scene it collides with sprites and becomes unreadable.
            let plate = Rect::new(330.0, 108.0, 620.0, 56.0);
            frame.push_quad(Quad::solid(plate, COL_CHOICE));
            frame.push_text(
                TextItem::new(prompt.clone(), plate.x + 24.0, plate.y + 14.0, 26.0)
                    .with_color(COL_TEXT)
                    .with_wrap(plate.width - 48.0),
            );
        }

        let x = (WIDTH as f32 - CHOICE_WIDTH) / 2.0;
        for (index, choice) in choices.iter().enumerate() {
            let y = CHOICE_TOP + index as f32 * (CHOICE_HEIGHT + CHOICE_GAP);
            let rect = Rect::new(x, y, CHOICE_WIDTH, CHOICE_HEIGHT);
            frame.push_quad(Quad::solid(rect, COL_CHOICE));
            frame.push_quad(Quad::solid(
                Rect::new(rect.x, rect.y, 4.0, rect.height),
                COL_CHOICE_EDGE,
            ));
            frame.push_text(
                TextItem::new(
                    format!("{}. {}", index + 1, choice.label),
                    rect.x + 24.0,
                    rect.y + 18.0,
                    26.0,
                )
                .with_color(COL_TEXT)
                .with_wrap(rect.width - 48.0),
            );
        }
    }

    fn push_gameplay_prompt(&self, frame: &mut Frame) {
        let Some(prompt) = self.gameplay_prompt else {
            return;
        };
        // While a gameplay system has control there is no dialogue on screen, so
        // the prompt takes the space the dialogue box would occupy. Putting it at
        // the top instead would cover the sprites' heads, which is where the
        // player is looking.
        let plate = Rect::new(
            DIALOGUE_BOX.x,
            DIALOGUE_BOX.y,
            DIALOGUE_BOX.width,
            DIALOGUE_BOX.height,
        );
        frame.push_quad(Quad::solid(plate, COL_CHOICE));
        frame.push_quad(Quad::solid(
            Rect::new(plate.x, plate.y, plate.width, 2.0),
            COL_CHOICE_EDGE,
        ));
        frame.push_text(
            TextItem::new(prompt, plate.x + 32.0, plate.y + 28.0, 28.0)
                .with_color(COL_TEXT)
                .with_wrap(plate.width - 64.0),
        );
    }

    fn push_hint(&self, frame: &mut Frame) {
        let Some(hint) = self.hint else {
            return;
        };
        frame.push_text(TextItem::new(hint, 92.0, HEIGHT as f32 - 34.0, 18.0).with_color(COL_HINT));
    }
}

/// `yuki` becomes `Yuki`. Scripts keep ids lowercase; screens capitalise them.
#[must_use]
pub fn display_name(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A one-line summary of the current line, used by the CLI transcript.
#[must_use]
pub fn line_summary(line: &LineView<'_>) -> String {
    match line.speaker {
        Some(speaker) => format!("{}: {}", display_name(speaker.as_str()), line.full_text),
        None => line.full_text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustory_vn_core::{ActorId, Beat, Choice, Exit, Node, NodeId, Story, StoryMeta};
    use std::collections::BTreeMap;

    fn bank_with(names: &[&str]) -> TextureBank {
        let mut bank = TextureBank::new();
        for (index, name) in names.iter().enumerate() {
            bank.insert(*name, TextureId((index + 1) as u32));
        }
        bank
    }

    fn talking_story(beats: Vec<Beat>, exit: Exit) -> Narrative {
        let mut nodes = BTreeMap::new();
        nodes.insert(NodeId::from("a"), Node { beats, exit });
        Narrative::new(Story {
            meta: StoryMeta::default(),
            start: NodeId::from("a"),
            nodes,
        })
        .expect("story is valid")
    }

    #[test]
    fn an_empty_stage_still_draws_the_dialogue_box_once_a_line_is_active() {
        let narrative = talking_story(
            vec![Beat::Say {
                speaker: Some(ActorId::from("yuki")),
                text: "Hello there.".into(),
            }],
            Exit::End { ending: None },
        );
        let bank = TextureBank::new();

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: None,
            hint: None,
        }
        .build();

        // Box, top border, name and line.
        assert_eq!(frame.quads.len(), 2, "box plus border");
        assert_eq!(frame.texts.len(), 2, "speaker name plus line");
        assert_eq!(frame.texts[0].text, "Yuki");
        assert_eq!(frame.texts[1].text, "");
    }

    #[test]
    fn the_background_fills_the_frame_when_the_asset_resolves() {
        let narrative = talking_story(
            vec![
                Beat::SetBackground {
                    image: Some("bg_classroom.png".into()),
                },
                Beat::Say {
                    speaker: None,
                    text: ".".into(),
                },
            ],
            Exit::End { ending: None },
        );
        let bank = bank_with(&["bg_classroom"]);

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: None,
            hint: None,
        }
        .build();

        let background = &frame.quads[0];
        assert_eq!(background.rect.width, WIDTH as f32);
        assert_eq!(background.rect.height, HEIGHT as f32);
        assert!(background.texture.is_some());
    }

    #[test]
    fn a_missing_background_asset_draws_nothing_rather_than_a_black_slab() {
        let narrative = talking_story(
            vec![
                Beat::SetBackground {
                    image: Some("not_here.png".into()),
                },
                Beat::Say {
                    speaker: None,
                    text: ".".into(),
                },
            ],
            Exit::End { ending: None },
        );
        let bank = TextureBank::new();

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: None,
            hint: None,
        }
        .build();

        assert!(
            frame.quads.iter().all(|q| q.texture.is_none()),
            "nothing should be textured when the asset is absent"
        );
    }

    #[test]
    fn actors_are_placed_by_slot_and_rest_on_the_dialogue_box() {
        let narrative = talking_story(
            vec![
                Beat::ShowActor {
                    actor: ActorId::from("yuki"),
                    pose: "yuki_smile".into(),
                    slot: Slot::Left,
                },
                Beat::ShowActor {
                    actor: ActorId::from("kai"),
                    pose: "kai_neutral".into(),
                    slot: Slot::Right,
                },
                Beat::Say {
                    speaker: None,
                    text: ".".into(),
                },
            ],
            Exit::End { ending: None },
        );
        let bank = bank_with(&["yuki_smile", "kai_neutral"]);

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: None,
            hint: None,
        }
        .build();

        let sprites: Vec<_> = frame.quads.iter().filter(|q| q.texture.is_some()).collect();
        assert_eq!(sprites.len(), 2);

        // Actors are stored in a map keyed by id, so iteration order is name
        // order, not insertion order. Look them up by position instead.
        let left = sprites
            .iter()
            .find(|q| q.rect.x == 80.0)
            .expect("a sprite in the left slot");
        let right = sprites
            .iter()
            .find(|q| q.rect.x > WIDTH as f32 / 2.0)
            .expect("a sprite on the right half");
        for sprite in [left, right] {
            assert!(
                (sprite.rect.bottom() - DIALOGUE_BOX.y).abs() < 0.01,
                "sprites should rest just above the dialogue box"
            );
        }
    }

    #[test]
    fn choices_are_stacked_and_numbered() {
        let narrative = talking_story(
            vec![],
            Exit::Choose {
                prompt: Some("Pick one".into()),
                choices: vec![
                    Choice {
                        label: "First".into(),
                        to: NodeId::from("a"),
                    },
                    Choice {
                        label: "Second".into(),
                        to: NodeId::from("a"),
                    },
                ],
            },
        );
        let bank = TextureBank::new();

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: None,
            hint: None,
        }
        .build();

        let labels: Vec<&str> = frame
            .texts
            .iter()
            .map(|t| t.text.as_str())
            .filter(|t| t.contains(". "))
            .collect();
        assert_eq!(labels, ["1. First", "2. Second"]);

        // A choice body is the only quad that is both choice-sized: the prompt
        // plate shares the width and the edge bar shares the height.
        let boxes: Vec<_> = frame
            .quads
            .iter()
            .filter(|q| q.rect.width == CHOICE_WIDTH && q.rect.height == CHOICE_HEIGHT)
            .collect();
        assert_eq!(boxes.len(), 2);
        assert!(boxes[1].rect.y > boxes[0].rect.y, "choices stack downward");
    }

    #[test]
    fn a_gameplay_prompt_gets_its_own_band_above_the_scene() {
        let mut narrative = talking_story(
            vec![Beat::Say {
                speaker: None,
                text: "go".into(),
            }],
            Exit::Gameplay {
                spec: rustory_vn_core::GameplaySpec {
                    system: rustory_vn_core::SystemId::from("test"),
                    config: None,
                    outcomes: BTreeMap::new(),
                },
            },
        );
        narrative.tick(1000, 0);
        narrative.advance().expect("advance");
        let bank = TextureBank::new();

        let frame = Scene {
            narrative: &narrative,
            textures: &bank,
            gameplay_prompt: Some("Pick one (3.0s)"),
            hint: None,
        }
        .build();

        assert!(
            frame.texts.iter().any(|t| t.text == "Pick one (3.0s)"),
            "the gameplay prompt must reach the frame"
        );
    }

    #[test]
    fn display_names_capitalise_only_the_first_letter() {
        assert_eq!(display_name("yuki"), "Yuki");
        assert_eq!(display_name("yuki_tanaka"), "Yuki_tanaka");
        assert_eq!(display_name(""), "");
    }

    #[test]
    fn texture_lookup_accepts_a_name_with_or_without_an_extension() {
        let bank = bank_with(&["bg_classroom"]);
        assert!(bank.resolve("bg_classroom").is_some());
        assert!(bank.resolve("bg_classroom.png").is_some());
        assert!(bank.resolve("missing.png").is_none());
    }
}
