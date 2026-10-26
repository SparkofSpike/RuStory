//! `rustory` — probe the rendering stack.

fn main() {
    println!("rustory {}", env!("CARGO_PKG_VERSION"));

    match rustory_render::probe_gpu() {
        Ok(adapter) => println!("gpu:  {adapter}"),
        Err(err) => println!("gpu:  UNAVAILABLE — {err}"),
    }
    println!("text: {}", rustory_render::probe_text());
}
