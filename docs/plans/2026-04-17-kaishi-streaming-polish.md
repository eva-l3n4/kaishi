# Kaishi Streaming Polish — Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Add CC-level streaming polish to hermes-tui (now Kaishi): animated spinner with bounce cycle, shimmer effect, phase-aware status, elapsed time, turn completion summary, and rename.

**Architecture:** Add `AnimationState` struct to `App`, dual-rate tick system (50ms animation + 200ms UI), render spinner line outside the line cache. Phase transitions driven by existing ACP event handlers.

**Tech Stack:** Rust, ratatui 0.29, crossterm 0.28, tokio

**Spec:** `docs/superpowers/specs/2026-04-17-kaishi-streaming-polish-design.md`

---

## Task 1: Add AnimationState and AgentPhase to app.rs

**Objective:** Define the animation data structures and wire them into App state.

**Files:**
- Modify: `src/app.rs`
- Modify: `src/event.rs`

**Step 1: Add AgentPhase enum and AnimationState struct to app.rs**

Add after the existing `AgentStatus` enum (line 36):

```rust
/// What the agent is actively doing (drives spinner animation).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentPhase {
    Idle,
    Thinking,
    Streaming,
    Executing,
}

/// Animation state for the spinner line — updated on AnimationTick.
pub struct AnimationState {
    /// Index into the bounce sequence.
    pub frame: usize,
    /// Sub-counter for spinner advancement (~120ms).
    pub spinner_tick: u8,
    /// Current agent phase.
    pub phase: AgentPhase,
    /// When the current phase began.
    pub phase_start: std::time::Instant,
    /// Last time content was received (future: stall detection).
    pub last_output: std::time::Instant,
    /// Shimmer highlight position (which char in the label).
    pub shimmer_pos: usize,
    /// Sub-counter for shimmer advancement (~150ms).
    pub shimmer_tick: u8,
    /// Name of currently executing tool.
    pub active_tool: Option<String>,
    /// When the current turn (prompt) started.
    pub turn_start: Option<std::time::Instant>,
}

impl AnimationState {
    pub fn new() -> Self {
        let now = std::time::Instant::now();
        Self {
            frame: 0,
            spinner_tick: 0,
            phase: AgentPhase::Idle,
            phase_start: now,
            last_output: now,
            shimmer_pos: 0,
            shimmer_tick: 0,
            active_tool: None,
            turn_start: None,
        }
    }

    /// Transition to a new phase, resetting timers.
    pub fn set_phase(&mut self, phase: AgentPhase) {
        if self.phase != phase {
            self.phase = phase;
            self.phase_start = std::time::Instant::now();
            self.shimmer_pos = 0;
            self.shimmer_tick = 0;
        }
    }
}
```

**Step 2: Add `elapsed_secs` to the Usage struct in event.rs**

In `src/event.rs`, modify the `Usage` struct (line 8):

```rust
#[derive(Debug, Clone)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_secs: Option<f64>,
}
```

**Step 3: Add `animation` field to App struct**

In the App struct (after line 83 `pub tick: u64`), add:

```rust
    pub animation: AnimationState,
```

In `App::new()` (around line 138), add to the initializer:

```rust
            animation: AnimationState::new(),
```

**Step 4: Verify it compiles**

Run: `cd /home/opus/hermes-tui && cargo check 2>&1`

Fix any compilation errors (the `Usage` change may require updating call sites that construct `Usage` — search for `Usage {` in `acp.rs` and `main.rs`).

**Step 5: Commit**

```bash
git add -u && git commit -m "feat: add AnimationState, AgentPhase, and Usage.elapsed_secs"
```

---

## Task 2: Add AnimationTick to the event loop

**Objective:** Add the 50ms animation tick alongside the existing ~200ms UI tick.

**Files:**
- Modify: `src/event.rs`

**Step 1: Add AnimationTick variant to AppEvent**

In the `AppEvent` enum (after `Tick` on line 40), add:

```rust
    AnimationTick,
```

**Step 2: Add second timer to the event loop**

In `EventLoop::new()` (line 98), the existing task spawns one tick timer. Add a second one. Modify the spawned async block to run two timers. The simplest approach: spawn a second task for the animation tick:

After the existing `tokio::spawn(async move { ... })` block (around line 140), add:

```rust
        // Animation tick — 50ms for smooth spinner/shimmer
        let anim_tx = tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(50));
            loop {
                interval.tick().await;
                if anim_tx.send(AppEvent::AnimationTick).is_err() {
                    break;
                }
            }
        });
```

