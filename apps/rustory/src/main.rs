//! `rustory` — run a RuStory script, or validate one.

fn main() {
    println!("rustory 0.1.0 — scaffold only, no playable slice yet");
    println!("core:   {}", rustory_core::crate_name());
    println!("script: {}", rustory_script::crate_name());
    println!("render: {}", rustory_render::crate_name());
}
