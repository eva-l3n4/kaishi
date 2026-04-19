# Subagent Zoom View Implementation Plan

> **Status: SHIPPED 2026-04-19** — see commit table at bottom.

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Let the user click a running subagent task in Kaishi to zoom into a full-screen live view of that child session, and pop back out with a single keystroke, without interacting with the child.

**Architecture:** Observation-only, wire-layer minimal. Hermes's `delegate_tool.py` already emits `subagent.*` events on the parent's `tool_progress_callback`, and every child runs as a real `AIAgent` with its own `session_id` persisted to the shared `SessionDB`. The ACP adapter currently swallows non-`tool.started` events. This plan extends the adapter to bridge `subagent.*` events through a new `_hermes/subagent_update` notification, keyed by `child_session_id`. Kaishi adds a task-line widget in the transcript and a full-screen "zoom" overlay that replays the child's history via the existing `_hermes/get_session_history` endpoint and then follows live events.

**Tech Stack:** Python (Hermes ACP adapter), Rust / ratatui (Kaishi TUI), ACP over stdio JSON-RPC.

**Non-goals:**
- Interacting with child sessions (sending messages, interrupting). Kaishi stays read-only on children.
- Watching multiple children simultaneously in split view. Batch mode renders multiple task lines; user zooms into one at a time.
- Auto-popping the zoom view when the child finishes. Stay until the user leaves.

---

## Context: Current State

### What already works (do not change)

- `tools/delegate_tool.py::_build_child_progress_callback` emits these event types to the parent's `tool_progress_callback`:
  - `subagent.start` — goal, task_index, task_count
  - `subagent.thinking` — reasoning text
  - `subagent.tool` — tool_name, preview, args (per tool call)
  - `subagent.progress` — batched tool names (every 5, CLI-only, will NOT bridge)
  - `subagent.complete` — final status, summary
- Each child `AIAgent` is constructed (line ~393) with `session_db=parent._session_db` and `parent_session_id=parent.session_id`, so child conversations are saved to the same SQLite store with their own `session_id`.
- `acp_adapter/server.py::ext_method` already dispatches `hermes/get_session_history` (wire name `_hermes/get_session_history`) on any session_id the SessionManager knows about.
- Kaishi `src/acp.rs` already handles `_hermes/get_session_history` responses (used for picker / resume flows).

### The blocker (line to change)

`acp_adapter/events.py:68`:

```python
if event_type != "tool.started":
    return
```

This drops every `subagent.*` event. Fix here is the whole Hermes-side bridge.

---

## Wire Protocol: `_hermes/subagent_update`

Notification (no response), sent from Hermes → Kaishi. Extension namespace per Kaishi ACP conventions (underscore prefix on the wire).

```json
{
  "jsonrpc": "2.0",
  "method": "_hermes/subagent_update",
  "params": {
    "session_id": "<parent ACP session_id>",
    "child_session_id": "<child's AIAgent.session_id>",
    "task_index": 0,
    "task_count": 3,
    "event_type": "start" | "thinking" | "tool" | "complete",
    "goal": "...",                 // on start
    "tool_name": "read_file",      // on tool
    "preview": "...",              // on tool, thinking
    "args": { ... },               // on tool (may be omitted if large)
    "status": "success" | "failed", // on complete
    "summary": "...",              // on complete (one-line result)
    "duration_seconds": 12.3        // on complete
  }
}
```

Notes:
- Discriminator is `event_type`. Adding fields later bumps no version.
- `subagent.progress` (batched) is CLI formatting, not bridged.
- `child_session_id` is stable for the child's lifetime and is the key Kaishi uses for both the task line and zoom view.

---

## Task Breakdown

### Task 1: Add `child_session_id` to the relay kwargs

**Objective:** Make the child's session_id available to the ACP bridge so it can tag every notification.

**Files:**
- Modify: `hermes-agent/tools/delegate_tool.py` (the `_build_child_progress_callback` closure and the `child` construction site ~line 420)