**Step 3: Verify it compiles**

Run: `cargo check`

**Step 4: Commit**

```bash
git add -u && git commit -m "feat: add 50ms AnimationTick to event loop"
```

---

## Task 3: Handle AnimationTick in main.rs and app.rs

**Objective:** Wire up the animation tick handler to bump frame/shimmer counters.

**Files:**
- Modify: `src/main.rs`
- Modify: `src/app.rs`

**Step 1: Add handle_animation_tick to App**

In `src/app.rs`, add this method (after the existing `tick()` method around line 164):

```rust
    /// Advance animation counters (called every 50ms).
    pub fn handle_animation_tick(&mut self) {
        if self.animation.phase == AgentPhase::Idle {
            return; // No animation when idle
        }

        // Spinner glyph: advance every ~120ms (every 2-3 ticks at 50ms)
        self.animation.spinner_tick += 1;
        if self.animation.spinner_tick >= 2 {
            self.animation.spinner_tick = 0;
            self.animation.frame = self.animation.frame.wrapping_add(1);
        }

        // Shimmer: advance every ~150ms (every 3rd tick)
        self.animation.shimmer_tick += 1;
        if self.animation.shimmer_tick >= 3 {
            self.animation.shimmer_tick = 0;
            let label_len = match self.animation.phase {
                AgentPhase::Thinking => 8,   // "thinking"
                AgentPhase::Streaming => 9,  // "streaming"
                AgentPhase::Executing => 9,  // "executing"
                AgentPhase::Idle => 1,
            };
            self.animation.shimmer_pos = (self.animation.shimmer_pos + 1) % label_len;
        }
    }
```

**Step 2: Dispatch AnimationTick in main.rs**

In the main event dispatch loop (around line 191), add after the `Tick` handler:

```rust
            event::AppEvent::AnimationTick => {
                app.handle_animation_tick();
            }
```

**Step 3: Verify it compiles**

Run: `cargo check`

**Step 4: Commit**

```bash
git add -u && git commit -m "feat: handle AnimationTick — bump spinner frame and shimmer pos"
```

---

## Task 4: Wire phase transitions into existing handlers

**Objective:** Set the correct AgentPhase when ACP events arrive.

**Files:**
- Modify: `src/app.rs`

**Step 1: Set Thinking phase when prompt is sent**

Find where the prompt is sent (in `handle_key` or wherever `status = AgentStatus::Thinking` is set on Enter press, around line 400). Add alongside it:

```rust
self.animation.set_phase(AgentPhase::Thinking);
self.animation.turn_start = Some(std::time::Instant::now());
```

**Step 2: Set Streaming phase on first message chunk**

In `handle_agent_message()` (line 794), add at the top:

```rust
        if self.animation.phase != AgentPhase::Streaming {
            self.animation.set_phase(AgentPhase::Streaming);
        }
        self.animation.last_output = std::time::Instant::now();
```

**Step 3: Keep Thinking on thought chunks**

In `handle_agent_thought()` (line 801), add:

```rust
        if self.animation.phase == AgentPhase::Idle {
            self.animation.set_phase(AgentPhase::Thinking);
        }
        self.animation.last_output = std::time::Instant::now();
```

**Step 4: Set Executing on tool_call**

In `handle_tool_start()` (line 810), add after the existing code:

```rust
        self.animation.set_phase(AgentPhase::Executing);
        self.animation.active_tool = Some(name.to_string());
```

**Step 5: Return to Thinking on tool completion**

In `handle_tool_update()` (line 834), when status is "completed" or "error" and `active_tools` is now empty, add:

```rust
        if self.active_tools.is_empty() {
            self.animation.set_phase(AgentPhase::Thinking);
            self.animation.active_tool = None;
        }
```

**Step 6: Set Idle on prompt done**

In `handle_prompt_done()` (line 893), add:

```rust
        // Compute elapsed time for turn summary
        let elapsed = self.animation.turn_start
            .map(|t| t.elapsed().as_secs_f64());

        self.animation.set_phase(AgentPhase::Idle);
        self.animation.active_tool = None;
        self.animation.turn_start = None;
```

Also modify the `flush_pending_response` call to pass elapsed time into usage. Update the usage construction:

```rust
        // Attach elapsed time to usage
        let usage_with_elapsed = usage.map(|mut u| {
            u.elapsed_secs = elapsed;
            u
        });
        self.flush_pending_response(usage_with_elapsed);
```

**Step 7: Verify it compiles**

Run: `cargo check`

**Step 8: Commit**

