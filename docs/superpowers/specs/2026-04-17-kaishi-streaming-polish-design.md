# Kaishi — Streaming Polish Design Spec

**Date:** 2026-04-17
**Status:** Draft
**Scope:** Streaming feel (P1), animation system, rename, layout chrome (documented/deferred)

---

## Summary

Bring Claude Code-level streaming polish to the Hermes TUI (renamed **Kaishi** — 懐紙).
Primary focus is on the _feel during active streaming_: animated spinner, shimmer effect,
phase-aware status, elapsed time, and turn completion summary. Layout chrome improvements
(status bar, prompt border, key hints) are documented but deferred to a later pass.

## Decisions Made

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Shimmer approach | Traveling highlight (ANSI-native) | Theme-compatible — no `Color::Rgb`. Three brightness levels via DarkGray/Gray/White. |
| Animation system | Dual-rate ticks (50ms animation + 200ms UI) | 20fps animation matches CC's `useAnimationFrame(50)`. Negligible CPU cost. |
| Spinner phases | Thinking / Streaming / Executing / Idle | Matches CC's context-aware labels. Future: word bank per phase. |
| Token display | Honest — only in turn completion summary | No fake estimates during streaming. Tokens from `session/prompt` response only. |
| Layout chrome | Documented, not implemented | Status bar, prompt border, key hints captured for future work. |
| Name | Kaishi (懐紙) | The paper for the tea ceremony — a surface for the exchange. |

---

## 1. Animation System — Dual-Rate Ticks

### Event Loop Changes

Add `AnimationTick` to the event enum. The event loop spawns two interval timers:

```rust
pub enum AppEvent {
    // ... existing variants ...
    Tick,           // ~200ms — UI refresh, cursor blink
    AnimationTick,  // ~50ms  — spinner frame, shimmer position, elapsed time
}
```

In `event.rs`, the existing tick timer stays at ~200ms. A second timer fires at ~50ms
and sends `AnimationTick`. Both coexist in the same `tokio::select!` loop.

### Animation Tick Handler

`AnimationTick` only:
1. Bumps `animation.frame` (spinner glyph index)
2. Advances `animation.shimmer_pos` (every 3rd tick → ~150ms shimmer speed)
3. Triggers a re-render

It **never** touches ACP state, the message list, or the line cache. The draw function
reads `AnimationState` to render the spinner line — that single line is rebuilt each
frame, everything else comes from cache.

### Performance

The line cache (existing) already avoids rebuilding message Spans on every frame.
Animation ticks only cause the spinner line (1-2 Lines) to be reconstructed. The
full-frame double-buffer redraw is cheap — ratatui only writes changed cells to the
terminal via crossterm's diff algorithm.

---

## 2. AnimationState Struct

New struct added to `App`:

```rust
pub struct AnimationState {
    // Spinner
    pub frame: usize,              // index into bounce sequence
    pub phase: AgentPhase,         // Thinking | Streaming | Executing | Idle
    pub phase_start: Instant,      // when current phase began
    pub last_output: Instant,      // last content received (future: stall detection)

    // Shimmer (traveling highlight)
    pub shimmer_pos: usize,        // which char in the label is highlighted
    pub shimmer_tick: u8,          // sub-counter — advance every 3rd animation tick

    // Executing tool context
    pub active_tool: Option<String>, // name of currently running tool
}

#[derive(Debug, Clone, PartialEq)]
pub enum AgentPhase {
    Idle,
    Thinking,
    Streaming,
    Executing,
}
```

### Phase Transitions

| Event | New Phase | Reset |
|-------|-----------|-------|
| Prompt sent (no response yet) | Thinking | `phase_start = now()` |
| First `agent_thought_chunk` | Thinking | (already Thinking, no reset) |
| First `agent_message_chunk` | Streaming | `phase_start = now()` |
| `tool_call` event | Executing | `phase_start = now()`, `active_tool = Some(name)` |
| `tool_call_update` (completed) | Thinking | `phase_start = now()`, `active_tool = None` (model will process tool result → next thought/message chunk determines real phase) |
| `prompt` response (stopReason) | Idle | clear all |