**Step 1:** In `_run_single_task` (around line 346), after the `child = AIAgent(...)` block, pass the child's session_id into the callback by closure. Simplest: mutate the callback to know about it.

```python
# After the child AIAgent is constructed (~line 422), expose child session_id
# to the progress callback via a setter attached to the callback.
if child_progress_cb is not None:
    setattr(child_progress_cb, "_child_session_id", getattr(child, "session_id", None))
```

**Step 2:** In `_build_child_progress_callback`, extend `_relay` so every outbound call includes `child_session_id`:

```python
def _relay(event_type: str, tool_name: str = None, preview: str = None, args=None, **kwargs):
    if not parent_cb:
        return
    try:
        parent_cb(
            event_type,
            tool_name,
            preview,
            args,
            task_index=task_index,
            task_count=task_count,
            goal=goal_label,
            child_session_id=getattr(_callback, "_child_session_id", None),
            **kwargs,
        )
    except Exception as e:
        logger.debug("Parent callback failed: %s", e)
```

**Step 3:** Verify — no test yet. Run the existing delegation test suite to ensure zero regression:

```bash
cd ~/.hermes/hermes-agent
source venv/bin/activate
pytest tests/tools/test_delegate_tool.py -v
```

Expected: all existing tests pass (we only added a kwarg).

**Step 4:** Commit.

```bash
git add tools/delegate_tool.py
git commit -m "feat(delegate): plumb child_session_id through progress callback"
```

---

### Task 2: Extend ACP event bridge to emit `_hermes/subagent_update`

**Objective:** Translate the four bridged `subagent.*` event types into `_hermes/subagent_update` notifications on the wire.

**Files:**
- Modify: `hermes-agent/acp_adapter/events.py`
- Test: `hermes-agent/tests/acp_adapter/test_events.py`

**Step 1:** Write failing test for the new branch.

```python
# tests/acp_adapter/test_events.py
def test_tool_progress_emits_subagent_update_on_start(monkeypatch):
    sent = []
    class FakeConn:
        pass
    def fake_send(conn, session_id, loop, update):
        sent.append(("tool_start", update))
    monkeypatch.setattr("acp_adapter.events._send_update", fake_send)

    notifications = []
    def fake_notify(conn, method, params):
        notifications.append((method, params))
    monkeypatch.setattr("acp_adapter.events._send_notification", fake_notify)

    cb = make_tool_progress_cb(FakeConn(), "parent-sid", None, {}, {})
    cb("subagent.start",
       preview="do the thing",
       goal="do the thing",
       child_session_id="child-sid",
       task_index=0, task_count=1)

    assert ("_hermes/subagent_update", {
        "session_id": "parent-sid",
        "child_session_id": "child-sid",
        "task_index": 0,
        "task_count": 1,
        "event_type": "start",
        "goal": "do the thing",
    }) in notifications
```

Run: `pytest tests/acp_adapter/test_events.py::test_tool_progress_emits_subagent_update_on_start -v`
Expected: FAIL — `_send_notification` does not exist yet.

**Step 2:** Add a helper `_send_notification` next to `_send_update` in `events.py`. `conn.send_notification` is `async` (confirmed in `acp/connection.py:143`), and `tool_progress_callback` fires from the sync agent thread, so mirror `_send_update`'s `run_coroutine_threadsafe` pattern exactly:

```python
def _send_notification(
    conn: acp.Client,
    loop: asyncio.AbstractEventLoop,
    method: str,
    params: dict,
) -> None:
    """Fire-and-forget a JSON-RPC notification from a worker thread."""
    try:
        future = asyncio.run_coroutine_threadsafe(
            conn.send_notification(method, params), loop
        )
        future.result(timeout=5)
    except Exception:
        logger.debug("Failed to send notification %s", method, exc_info=True)
```

Threading `loop` through to `_emit_subagent_update` means `make_tool_progress_cb` captures it in its closure (it already does — line 50).

**Step 3:** Rewrite the top of `_tool_progress` to dispatch on event family.