```bash
git add -u && git commit -m "feat: wire AgentPhase transitions into ACP event handlers"
```

---

## Task 5: Render the spinner line with shimmer

**Objective:** Replace the old braille spinner with the new animated spinner line in the message area.

**Files:**
- Modify: `src/ui.rs`

**Step 1: Add spinner constants and shimmer function**

Replace the old `SPINNER` constant (line 36) with:

```rust
/// Spinner glyph characters (CC-style).
const GLYPHS: &[char] = &['·', '✢', '✳', '✶', '✻', '✽'];

/// Build the bounce sequence: forward then reverse, skipping endpoints.
fn bounce_sequence() -> Vec<char> {
    let mut seq: Vec<char> = GLYPHS.to_vec();
    seq.extend(GLYPHS.iter().rev().skip(1).take(GLYPHS.len() - 2));
    seq // [· ✢ ✳ ✶ ✻ ✽ ✻ ✶ ✳ ✢]
}

/// Three-step shimmer: highlighted char, adjacent, and base.
fn shimmer_color(char_idx: usize, highlight_pos: usize) -> Color {
    let dist = if char_idx >= highlight_pos {
        char_idx - highlight_pos
    } else {
        highlight_pos - char_idx
    };
    match dist {
        0 => Color::White,    // highlight
        1 => Color::Gray,     // adjacent
        _ => Color::DarkGray, // base
    }
}

/// Format elapsed seconds as "Xs" or "Xm Ys".
fn format_elapsed(secs: f64) -> String {
    let s = secs as u64;
    if s < 60 {
        format!("{}s", s)
    } else {
        format!("{}m {}s", s / 60, s % 60)
    }
}
```

**Step 2: Add render_spinner_line function**

```rust
/// Render the animated spinner line (thinking/streaming/executing).
fn render_spinner_line(app: &App) -> Option<Line<'static>> {
    use crate::app::AgentPhase;

    if app.animation.phase == AgentPhase::Idle {
        return None;
    }

    let bounce = bounce_sequence();
    let glyph = bounce[app.animation.frame % bounce.len()];

    let label = match app.animation.phase {
        AgentPhase::Thinking => "thinking",
        AgentPhase::Streaming => "streaming",
        AgentPhase::Executing => "executing",
        AgentPhase::Idle => return None,
    };

    let elapsed = format_elapsed(app.animation.phase_start.elapsed().as_secs_f64());

    // Build spans
    let mut spans: Vec<Span> = Vec::new();

    // Leading indent
    spans.push(Span::raw("  "));

    // Glyph (accent color)
    spans.push(Span::styled(
        glyph.to_string(),
        Style::default().fg(palette::ACCENT_ASSISTANT),
    ));

    spans.push(Span::raw(" "));

    // Shimmer label — each char gets its own span
    for (i, ch) in label.chars().enumerate() {
        spans.push(Span::styled(
            ch.to_string(),
            Style::default().fg(shimmer_color(i, app.animation.shimmer_pos)),
        ));
    }

    // Separator + elapsed
    spans.push(Span::styled(
        format!(" · {}", elapsed),
        Style::default().fg(Color::DarkGray),
    ));

    Some(Line::from(spans))
}
```

**Step 3: Integrate into draw_messages**

In `draw_messages()` (line 176), after building `all_lines` from the line cache and any streaming content, add the spinner line at the end (before creating the Paragraph):

Find the spot where `all_lines` is complete and the `Paragraph` is about to be created. Add:

```rust
    // Animated spinner line (not cached — rebuilt every frame)
    if let Some(spinner_line) = render_spinner_line(app) {
        all_lines.push(spinner_line);
    }
```

**Step 4: Remove old spinner from status bar**

In `draw_status_bar()` (line 90), the `AgentStatus::Thinking` branch currently shows the braille spinner. Replace it with a simple status label since the spinner is now in the message area:

```rust
        AgentStatus::Thinking => {
            if let Some(ref tool_name) = app.animation.active_tool {
                format!(" {} > {}", model, tool_name)
            } else {
                format!(" {} > working…", model)
            }
        }
```

**Step 5: Verify it compiles and test visually**

Run: `cargo check`
Then: `cargo run` — send a message and verify the spinner appears with shimmer.

**Step 6: Commit**

```bash
git add -u && git commit -m "feat: animated spinner line with shimmer and phase-aware labels"
```

---

## Task 6: Render turn completion summary

**Objective:** Show `── tokens in · tokens out · elapsed ──` after completed turns.

**Files:**
- Modify: `src/ui.rs`

**Step 1: Add render_turn_summary function**

