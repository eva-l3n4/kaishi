# Tool Display & Line Wrap Fixes

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Fix three rendering issues — first-line wrap misalignment under assistant indicator, swallowed tool call diffs/content, and raw JSON showing for unrecognized tools.

**Architecture:** Targeted patches in `ui.rs` (wrap indent + streaming icon), `acp.rs` (content extraction), and `app.rs` (tool input summarization + diff detection).

**Files:**
- `src/ui.rs` — pre_wrap_lines indent, icon prepend (cached + streaming paths)
- `src/acp.rs` — tool_call_update content extraction
- `src/app.rs` — looks_like_diff heuristic, summarize_tool_input fallback

---

## Task 1: Fix first-line continuation indent for assistant messages

**Objective:** Wrapped assistant text should align with the first letter after `◆`, not under the diamond itself.

**Files:**
- Modify: `src/ui.rs:517-660` (pre_wrap_lines)
- Modify: `src/ui.rs:825-856` (Role::Assistant icon prepend)

**Problem:** `pre_wrap_lines` computes `indent_width` from leading whitespace only. The icon prefix `  ◆ ` has 2 whitespace chars + 2 non-whitespace chars (◆ + space). Continuation lines indent to column 2 instead of column 4.

**Fix:** After the icon is prepended in `render_message`, the first line's visual prefix is `  ◆ ` (4 columns). `pre_wrap_lines` should detect that the first span is the icon prefix and use its full display width for continuation indent — or, more simply, the `Role::Assistant` handler should set the indent explicitly.

**Approach — explicit indent hint via leading whitespace:**

Rather than modifying `pre_wrap_lines` to understand icons, ensure the icon prefix width matches the indent that `pre_wrap_lines` will detect. The icon `  ◆ ` is 4 visual columns. But `pre_wrap_lines` only counts leading whitespace — it sees 2 spaces then stops.

**Simplest fix:** In the `Role::Assistant` handler, after building `new_spans`, set the first span to `"    "` (4 spaces) with icon styling is wrong — we need the icon visible.

**Better fix:** Modify `pre_wrap_lines` to measure the full visual width of the first span(s) up to and including the first non-whitespace content, not just whitespace. This way `  ◆ ` (2 space + diamond + space) correctly measures as 4 columns for continuation.

In `pre_wrap_lines`, replace the `indent_width` calculation:

```rust
// Current: only counts whitespace
let indent_width = {
    let mut w = 0usize;
    for span in &line.spans {
        let mut all_ws = true;
        for ch in span.content.chars() {
            if ch.is_whitespace() {
                w += UnicodeWidthChar::width(ch).unwrap_or(0);
            } else {
                all_ws = false;
                break;
            }
        }
        if !all_ws { break; }
    }
    w
};

// New: count through the first non-whitespace "prefix" span(s)
// This handles icon prefixes like "  ◆ " where the icon is part of the indent
let indent_width = {
    let mut w = 0usize;
    let mut found_content = false;
    for span in &line.spans {
        for ch in span.content.chars() {
            if ch.is_whitespace() {
                w += UnicodeWidthChar::width(ch).unwrap_or(0);
                if found_content {
                    // Whitespace after non-ws = end of prefix
                    // (e.g., the space after ◆)
                    // Include this trailing space, then stop
                    found_content = false; // reset — we keep going
                }
            } else if !found_content {
                // First non-ws char (the icon glyph)
                w += UnicodeWidthChar::width(ch).unwrap_or(1);
                found_content = true;
            } else {
                // Second non-ws char = actual content, stop
                break;
            }
        }
        // If we hit actual content (second non-ws char), break outer too
        // We need a way to signal this — use a flag
    }
    w
};
```

Actually, this is getting fiddly. Cleaner approach — measure the full width of the **first span** when it looks like a role prefix (contains a non-ASCII icon char):

```rust
let indent_width = {
    let first_span = line.spans.first().map(|s| s.content.as_ref()).unwrap_or("");
    let has_icon = first_span.chars().any(|c| !c.is_ascii() && !c.is_whitespace());
    if has_icon {
        // Icon prefix span — use its full visual width as indent
        unicode_width::UnicodeWidthStr::width(first_span)
    } else {
        // Normal: measure leading whitespace only
        let mut w = 0usize;
        for span in &line.spans {
            let mut all_ws = true;
            for ch in span.content.chars() {
                if ch.is_whitespace() {
                    w += UnicodeWidthChar::width(ch).unwrap_or(0);
                } else {
                    all_ws = false;
                    break;
                }
            }
            if !all_ws { break; }
        }
        w
    }
};
```