```python
def _tool_progress(event_type: str, name: str = None, preview: str = None,
                   args: Any = None, **kwargs) -> None:
    # --- Subagent bridge ---
    if event_type.startswith("subagent.") and event_type != "subagent.progress":
        _emit_subagent_update(conn, loop, session_id, event_type, name, preview, args, kwargs)
        return

    # --- Existing tool.started path unchanged below ---
    if event_type != "tool.started":
        return
    # ... rest of existing function stays identical
```

**Step 4:** Add `_emit_subagent_update`.

```python
def _emit_subagent_update(conn, loop, parent_session_id: str, event_type: str,
                          tool_name, preview, args, kwargs) -> None:
    short_type = event_type.split(".", 1)[1]  # "subagent.start" -> "start"
    params = {
        "session_id": parent_session_id,
        "child_session_id": kwargs.get("child_session_id"),
        "task_index": kwargs.get("task_index", 0),
        "task_count": kwargs.get("task_count", 1),
        "event_type": short_type,
    }
    if short_type == "start":
        params["goal"] = kwargs.get("goal") or preview or ""
    elif short_type == "thinking":
        params["preview"] = preview or ""
    elif short_type == "tool":
        params["tool_name"] = tool_name or ""
        if preview:
            params["preview"] = preview
        if isinstance(args, dict):
            params["args"] = args
    elif short_type == "complete":
        params["status"] = kwargs.get("status", "success")
        if preview:
            params["summary"] = preview
        if "duration_seconds" in kwargs:
            params["duration_seconds"] = kwargs["duration_seconds"]

    _send_notification(conn, loop, "_hermes/subagent_update", params)
```

**Step 5:** Run test, expect PASS.

```bash
pytest tests/acp_adapter/test_events.py::test_tool_progress_emits_subagent_update_on_start -v
```

**Step 6:** Add three more tests covering `thinking`, `tool`, `complete` event types. Same pattern. Run all, expect PASS.

**Step 7:** Commit.

```bash
git add acp_adapter/events.py tests/acp_adapter/test_events.py
git commit -m "feat(acp): bridge subagent.* events to _hermes/subagent_update"
```

---

### Task 3: Ensure `subagent.complete` carries status and duration

**Objective:** The `_build_child_progress_callback` doesn't currently emit `status` or `duration_seconds` on completion. Enrich it so the zoom view can render a proper terminal state.

**Files:**
- Modify: `hermes-agent/tools/delegate_tool.py` (the `subagent.complete` emission site — search for `"subagent.complete"`)

**Step 1:** Locate the `subagent.complete` relay site (currently around line 213 in `_build_child_progress_callback` plus the delegation-complete emissions near `_run_single_task` end, ~line 620).

**Step 2:** At the completion relay site, include `status` and `duration_seconds` in kwargs:

```python
_relay(
    "subagent.complete",
    preview=summary_line,
    status="success" if not failed else "failed",
    duration_seconds=round(time.monotonic() - start, 2),
)
```

**Step 3:** Test manually — write a small script that delegates a task and prints notifications received, or add a test asserting kwargs shape.

**Step 4:** Commit.

---

### Task 4: Kaishi — parse `_hermes/subagent_update` into app events

**Objective:** Make Kaishi's JSON-RPC dispatcher turn incoming subagent notifications into typed app events.

**Files:**
- Modify: `~/hermes-tui/src/acp.rs`
- Modify: `~/hermes-tui/src/event.rs` (add new event variant)

**Step 1:** Add the event variant in `event.rs`:

```rust
pub enum AppEvent {
    // ...existing variants...
    SubagentUpdate(SubagentUpdate),
}

#[derive(Debug, Clone)]
pub struct SubagentUpdate {
    pub child_session_id: String,
    pub task_index: usize,
    pub task_count: usize,
    pub kind: SubagentEventKind,
}

#[derive(Debug, Clone)]
pub enum SubagentEventKind {
    Start { goal: String },
    Thinking { text: String },
    Tool { name: String, preview: Option<String> },
    Complete { status: String, summary: Option<String>, duration_seconds: Option<f64> },
}
```

