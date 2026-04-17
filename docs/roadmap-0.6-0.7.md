# Kaishi Roadmap: v0.6.0 – v0.7.0

Feature parity targets against Claude Code, OpenCode, and Hermes CLI.

## v0.6.0 — Polish & Visual Leap

### Quick Wins (trivial–small)

1. **Terminal bell on turn completion**
   Add `\x07` in `handle_prompt_done` so backgrounded terminals get notified.

2. **Ctrl+L clear screen**
   Clear messages + line cache + reset scroll. Standard terminal shortcut.

3. **Word-level cursor movement**
   Alt+Left / Alt+Right to jump by word boundaries. Complements existing
   Ctrl+W (delete word back) and Ctrl+K (kill to EOL).

4. **Tab completion for /commands**
   When input starts with `/`, Tab cycles through known commands (local +
   server). Static list, no popup needed — bash-style cycle-through.

5. **Link rendering in inline markdown**
   Parse `[text](url)` in `parse_inline_spans`. Render text as underlined
   (discard URL — can't click in a terminal). Currently passes through raw.

6. **/compact feedback**
   After forwarding `/compact` to the server, show a system message with
   the result — confirmation that context was compressed, new token count
   if available from the ACP response.

### Main Feature

7. **Syntax highlighting in code blocks**
   Replace monochrome `palette::CODE_FG` with language-aware highlighting.
   The `code_lang` variable is already parsed from fences but unused.

   Options:
   - `syntect` — mature, Sublime Text syntax definitions, 200+ languages.
     Heavy dependency (~2MB binary size increase) but excellent coverage.
   - `tree-sitter-highlight` — faster, incremental, but needs per-language
     grammars compiled in. More work to set up.
   - Minimal DIY — keyword-only highlighting for top 5-10 languages
     (Python, Rust, JS, bash, JSON, YAML, SQL). Smallest footprint.

   Recommendation: `syntect` with a bundled Catppuccin-compatible theme,
   falling back to green monochrome for unknown languages.

---

## v0.7.0 — Power User Features

### Medium Effort

8. **Context window health indicator**
   Show context fill level in the status bar (e.g., `ctx 42%` or a colored
   bar ●●●●○○○○). Requires ACP to expose context usage — check whether
   `PromptDone`, `session/update`, or `_hermes/get_session_info` carries
   this data. If not available, request upstream.

9. **Markdown table rendering**
   Detect pipe-delimited tables and render with aligned columns. Tables
   currently pass through as raw text. Even basic column alignment would
   be a big readability win. Consider `comfy-table` crate or hand-rolled
   column width calculation.

10. **Session export (/save)**
    Dump current conversation as markdown to a file. Iterate `self.messages`,
    format by role, write to `~/kaishi-export-{timestamp}.md` or a
    user-specified path. Hermes CLI has this as `/save`.

11. **Effort / reasoning control**
    `/effort low|medium|high` or `/reasoning` level selector. Store as a
    session-local setting and pass as param on subsequent `session/prompt`
    calls. Depends on ACP supporting effort params — verify wire format.

12. **Session deletion from picker**
    `d` or `Delete` key on a session in the picker to delete it. Needs an
    ACP method (`session/delete` or `_hermes/delete_session`). Confirm
    with a mini-modal before deleting.

### Larger Features (scope carefully)

13. **Search within conversation (Ctrl+F)**
    Search overlay: input bar at top/bottom, highlight matches in messages,
    `n`/`N` to jump between matches. Needs a search state struct, match
    index tracking, and scroll-to-match logic.

14. **Image paste / attach**
    Ctrl+V or `/image <path>` to include images in prompts. ACP supports
    content blocks with image data. Complexity: terminal image paste varies
    by emulator (iTerm2 OSC 1337, kitty protocol, etc.). File path is the
    safer starting point.

15. **File path autocomplete (`@` prefix)**
    Claude Code's signature feature. Type `@` to trigger filesystem
    autocomplete with a popup/dropdown. Needs async directory scanning,
    a filtered popup widget, and Enter/Tab to confirm selection.
    Significant UI work — consider as a standalone mini-project.

### Nice-to-Have / Deferred

16. **Vim keybindings toggle**
    Normal/Insert mode state machine for input. Config flag to enable.
    Niche audience but vocal. Could use `tui-input` crate or hand-roll.

17. **Theme / skin support**
    `--theme` flag or config option for different color palettes. Current
    terminal-native approach works well with Catppuccin remapping, but
    explicit light-terminal support would help others.

18. **Notification badges on session picker**
    Show which sessions have unread/new activity. Relevant if background
    tasks or multi-session workflows are supported later.

---

## Current State (v0.5.0)

For reference, what's already shipped:

- Markdown rendering (headings, bold, italic, inline code, fenced code
  blocks with box-drawing, bullets, numbered lists, blockquotes, HR)
- Animated spinner with shimmer, stall detection, phase word bank
- Tool call smart summaries (20+ tools recognized)
- Turn completion divider with per-turn token deltas + elapsed time
- Thinking collapse/expand (Ctrl+O)
- Status bar: model, active tool, cumulative tokens, CWD
- Approval modal (Enter/Esc/j/k)
- Session picker with title, timestamps, source badges, message counts
- Lazy history pagination (scroll-up to load more)
- Slash command passthrough to ACP server
- Local commands: /quit /clear /new /verbose /usage /help /title /reset
- Input: multiline (Ctrl+J, Shift+Enter), cursor movement (Ctrl+A/E/W/K),
  input history (Up/Down), placeholder text
- Mouse scroll, keyboard scroll (PgUp/PgDn, Ctrl+U)
- ACP reconnect on crash (Disconnected screen → Enter to respawn)
- CLI args: --profile, --cwd, --session, --help
- Word-level wrapping with continuation indent
- 4078 LOC across 7 source files
