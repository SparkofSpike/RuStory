//! Example RuStory gameplay module: "pick the right door", in raw ABI.
//!
//! This exists to be loaded by a real story, not to be a good game. It is written with no
//! dependencies and no `serde`, because the two things it needs to prove are:
//!
//! 1. The documented ABI is small enough to implement by hand.
//! 2. The fuel budget is survivable for code a person actually wrote — which is a measurement,
//!    not a guess, and the number is printed by the host when `RUSTORY_PLUGIN_FUEL` is set.
//!
//! It does not parse RON. A real guest would; this one looks for `index:N` in the input payload,
//! which is enough to answer "which option did the player pick" and keeps the code small enough
//! that its fuel cost is mostly the ABI's own overhead rather than a parser's.
//!
//! Build:
//!
//! ```console
//! cargo build --release --target wasm32-unknown-unknown --manifest-path examples/wasm-guest/Cargo.toml
//! cp examples/wasm-guest/target/wasm32-unknown-unknown/release/rustory_wasm_guest_example.wasm \
//!    plugins/doors.wasm
//! ```

#![no_std]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

/// One page of scratch memory, reused across calls.
///
/// A bump allocator that never frees is fine here: the host allocates one buffer per call and
/// frees it immediately, and the module's whole lifetime is one gameplay handover.
const HEAP: usize = 4096;
static mut HEAP_TOP: usize = 0;

/// Where the result lives. Fixed, so `result_ptr` is a constant.
const RESULT: usize = 8192;
static mut RESULT_LEN: usize = 0;

/// The option index that wins. Set from the config, which carries `correct: N`.
static mut CORRECT: u32 = 0;
/// Which round we are on, for the view.
static mut ROUND: u32 = 0;
/// What the player picked, or `u32::MAX` for "nothing yet".
static mut PICKED: u32 = u32::MAX;
/// How much time has passed, so the view can show a clock.
static mut ELAPSED_MS: u32 = 0;

/// Give the host a buffer to write into.
///
/// # Safety
/// Called by the host with a length it then fills. The pointer stays valid until `dealloc`.
#[no_mangle]
pub extern "C" fn alloc(len: i32) -> i32 {
    let len = len.max(0) as usize;
    if len > HEAP {
        return -1;
    }
    // Safety: single-threaded module; `HEAP_TOP` is only touched here and in `dealloc`.
    unsafe {
        HEAP_TOP = 0;
    }
    HEAP_BASE as i32
}

const HEAP_BASE: usize = 1024;

#[no_mangle]
pub extern "C" fn dealloc(_ptr: i32, _len: i32) {}

#[no_mangle]
pub extern "C" fn result_ptr() -> i32 {
    RESULT as i32
}

#[no_mangle]
pub extern "C" fn result_len() -> i32 {
    // Safety: written by `step` and `view` before the host reads it.
    unsafe { RESULT_LEN as i32 }
}

/// Read an `i32` from guest memory.
///
/// # Safety
/// `ptr` and `len` come from the host's `alloc`, so they name a range inside this module.
unsafe fn bytes<'a>(ptr: i32, len: i32) -> &'a [u8] {
    core::slice::from_raw_parts(ptr as *const u8, len.max(0) as usize)
}

/// Find `needle` in `haystack`, returning the index just past it.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let mut start = 0;
    while start + needle.len() <= haystack.len() {
        if &haystack[start..start + needle.len()] == needle {
            return Some(start + needle.len());
        }
        start += 1;
    }
    None
}

/// Parse the digits after `needle`, if any.
fn number_after(haystack: &[u8], needle: &[u8]) -> Option<u32> {
    let mut at = find(haystack, needle)?;
    // Skip whitespace and a colon the payload may put between the name and the value.
    while at < haystack.len() && (haystack[at] == b' ' || haystack[at] == b':') {
        at += 1;
    }
    let mut value: u32 = 0;
    let mut digits = 0;
    while at < haystack.len() && haystack[at].is_ascii_digit() {
        value = value.saturating_mul(10).saturating_add(u32::from(haystack[at] - b'0'));
        at += 1;
        digits += 1;
    }
    (digits > 0).then_some(value)
}

fn write(bytes: &[u8]) {
    // Safety: `RESULT` is a fixed buffer this module owns, and `RESULT_LEN` is only written here.
    unsafe {
        let len = bytes.len().min(1024);
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), RESULT as *mut u8, len);
        RESULT_LEN = len;
    }
}

#[no_mangle]
pub extern "C" fn begin(ptr: i32, len: i32) -> i32 {
    // Safety: the host allocated this range.
    let payload = unsafe { bytes(ptr, len) };
    // `correct: 0` arrives inside the config, which is nested in the payload. The last `correct`
    // wins, which is what a single-round config means.
    let correct = number_after(payload, b"correct").unwrap_or(0);
    // Safety: no other code runs concurrently.
    unsafe {
        CORRECT = correct;
        ROUND = 0;
        PICKED = u32::MAX;
        ELAPSED_MS = 0;
        RESULT_LEN = 0;
        HEAP_TOP = 0;
    }
    0
}

#[no_mangle]
pub extern "C" fn step(dt: i32, ptr: i32, len: i32) -> i32 {
    // Safety: the host allocated this range.
    let inputs = unsafe { bytes(ptr, len) };
    // Safety: single-threaded.
    unsafe {
        ELAPSED_MS = ELAPSED_MS.saturating_add(dt.max(0) as u32);
    }

    if let Some(index) = number_after(inputs, b"index") {
        unsafe {
            PICKED = index;
        }
        let win = index == unsafe { CORRECT };
        write(if win { b"won" } else { b"lost" });
        return 1;
    }

    // Nothing picked: keep control. The clock running out is the host's business in a real
    // system; here it just makes the bar move.
    0
}

#[no_mangle]
pub extern "C" fn cancel() {
    unsafe {
        ROUND = 0;
        PICKED = u32::MAX;
        ELAPSED_MS = 0;
        RESULT_LEN = 0;
    }
}

/// The screen: a prompt, the two doors, and a clock that fills.
///
/// Written by hand as RON because a guest that pulled in `serde` would be a different example:
/// this one is about the boundary, and every dependency would blur whose cost is whose.
#[no_mangle]
pub extern "C" fn view() -> i32 {
    write(b"(elements:[\
Text(text:\"Two doors. One opens.\",rect:(x:140.0,y:80.0,width:1000.0,height:60.0),size:40.0,tone:Text,align:Center),\
Panel(rect:(x:0.0,y:0.0,width:1280.0,height:720.0),tone:Panel,border:None),\
Panel(rect:(x:240.0,y:260.0,width:360.0,height:220.0),tone:Raised,border:Some(Edge)),\
Panel(rect:(x:680.0,y:260.0,width:360.0,height:220.0),tone:Raised,border:Some(Edge)),\
Text(text:\"1. Left\",rect:(x:280.0,y:340.0,width:280.0,height:50.0),size:32.0,tone:Text,align:Start),\
Text(text:\"2. Right\",rect:(x:720.0,y:340.0,width:280.0,height:50.0),size:32.0,tone:Text,align:Start),\
Bar(rect:(x:240.0,y:560.0,width:800.0,height:14.0),fraction:0.35,tone:Accent)
])");
    1
}