**Step 2:** In `acp.rs`, extend the notification branch. Currently it matches on `session_update`; add another match arm for `_hermes/subagent_update`:

```rust
// Around the existing notification dispatch
} else if method == "_hermes/subagent_update" {
    if let Some(update) = parse_subagent_update(&params) {
        let _ = event_tx.send(AppEvent::SubagentUpdate(update));
    }
}
```

**Step 3:** Write `parse_subagent_update(&Value) -> Option<SubagentUpdate>`. Discriminate on `event_type` field.

**Step 4:** Run `cargo check`, then a minimal unit test if the acp.rs module has one.

**Step 5:** Commit.

```bash
cd ~/hermes-tui
git add src/acp.rs src/event.rs
git commit -m "feat(acp): parse _hermes/subagent_update into SubagentUpdate events"
```

---

### Task 5: Kaishi — task-line widget in parent transcript

**Objective:** When `SubagentEventKind::Start` arrives, render a task line in the transcript with a status dot and goal text. Update in place on subsequent events for the same `child_session_id`.

**Files:**
- Modify: `~/hermes-tui/src/app.rs` (subagent task registry)
- Modify: `~/hermes-tui/src/ui.rs` (render path for task lines)

**Step 1:** Add to `App`:

```rust
pub struct SubagentTask {
    pub child_session_id: String,
    pub goal: String,
    pub task_index: usize,
    pub task_count: usize,
    pub status: SubagentStatus,        // Running, Done(success/fail)
    pub last_tool: Option<String>,
    pub last_preview: Option<String>,
    pub events: Vec<SubagentTranscriptEvent>, // full history for zoom view
    pub started_at: Instant,
}
```

Keep tasks in a `Vec<SubagentTask>` (or `HashMap<String, SubagentTask>`) on `App`, plus per-parent-turn ordering so they're inlined at the right transcript position.

**Step 2:** On `SubagentUpdate::Start`, push a new task and insert a transcript marker referencing its index. On subsequent events for the same `child_session_id`, append to `events` and update `status` / `last_tool`.

**Step 3:** In `ui.rs`, add a render branch for the task line. Keep it subtle — Catppuccin-aligned terminal colors, not hardcoded RGB. Example format:

```
  ⎇ [1/3] • read_file "src/acp.rs"          running
```

Pending = `Color::DarkGray` dot, Running = `Color::Yellow` dot, Success = `Color::Green`, Failed = `Color::Red`. Goal text uses `Color::Cyan`.

**Step 4:** Compile, run against a Hermes that delegates something (simplest test: spin up the agent, ask it to delegate a trivial task).

**Step 5:** Commit.

---

### Task 6: Kaishi — zoom view screen

**Objective:** On Enter over a task line (or click), replace the transcript with a full-screen view of the child session. Arrow-up or Esc returns to parent.

**Files:**
- Create: `~/hermes-tui/src/ui_subagent_zoom.rs`
- Modify: `~/hermes-tui/src/app.rs` (screen state machine)
- Modify: `~/hermes-tui/src/ui.rs` (dispatch to the zoom renderer when in zoom mode)

**Step 1:** Add a screen state:

```rust
enum Screen {
    Main,
    Picker,
    SubagentZoom { child_session_id: String },
    // ...
}
```

**Step 2:** Keybinding — when cursor is on a task line and user presses Enter (or click), transition to `SubagentZoom { child_session_id }`. Before rendering, dispatch an outbound `_hermes/get_session_history` request with the child's session_id, and store the returned history in the task's `events` buffer if not already populated (live events may already be there; merge or replace based on monotonic ordering).

**Step 3:** Render the zoom screen. Reuse the main transcript renderer where possible — the child's events are structurally similar to normal session events (thinking, tool calls, results). Header bar at the top:

```
  ← Subagent [1/3] • do the thing                        running
```

Footer hint:

```
  ↑ back to parent    ↓/mouse to scroll
```