```rust
/// Render a turn completion summary as a dim divider line.
fn render_turn_summary(usage: &Usage, width: usize) -> Line<'static> {
    let in_tok = format_tokens(usage.input_tokens);
    let out_tok = format_tokens(usage.output_tokens);
    let elapsed = usage.elapsed_secs
        .map(|s| format_elapsed(s))
        .unwrap_or_default();

    let content = if elapsed.is_empty() {
        format!("{} in · {} out", in_tok, out_tok)
    } else {
        format!("{} in · {} out · {}", in_tok, out_tok, elapsed)
    };

    // Center with ── dashes
    let content_width = content.len() + 2; // spaces around content
    let available = width.saturating_sub(4); // minimum 2 dashes each side
    let dash_total = available.saturating_sub(content_width);
    let left_dashes = dash_total / 2;
    let right_dashes = dash_total - left_dashes;

    let line_str = format!(
        "  {}── {} ──{}",
        "─".repeat(left_dashes),
        content,
        "─".repeat(right_dashes),
    );

    Line::from(Span::styled(line_str, Style::default().fg(Color::DarkGray)))
}

/// Format token count with comma separators.
fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{},{:03}", n / 1_000, n % 1_000)
    } else {
        format!("{},{:03},{:03}", n / 1_000_000, (n / 1_000) % 1_000, n % 1_000)
    }
}
```

**Step 2: Integrate into message rendering**

In the message rendering function (where each `ChatMessage` is rendered), when an assistant message has `tokens: Some(usage)`, append the summary line after the message content:

Find where `Role::Assistant` messages are rendered and their lines are added to the cache. After the message content lines, add:

```rust
    if let Some(ref usage) = msg.tokens {
        lines.push(render_turn_summary(usage, width));
    }
```

**Step 3: Verify it compiles**

Run: `cargo check`

**Step 4: Commit**

```bash
git add -u && git commit -m "feat: render turn completion summary with token counts and elapsed time"
```

---

## Task 7: Rename hermes-tui → kaishi

**Objective:** Rename the project everywhere.

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/app.rs`
- Modify: `src/ui.rs`
- Modify: `src/ui_picker.rs`

**Step 1: Update Cargo.toml**

Change:
```toml
[package]
name = "kaishi"
version = "0.5.0"
edition = "2021"
description = "懐紙 Kaishi — Terminal UI for Hermes Agent"

[[bin]]
name = "kaishi"
path = "src/main.rs"
```

**Step 2: Update welcome message in app.rs**

In `App::new()` (line 124), change the welcome message:

```rust
            messages: vec![ChatMessage {
                role: Role::System,
                content: "Welcome to 懐紙 Kaishi. Type a message or /help for commands."
                    .into(),
                tokens: None,
            }],
```

**Step 3: Update status bar title in ui.rs**

In `draw_status_bar()`, replace any "hermes-tui" or "hermes" fallback with "kaishi":

```rust
    let model = if app.model_name.is_empty() {
        "kaishi"
    } else {
        &app.model_name
    };
```

**Step 4: Update picker header in ui_picker.rs**

Find the header text and change "hermes-tui" or "Hanami" to "Kaishi" / "懐紙".

**Step 5: Verify it compiles and the binary name is correct**

Run: `cargo build && ls target/debug/kaishi`

**Step 6: Commit**

```bash
git add -u && git commit -m "feat: rename hermes-tui to kaishi (懐紙)"
```

---

## Task 8: Polish and verify

**Objective:** End-to-end verification and cleanup.

**Files:**
- All source files

**Step 1: Run clippy**

```bash
cargo clippy --all-targets -- -D warnings
```

Fix any warnings.

**Step 2: Test the full flow**

1. `cargo run` — verify picker shows "Kaishi" header
2. Create/resume a session
3. Send a message — verify:
   - Spinner line appears with glyph animation
   - "thinking" label has traveling shimmer
   - Elapsed time counts up
   - Phase transitions (thinking → executing → thinking → streaming)
   - Turn completion summary appears after response

**Step 3: Verify the old braille spinner is completely removed**

Search for any remaining references:

```bash
grep -rn "⠋\|⠙\|⠹\|⠸\|braille" src/
```

**Step 4: Remove dead code**

Remove any unused `AgentStatus` variants (the old `Thinking` variant may now be partially redundant since `AgentPhase` handles the animation state — but `AgentStatus` still gates input acceptance, so keep it but audit).

**Step 5: Final commit**

```bash
git add -u && git commit -m "chore: clippy fixes and cleanup after kaishi polish"
```
