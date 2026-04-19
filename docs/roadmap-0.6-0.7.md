# Kaishi Roadmap: v0.6.0 – v0.9.0+

Feature parity targets against Claude Code, OpenCode, and Hermes CLI.

## v0.6.0 — Polish & Visual Leap ✓

All shipped. See git tag `v0.6.0`.

1. ✓ Terminal bell on turn completion
2. ✓ Ctrl+L clear screen
3. ✓ Word-level cursor movement (Alt+Left/Right)
4. ✓ Tab completion for /commands
5. ✓ Link rendering in inline markdown
6. ✓ /compact feedback
7. ✓ Syntax highlighting in code blocks (syntect, base16-eighties.dark)

---

## v0.7.0 — Power User Features ✓

### Quick Wins ✓

8. ✓ **`!` shell escape prefix**
9. ✓ **Permission mode indicator + toggle** (Shift+Tab for YOLO)
10. ✓ **Thinking toggle (Ctrl+O)**
11. ✓ **`/compact` with focus hint**

### Medium Effort ✓

12. ✓ **Context window health indicator** (status bar fill bar)
13. ✓ **Diff view for file edits** (colored unified diff inline)
14. ✓ **Session export (/save)**
15. ✓ **Markdown table rendering** (aligned columns)
16. ✓ **External editor for input (Ctrl+G)**
17. ✓ **Ctrl+R reverse history search**

### Larger Features ✓

18. ✓ **Effort / reasoning control** (/effort + slider overlay)
22. ✓ **File path autocomplete (`@` prefix)** (async walkdir scan + popup)

---

## v0.8.0 — Feature Parity Sprint ✓

All shipped 2026-04-18.

- ✓ **Command palette (Ctrl+P)** — fuzzy search across all actions
- ✓ **Undo/rewind (Esc Esc)** — revert last user+assistant turn
- ✓ **Effort slider overlay** — interactive 3-position (low/med/high)
- ✓ **Context window health indicator** — 10-block fill bar, color-coded
- ✓ **Reverse history search (Ctrl+R)** — bash-style incremental
- ✓ **@ file mentions** — async scanning, fuzzy autocomplete popup

Build stats: 5,331 LOC across 11 source files, 4.0MB release binary.

---

## v0.8.2 — Navigation ✓

Shipped 2026-04-19.

- ✓ **Return to session picker (Ctrl+B)** — one-way state reset,
  background session list refresh, `/sessions` command (`/switch`
  alias), "Switch session" palette entry.

---

## v0.8.3 — Scroll fidelity ✓

Shipped 2026-04-19.

- ✓ **Picker opens at top**, not bottom — fixed inverted scroll math
  that had the oldest session pinned to the viewport bottom with
  `+ New Session` off-screen above.
- ✓ **Picker keyboard scroll-follow** — `j`/`k`/Up/Down past the
  viewport edge now scrolls the list so the selected card stays
  visible. `CARD_HEIGHT` extracted as a shared constant.
- ✓ **Palette scroll-follow** — `Ctrl+P` list uses `render_stateful_widget`
  with `ListState`, auto-scrolls to keep the selected entry on-screen.
- ✓ **File popup scroll-follow** — `@` autocomplete drops the
  hardcoded `take(8)` cap, renders via `ListState`, scrolls past
  the visible window.

---

## v0.8.4 — Scroll direction ✓

Shipped 2026-04-19.

- ✓ **Picker mouse wheel no longer inverted** — wheel-up goes up
  the list, wheel-down goes down. The handler was tuned for the
  old inverted scroll math fixed in v0.8.3; flipped the signs to
  match offset-0-means-top semantics.

---

## v0.9.0+ — Aspirational

19. **Session deletion from picker**
    `d` or `Delete` key on a session in the picker to delete it. Needs
    ACP method (`session/delete` or `_hermes/delete_session`). Confirm
    with a mini-modal before deleting.

20. **Search within conversation (Ctrl+F)**
    Search overlay: input bar, highlight matches in messages, `n`/`N` to
    jump between. Needs search state struct, match index tracking, and
    scroll-to-match logic.

21. **Image paste / attach**
    Ctrl+V or `/image <path>` to include images in prompts. ACP supports
    content blocks with image data.

23. **Agent team display**
    When Hermes spawns subagents, show nested agent activity inline —
    expandable/collapsible per-agent sections with their own tool call
    summaries.

24. **Side questions (`/btw`)**
    Ask something without it counting toward context cost. Needs ACP
    support for ephemeral prompts.

25. **`#` quick memory**
    Type `# Always use 2-space indent` to save to project/session
    memory. Needs local notes file or Hermes memory integration.

26. **Vim keybindings toggle**
    Normal/Insert mode state machine for input. Config flag to enable.

27. **Theme / skin support**
    `--theme` flag or config for different color palettes.

28. **Notification badges on session picker**
    Show which sessions have unread/new activity.

---

## Current State (v0.8.0)

- Markdown rendering (headings, bold, italic, inline code, fenced code
  blocks with box-drawing, bullets, numbered lists, blockquotes, HR,
  tables with aligned columns)
- Syntax highlighting in code blocks (syntect, 200+ languages)
- Animated spinner with shimmer, stall detection, phase word bank
- Tool call smart summaries (20+ tools recognized) + diff coloring
- Turn completion divider with per-turn token deltas + elapsed time
- Thinking collapse/expand (Ctrl+O)
- Command palette (Ctrl+P) with fuzzy search
- Undo/rewind last turn (Esc Esc)
- Effort slider (/effort, also via palette)
- Context window health indicator in status bar
- Reverse history search (Ctrl+R)
- @ file mentions with async scanning
- Status bar: model, effort, context fill, active tool, tokens, CWD
- Approval modal (Enter/Esc/j/k), YOLO toggle (Shift+Tab)
- Session picker with title, timestamps, source badges, message counts
- Lazy history pagination (scroll-up to load more)
- Slash command passthrough to ACP server
- Tab completion for /commands (bash-style cycle-through)
- Local commands: /quit /clear /new /verbose /usage /help /title /reset
  /save /effort
- Shell escape (!command)
- External editor (Ctrl+G)
- Input: multiline (Ctrl+J, Shift+Enter), cursor movement
  (Ctrl+A/E/W/K, Alt+Left/Right), input history (Up/Down), placeholder
- Terminal bell on turn completion
- Ctrl+L clear screen
- Mouse scroll, keyboard scroll (PgUp/PgDn, Ctrl+U)
- ACP reconnect on crash (Disconnected screen → Enter to respawn)
- CLI args: --profile, --cwd, --session, --help
- Word-level wrapping with continuation indent
- 5331 LOC across 11 source files