**Step 1:** Apply the indent_width fix in `pre_wrap_lines` (ui.rs ~530-545).

**Step 2:** Fix the streaming path (ui.rs ~428-434). Currently it does NOT strip the original indent — it prepends `  ◆ ` and then extends ALL original spans including the leading whitespace span. This creates `  ◆     text` (double indent). Apply the same stripping logic as the cached path:

```rust
// Streaming path — match the cached-message logic
if all_lines.len() > before {
    let first = &mut all_lines[before];
    let mut new_spans = vec![Span::styled(
        "  ◆ ",
        Style::default().fg(palette::ACCENT_ASSISTANT),
    )];
    // Skip leading whitespace spans (same as Role::Assistant handler)
    for span in first.spans.iter() {
        let trimmed = span.content.trim_start();
        if trimmed.is_empty() { continue; }
        if span.content.len() != trimmed.len() {
            new_spans.push(Span::styled(trimmed.to_string(), span.style));
        } else {
            new_spans.push(span.clone());
        }
    }
    *first = Line::from(new_spans);
}
```

**Step 3:** Build and visually verify: `cargo build --release`

**Step 4:** Commit:
```bash
git add src/ui.rs
git commit -m "fix: align wrapped lines with first letter after assistant indicator"
```

---

## Task 2: Fix tool_call_update content extraction in ACP client

**Objective:** Ensure diff content and other tool outputs actually reach `handle_tool_update`.

**Files:**
- Modify: `src/acp.rs:332-354` (tool_call_update parsing)

**Problem:** Content extraction uses two JSON pointers:
1. `/content/0/content/text` — for `ContentToolCallContent` wrapping `TextContentBlock`
2. `/content/0/text` — for flat `TextContentBlock`

If the ACP server sends content in a different shape (e.g., raw string, array of strings, or a different nesting), content arrives as `None` and diffs are silently dropped.

**Fix:** Add broader fallback extraction and debug logging:

```rust
"tool_call_update" => {
    let id = params
        .get("toolCallId")
        .or_else(|| params.get("tool_call_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let status = params
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Try multiple content extraction paths
    let content = params
        .pointer("/content/0/content/text")           // ContentToolCallContent > TextContentBlock
        .or_else(|| params.pointer("/content/0/text")) // Flat TextContentBlock
        .or_else(|| params.pointer("/content/text"))   // Direct text field
        .or_else(|| params.get("content").filter(|v| v.is_string())) // Raw string
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // If content is an array of objects, try to join their text fields
    let content = content.or_else(|| {
        params.get("content")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item.pointer("/content/text")
                            .or_else(|| item.get("text"))
                            .and_then(|t| t.as_str())
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|s| !s.is_empty())
    });

    let _ = event_tx.send(AppEvent::ToolCallUpdate {
        id,
        status,
        content,
    });
}
```

**Step 1:** Add debug logging to see actual payloads. Add to `acp.rs` near the tool_call_update handler:

```rust
#[cfg(debug_assertions)]
if let Ok(debug_json) = serde_json::to_string_pretty(&params) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open("/tmp/kaishi-tool-updates.jsonl")
    {
        use std::io::Write;
        let _ = writeln!(f, "{}", debug_json);
    }
}
```

**Step 2:** Apply the broader content extraction chain above.

**Step 3:** Build, test with a real tool call that produces a diff (e.g., `patch` tool), verify content shows up.

**Step 4:** Commit:
```bash
git add src/acp.rs
git commit -m "fix: broaden tool_call_update content extraction for diffs"
```

---

## Task 3: Improve diff detection heuristic

**Objective:** Detect more diff formats so tool results with patch output get colored rendering.

**Files:**
- Modify: `src/app.rs:1558-1564` (looks_like_diff)

**Problem:** Current heuristic requires the first line to start with `--- ` or `diff --git`, OR all three markers (`\n+`, `\n-`, `\n@@`) present. This misses:
- Diffs that start with context lines before the `---` header
- Short diffs without `@@ ` markers
- Hermes `patch` tool output that starts with `*** Begin Patch` or just shows `+`/`-` lines

**Fix:**