**Step 4:** Keybindings inside zoom:
- `ArrowUp` at top-of-buffer, or `Esc` always → back to `Screen::Main`.
- Normal scroll keys behave as they do in the main transcript.
- Do NOT accept text input. No send.

**Step 5:** Complete state: when a `SubagentEventKind::Complete` arrives for the zoomed child, update the header bar color/label but do not pop. User stays.

**Step 6:** Test manually with a delegation that takes 20–30 seconds so there's time to zoom in and watch.

**Step 7:** Commit.

---

### Task 7: Kaishi — batch mode UX

**Objective:** When `task_count > 1`, render one task line per child, zoomable independently. Other children continue running invisibly while one is zoomed.

**Files:**
- Modify: `~/hermes-tui/src/app.rs`, `~/hermes-tui/src/ui.rs`

**Step 1:** Verify the data model already supports this — each `SubagentUpdate::Start` creates a new task keyed by `child_session_id`, so N tasks naturally produce N lines. Ordering in the transcript follows `task_index`.

**Step 2:** Render all three task lines as a group with a shared header:

```
  ⎇ Parallel subagents (3)
    [1/3] • read_file                         running
    [2/3] • web_search                        done ✓
    [3/3] • (pending…)                        pending
```

**Step 3:** Make each line independently selectable. Cursor navigation moves through them; Enter zooms into the highlighted one.

**Step 4:** Commit.

---

### Task 8: Handle child grandchildren gracefully

**Objective:** Subagents currently cannot spawn grandchildren (`delegate_tool.py` sets `_delegate_depth` and rejects depth > 1), but verify no `subagent.*` event from a grandchild ever reaches the ACP bridge with an unknown `child_session_id`.

**Files:** Read-only check.

**Step 1:** Confirm `_delegate_depth` enforcement in `delegate_tool.py` still prevents recursion. Re-read the relevant block (~line 425 and wherever depth is validated at delegation entry).

**Step 2:** If enforcement is solid, no code change. Add a comment in the bridge noting the assumption.

**Step 3:** If one day grandchildren become allowed, the bridge gracefully degrades: unknown `child_session_id` on Kaishi side creates a new task line rather than dropping. Add a one-line comment in `app.rs` affirming this.

**Step 4:** Commit (if anything changed).

---

### Task 9: Integration test — end-to-end

**Objective:** Live verification that a delegated task is visible, zoomable, and completes cleanly.

**Steps:**
1. Start Kaishi.
2. Ask Hermes to delegate a task that takes ~30 seconds (e.g. "spawn a subagent to web_search for X and summarize").
3. Observe a task line appears with ⎇ prefix and running status.
4. Press Enter — zoom view opens, shows thinking and tool events live.
5. Press ArrowUp — back to parent transcript. Parent is still waiting.
6. When subagent completes, task line turns green with ✓.
7. Press Enter again — zoom view shows the full child transcript including the final answer.
8. Repeat with a batch of 3 tasks to confirm batch UX.

**Acceptance:**
- No lag between child event and task-line update (< 200 ms).
- Zoom view history replay feels instant on enter (< 500 ms).
- No crashes, no orphan task lines, no duplicate entries.

---

## Risks and Mitigations

**Risk: `send_notification` method doesn't exist on the `acp.Client` object.**
Mitigation: Check `acp_adapter/server.py` for how notifications are sent elsewhere. If only `session/update` is sent via a special path, the helper may need to use the same underlying transport — `_send_update` will be the template.

**Risk: The child's session_id isn't populated yet when `subagent.start` fires.**
Mitigation: Confirm in Task 1 that `child.session_id` is set at construction time (AIAgent assigns it in `__init__`). If not, defer the `setattr` until after first agent call.

**Risk: Large `args` payloads bloat notifications.**
Mitigation: In `_emit_subagent_update`, cap `args` serialization at ~2 KB; if larger, include `preview` only. Match the existing ACP `ToolCallStart` pattern.

