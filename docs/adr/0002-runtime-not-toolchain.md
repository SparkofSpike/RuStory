# ADR 0002 — The engine is a runtime, not an authoring toolchain

- **Status**: Accepted
- **Date**: 2026-10-26
- **Open question**: is the engine a runtime or a toolchain?

---

## Context

The word "generator" carried two possible meanings in early project discussions,
and they were used interchangeably:

- **Runtime**: the engine reads a script, interprets it, and produces playable
  output.
- **Toolchain**: software that helps a creator (human or AI) produce scripts,
  sprites, voice and branching structure.

Both can be called "an engine", but they are entirely different engineering
efforts. Without settling this first, the MVP scope cannot converge.

## Options

### A. Runtime only

The engine consumes finished scripts. Authoring happens in an external editor or
in an AI tool.

### B. Toolchain only

The engine is authoring assistance: generate scripts, validate structure, preview
assets. It does not play anything.

### C. Runtime as the core, toolchain as a satellite

The engine is a runtime. Tooling exists as separate programs that **share one data
model** — a validator, a previewer, eventually a visual editor.

## Decision

**Adopt C, and build only the A part during the MVP.**

## Reasoning

1. **Without a runtime, a toolchain has nothing to validate against.** A script
   generator that cannot prove its output plays correctly produces output nobody
   should trust. The runtime is the toolchain's **acceptance test**.
2. **A shared data model is what makes tooling cheap.** `rustory-core` already
   defines `Story`, `Node`, `Beat` and `Exit`. Any future tool solves against that
   model instead of inventing its own. `rustory check` in this repository is
   already an instance of such a tool.
3. **Building tooling early distorts the architecture.** Generators need LLM
   calls, asset management and version-control integration — dependencies the core
   does not need. The core must stay at zero rendering and zero network.
4. **The real pain is not "I cannot write a script", it is "I cannot plug gameplay
   in".** The MVP should prove the latter first.

## Consequences

**Gained**

- A crisp MVP boundary: read a RON file, play it, stop.
- The core keeps zero external service dependencies and tests without a network.
- Tools can be added later and naturally reuse the same model.

**Paid**

- Authors have to write RON by hand for now. The syntax is more verbose than
  prose (`Some(...)`, `End()`).
- No visual preview; inspecting a scene means `shot` and `play`.

**The interface left open for later**

`rustory-script` is the only entry point for the format. If a DSL or an
LLM-driven generation flow is added later, it compiles **to** `rustory_core::Story`
rather than changing the core. That constraint keeps any toolchain a pluggable
satellite instead of a second engine.

## Verification

`rustory check` exists. It reports node count, beat count, choice points and
gameplay handovers, and warns when a script references a gameplay system that is
not registered. It demonstrates that a tool can consume the same model.