Phase transitions are handled in the existing `handle_*` methods in `app.rs`.
Each transition resets `phase_start` so elapsed time is per-phase.

---

## 3. Spinner Glyph

### Characters

```rust
const GLYPHS: &[char] = &['·', '✢', '✳', '✶', '✻', '✽'];
```

### Bounce Sequence

Forward then reverse, skipping endpoints to avoid stutter:

```rust
fn bounce_sequence() -> Vec<char> {
    let mut seq: Vec<char> = GLYPHS.to_vec();
    seq.extend(GLYPHS.iter().rev().skip(1).take(GLYPHS.len() - 2));
    seq // [· ✢ ✳ ✶ ✻ ✽ ✻ ✶ ✳ ✢]
}
```

Full cycle = 10 frames × ~120ms per frame = ~1.2s. Matches CC's breathing rhythm.

### Advancement

The spinner frame advances every ~120ms (roughly every 2-3 animation ticks at 50ms).
Use a sub-counter in `AnimationState` similar to the shimmer tick.

### Color

The spinner glyph uses `palette::ACCENT_ASSISTANT` (Magenta) — same as the `◆` role
indicator, keeping visual consistency with the assistant's identity.

---

## 4. Shimmer — Traveling Highlight

### Concept

A single bright character travels across the phase label text ("thinking", "streaming",
"executing"), with adjacent characters getting a half-brightness lift. Pure ANSI — no RGB.

### Color Mapping

Three brightness levels using terminal-native colors:

```rust
const SHIMMER_DIM: Color = Color::DarkGray;  // base text
const SHIMMER_MID: Color = Color::Gray;       // adjacent to highlight
const SHIMMER_HI:  Color = Color::White;      // the highlighted char

fn shimmer_color(char_idx: usize, highlight_pos: usize) -> Color {
    let dist = char_idx.abs_diff(highlight_pos);
    match dist {
        0 => SHIMMER_HI,
        1 => SHIMMER_MID,
        _ => SHIMMER_DIM,
    }
}
```

On Catppuccin (and other well-configured themes):
- `DarkGray` → the theme's muted gray (subtle)
- `Gray` → mid-tone (visible but not dominant)
- `White` → the theme's bright text (the highlight pop)

This gives a 3-step brightness wave that respects whatever palette the terminal maps.

### Advancement

Shimmer position advances every ~150ms (every 3rd animation tick). After reaching the
last character, it wraps to position 0. The label text changes per phase:

| Phase | Label | Shimmer length |
|-------|-------|----------------|
| Thinking | `thinking` | 8 chars |
| Streaming | `streaming` | 9 chars |
| Executing | `executing` | 9 chars |

### Rendering

The spinner line is rendered as a `Vec<Span>` where each character of the label gets
its own Span with the appropriate shimmer color. The elapsed time and separator are
always `Color::DarkGray` (no shimmer).

```
  ✶ t h i n k i n g  · 3s
  ^  \___________/      ^^
  |   shimmer chars      |
  glyph (Magenta)     dim (DarkGray)
```

---

## 5. Spinner Line Format

### Layout

```
  {glyph} {shimmer_label} · {elapsed}
```

Where:
- `{glyph}` — current bounce sequence character, colored Magenta
- `{shimmer_label}` — phase label with traveling highlight
- `{elapsed}` — seconds since phase start, DarkGray, format: `3s` / `1m12s`

### Indentation

Two spaces of leading indent, matching the assistant message `◆` indent. This keeps
the spinner visually aligned with the conversation flow.

### Placement

The spinner line renders at the bottom of the message area, below the last message
and above the input box. It only appears when `phase != Idle`.

When the phase transitions to Idle, the spinner line disappears and is replaced by
the turn completion summary (section 6).

---

## 6. Turn Completion Summary

When the `session/prompt` response arrives with `stopReason` and `usage`, append a
dim divider line after the assistant's final message:

```
  ── 1,234 in · 567 out · 12.3s ──
```

### Data Sources