**Risk: Zoom view feels cold because history replay is only text.**
Mitigation: `_hermes/get_session_history` already returns structured content blocks (confirmed via the picker/resume flow). Reuse the same renderer.

---

## Verification Before Completion

- [x] `pytest tests/acp/test_events.py -v` green (21 passed)
- [x] `pytest tests/tools/test_delegate.py -v` green (69 passed, +2 new regression tests)
- [x] `cargo check` on hermes-tui clean
- [x] `cargo clippy -- -D warnings` on hermes-tui clean
- [x] All 31 hermes-tui tests pass
- [ ] Manual end-to-end flow from Task 9 (Eva's responsibility — pending)
- [ ] Batch mode (3 parallel tasks) zooms independently (pending Task 9)

---

## Shipped Commits (2026-04-19)

Implementation completed via `subagent-driven-development` skill — fresh subagent per task with spec compliance review between Hermes-side tasks.

| Task | Repo | Commit | What |
|------|------|--------|------|
| 1 | hermes-agent | `5a1d2d0` | feat(delegate): plumb child_session_id through progress callback |
| 2 | hermes-agent | `1fb50c3` | feat(acp): bridge subagent.* events to `_hermes/subagent_update` |
| 3 | hermes-agent | `27e5f32` | feat(delegate): include status and duration_seconds in subagent.complete events (no-op for code, +2 regression tests) |
| 4 | hermes-tui | `7843142` | feat(acp): parse `_hermes/subagent_update` into SubagentUpdate events |
| 5 | hermes-tui | `4188955` | feat(ui): render subagent task line in transcript with live status |
| 6 | hermes-tui | `3060f1c` | feat(ui): subagent zoom view — Ctrl+Z zooms into child session, ↑/Esc returns |
| 7 | hermes-tui | `dff8b2f` | feat(ui): batch subagent cycling — Ctrl+Z in zoom cycles through siblings |
| 8 | hermes-agent | `bb1393d` | docs(acp): note grandchild assumption in subagent bridge |

### Plan-vs-reality notes

- **Task 1** ended up using a `_state` mutable dict instead of `setattr` on the callback to handle Python's forward-reference scoping cleanly. Better than what the plan proposed.
- **Task 3** turned out to be a no-op: the `subagent.complete` emission sites in `delegate_tool.py` already carried `status` + `duration_seconds`. The implementer wisely added regression tests instead of unnecessary code changes.
- **Task 5** required some manual cleanup after the implementer hit its iteration cap — three small fixes (a missing `Role::Subagent` arm in the `/save` exporter match, an unused import, and using `ind` instead of hardcoded `"  "` for indent so narrow viewports work). Worth noting: large UI tasks with many cross-file touches benefit from a higher `max_iterations` for the subagent.
- **Task 6** chose `Ctrl+Z` (Zoom mnemonic) for the zoom keybinding after confirming no conflict with existing chat bindings.
- **Task 7** scoped down to just the cycling feature (Ctrl+Z while in zoom cycles to next subagent). Sibling status header and transcript grouping were skipped as risky relative to value — the existing `[1/3]` prefix was already informative enough.

### Known follow-ups for future work

- **History replay on join.** The plan envisioned calling `_hermes/get_session_history` when entering the zoom view, in case Kaishi joined mid-stream (e.g. on reconnect). Skipped for the initial ship — the zoom currently renders only what's been buffered live in `SubagentTask.events`. Marked with `TODO(task-9)` in `src/ui_subagent_zoom.rs`. Not blocking.
- **Cursor-on-task-line zoom.** Ctrl+Z always picks the most recent Role::Subagent in transcript order. A future iteration could let the user move a cursor to any task line and zoom into that one. Pleasant-to-have, not needed yet.
- **Grandchild support.** If `MAX_DEPTH` is ever raised in `delegate_tool.py`, the bridge degrades gracefully (unknown `child_session_id` creates a new task line) but the zoom view would need breadcrumbs/nesting. See comment in `acp_adapter/events.py` above `_emit_subagent_update`.