```rust
fn looks_like_diff(text: &str) -> bool {
    let first = text.lines().next().unwrap_or("");

    // Explicit diff headers
    if first.starts_with("--- ")
        || first.starts_with("diff --git")
        || first.starts_with("*** Begin Patch")
    {
        return true;
    }

    // Heuristic: has unified diff markers
    if text.contains("\n@@ ") && (text.contains("\n+") || text.contains("\n-")) {
        return true;
    }

    // Heuristic: has enough +/- lines to look like a diff (at least 2 of each)
    let plus_lines = text.lines().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
    let minus_lines = text.lines().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
    if plus_lines >= 2 && minus_lines >= 1 {
        return true;
    }
    if minus_lines >= 2 && plus_lines >= 1 {
        return true;
    }

    false
}
```

**Step 1:** Apply the updated heuristic.

**Step 2:** Build and verify.

**Step 3:** Commit:
```bash
git add src/app.rs
git commit -m "fix: broaden diff detection for tool result rendering"
```

---

## Task 4: Better fallback for unrecognized tool inputs

**Objective:** Stop showing raw JSON blobs for tools that don't have explicit summarizers.

**Files:**
- Modify: `src/app.rs:1898-1912` (generic fallback in summarize_tool_input)

**Problem:** The generic fallback tries to find a short string value, then falls back to compact JSON truncated to 60 chars. For tools with complex object params or long string values, this produces unreadable `{"mode":"replace","path":"/home/...` snippets.

**Fix:** Improve the generic fallback to be smarter about extracting meaningful info:

```rust
_ => {
    if let Some(obj) = json.as_object() {
        // Priority 1: look for common meaningful keys
        let meaningful_keys = ["path", "name", "query", "url", "command", "goal", "file", "ref"];
        for key in meaningful_keys {
            if let Some(s) = obj.get(key).and_then(|v| v.as_str()) {
                if !s.is_empty() && s.len() < 120 {
                    return Some(truncate_summary(s, 80));
                }
            }
        }

        // Priority 2: first short string value (existing logic)
        for (_k, v) in obj.iter() {
            if let Some(s) = v.as_str() {
                if !s.is_empty() && s.len() < 100 {
                    return Some(truncate_summary(s, 60));
                }
            }
        }

        // Priority 3: show key names only (not values)
        let keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
        if !keys.is_empty() {
            return Some(truncate_summary(&keys.join(", "), 60));
        }
    }

    // Final fallback: compact JSON preview
    let compact = trimmed.replace('\n', " ");
    truncate_summary(&compact, 60)
}
```

Also add a few missing tool handlers:

```rust
"todo" => {
    let todos = json.get("todos").and_then(|v| v.as_array());
    match todos {
        Some(t) => format!("{} items", t.len()),
        None => "read list".to_string(),
    }
}
"session_search" => {
    let q = json.get("query").and_then(|v| v.as_str());
    match q {
        Some(q) => truncate_summary(q, 60),
        None => "recent sessions".to_string(),
    }
}
"execute_code" => {
    let code = json.get("code").and_then(|v| v.as_str()).unwrap_or("");
    let first_line = code.lines().next().unwrap_or("script");
    let line_count = code.lines().count();
    format!("{} ({} lines)", truncate_summary(first_line, 40), line_count)
}
```

**Step 1:** Add the new explicit tool handlers above the `_ =>` catch-all.

**Step 2:** Replace the generic fallback with the improved version.

**Step 3:** Build and verify.

**Step 4:** Commit:
```bash
git add src/app.rs
git commit -m "fix: better tool input summaries, smarter generic fallback"
```

---

## Task 5: Final integration check

**Objective:** Verify all three fixes work together in a real session.

**Steps:**
1. `cargo build --release`
2. Launch Kaishi, start a new session
3. Send a long message that wraps → verify continuation lines align with first letter, not diamond
4. Ask the agent to edit a file → verify diff shows in tool result with colors
5. Trigger an unusual tool (todo, execute_code) → verify summary instead of JSON blob
6. `cargo clippy -- -W clippy::all` — clean
7. Final commit:
```bash
git add -A
git commit -m "chore: clippy clean after tool display fixes"
```

---

## Summary

| Task | File | Issue |
|------|------|-------|
| 1 | ui.rs | Wrapped lines align under ◆ instead of after it |
| 2 | acp.rs | Tool update content extraction too narrow |
| 3 | app.rs | Diff detection misses common formats |
| 4 | app.rs | Raw JSON for unrecognized tools |
| 5 | — | Integration verification |