- `input_tokens` / `output_tokens` — from the prompt response's `usage` field
- Elapsed time — `Instant::now() - turn_start` (track `turn_start` when prompt is sent)
- Stored by extending `Usage` with `elapsed_secs: Option<f64>`, set in `handle_prompt_done()`

### Format

- Entire line in `Color::DarkGray`
- Leading/trailing `──` dashes fill to ~40 chars or content width, whichever is smaller
- Token counts formatted with comma separators for readability (`1,234`)
- Time formatted as `Xs` or `Xm Ys`

### Implementation

The summary is **not** a separate message. It's rendered as part of the last assistant
message's display. `ChatMessage` already has `tokens: Option<Usage>` — when the renderer
encounters an assistant message with `Some(usage)`, it appends the dim divider as the
final cached line(s) of that message block.

This keeps the message list clean (only real conversation), attaches metadata to the
message it describes, and works naturally with the line cache and history replay.

---

## 7. Integration Points in Existing Code

### `app.rs`

- Add `AnimationState` field to `App`
- Add `AgentPhase` enum
- Add phase transition logic to `handle_agent_thought()`, `handle_agent_message()`,
  `handle_tool_start()`, `handle_tool_update()`, `handle_prompt_done()`
- Track `turn_start: Option<Instant>` for total turn elapsed time
- `handle_animation_tick()` method: bump frame, shimmer, trigger redraw

### `event.rs`

- Add `AnimationTick` variant to `AppEvent`
- Second interval timer in the event loop (50ms)

### `ui.rs`

- New function `render_spinner_line()` that reads `AnimationState` and produces a
  `Vec<Line>` with the shimmer Spans
- Called in `draw_chat()` after the message area, before the input box
- `render_turn_summary()` for the completion divider
- The spinner line is NOT added to the line cache — it's rebuilt every frame

### `main.rs`

- Handle `AnimationTick` in the main event dispatch (just calls `app.handle_animation_tick()`)

---

## 8. Rename: hermes-tui → Kaishi

### Touches

- `Cargo.toml` — package name, binary name
- `main.rs` — any hardcoded "hermes-tui" strings
- `ui.rs` — status bar title
- `ui_picker.rs` — header
- `.gitignore`, `README.md` if present
- Git repo rename on GitHub (eva-l3n4/hermes-tui → eva-l3n4/kaishi)

### Binary Name

`kaishi` — invoked as `kaishi` or `kaishi --profile hanami`.

---

## 9. Layout Chrome (Documented, Deferred)

Not in scope for this implementation pass. Captured here for future reference.

### Status Bar

```
kaishi │ opus-4-6 │ ctx:42% │ manual │ ~/project
```

New fields (future): context window %, permission mode, CWD instead of session ID.
Requires upstream ACP support for context window stats.

### Prompt Input Border

Rounded corners (╭╮╰╯) with embedded key hints in the bottom border:

```
╭──────────────────────────────────────╮
│ _                                    │
╰─ esc interrupt · / commands ─────────╯
```

### Keyboard Shortcut Hints

Contextual hints in the footer area, matching CC's `KeyboardShortcutHint` pattern:

```
↑/↓ navigate · enter select · n new · q quit
```

### Phase Word Bank (Future Polish)

Instead of static "thinking" / "streaming" / "executing", rotate through themed words:

```rust
const THINKING_WORDS: &[&str] = &["thinking", "pondering", "reasoning", "considering"];
const STREAMING_WORDS: &[&str] = &["streaming", "composing", "writing", "crafting"];
const EXECUTING_WORDS: &[&str] = &["executing", "running", "working", "processing"];
```

Select randomly on each phase entry. Pure flavor — no functional impact.

---

## 10. Non-Goals

- **RGB color interpolation** — explicitly rejected for theme compatibility
- **Stall detection** (spinner turns red) — P2, not in this pass
- **Grouped/collapsed tool calls** — P3, significant rendering complexity
- **Reduced-motion mode** — P3, good accessibility practice but not urgent
- **Cost tracking** — requires upstream support, deferred
- **Thinking collapse/expand toggle** — P2, needs keybinding design
