# Kaishi Portfolio Site — Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Build a single-page portfolio site for Kaishi (懐紙) with the Gentle Dusk blossom theme, deployed on Netlify from the `site/` directory.

**Architecture:** One static `index.html` with embedded CSS. No build step, no JS framework. CSS-only falling petal animation in hero. Full-bleed sections with alternating dark backgrounds.

**Tech Stack:** HTML5, CSS3, Google Fonts (Noto Serif JP, Inter, JetBrains Mono)

**Spec:** `docs/specs/2026-04-18-portfolio-site-design.md`

---

### Task 1: Scaffold site directory and base HTML

**Objective:** Create `site/index.html` with the document skeleton, font imports, and CSS custom properties.

**Files:**
- Create: `site/index.html`

**Step 1: Create the base file**

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>懐紙 Kaishi — Terminal UI for Hermes Agent</title>
  <meta name="description" content="A beautiful terminal UI for conversing with Hermes Agent. Built in Rust with ratatui.">
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;600&family=Noto+Serif+JP:wght@400;700&family=JetBrains+Mono:wght@400&display=swap" rel="stylesheet">
  <style>
    :root {
      --dusk-deep: #2d1b33;
      --dusk-mid: #44274a;
      --dusk-warm: #6b3a5e;
      --blossom: #d4889c;
      --petal: #ffb7c5;
      --cream: #fff0f3;
      --muted: #b8a0b0;
      --surface: #1e1228;
      --surface-alt: #241530;
      --code-bg: #0d0a12;
    }

    *, *::before, *::after { margin: 0; padding: 0; box-sizing: border-box; }

    body {
      font-family: 'Inter', system-ui, sans-serif;
      color: var(--cream);
      background: var(--dusk-deep);
      -webkit-font-smoothing: antialiased;
      -moz-osx-font-smoothing: grayscale;
    }

    .section-inner {
      max-width: 800px;
      margin: 0 auto;
      padding: 0 24px;
    }
  </style>
</head>
<body>
  <!-- Sections will be added in subsequent tasks -->
</body>
</html>
```

**Step 2: Verify**

Open `site/index.html` in a browser — should show an empty dark page with no errors in console.

**Step 3: Commit**

```bash
git add site/index.html
git commit -m "feat(site): scaffold base HTML with fonts and CSS variables"
```

---

### Task 2: Hero section with gradient and typography

**Objective:** Build the full-viewport hero with the dusk gradient, 懐紙 kanji, KAISHI text, and tagline.

**Files:**
- Modify: `site/index.html`

**Step 1: Add hero CSS**

Add to the `<style>` block:

```css
.hero {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-direction: column;
  background: linear-gradient(180deg, var(--dusk-deep) 0%, var(--dusk-mid) 40%, var(--dusk-warm) 70%, var(--blossom) 100%);
  position: relative;
  overflow: hidden;
  text-align: center;
}

.hero-kanji {
  font-family: 'Noto Serif JP', serif;
  font-size: clamp(48px, 8vw, 72px);
  font-weight: 700;
  color: var(--cream);
  line-height: 1.2;
}

.hero-name {
  font-family: 'Inter', sans-serif;
  font-size: clamp(16px, 2.5vw, 22px);
  font-weight: 300;
  letter-spacing: 6px;
  color: var(--cream);
  margin-top: 4px;
}

.hero-tagline {
  font-family: 'Inter', sans-serif;
  font-size: clamp(13px, 1.8vw, 16px);
  font-weight: 400;
  color: var(--blossom);
  margin-top: 16px;
  letter-spacing: 1px;
}
```

**Step 2: Add hero HTML**

Replace the body comment with:

```html
<section class="hero">
  <div class="hero-kanji">懐紙</div>
  <div class="hero-name">KAISHI</div>
  <div class="hero-tagline">A terminal for cherry blossom season</div>
</section>
```

**Step 3: Verify**

Open in browser — should show a full-viewport dusk gradient with centered 懐紙, KAISHI, and tagline. The kanji should render in Noto Serif JP (may need a moment to load the font).

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add hero section with dusk gradient and typography"
```

---

### Task 3: Falling petal animation (hero only)

**Objective:** Add CSS-only falling cherry blossom petals to the hero section.

**Files:**
- Modify: `site/index.html`

**Step 1: Add petal CSS**

Add to the `<style>` block:

```css
.petal {
  position: absolute;
  top: -20px;
  width: 8px;
  height: 8px;
  background: var(--petal);
  border-radius: 50% 0 50% 50%;
  opacity: 0.3;
  pointer-events: none;
  animation: fall linear infinite;
}

@keyframes fall {
  0% {
    transform: translateY(-20px) rotate(0deg) translateX(0px);
    opacity: 0;
  }
  10% {
    opacity: 0.3;
  }
  90% {
    opacity: 0.3;
  }
  100% {
    transform: translateY(100vh) rotate(360deg) translateX(40px);
    opacity: 0;
  }
}

/* Each petal gets unique timing, position, and size */
.petal:nth-child(1) { left: 10%; animation-duration: 12s; animation-delay: 0s; width: 6px; height: 6px; opacity: 0.2; }
.petal:nth-child(2) { left: 25%; animation-duration: 15s; animation-delay: 2s; width: 8px; height: 8px; opacity: 0.35; }
.petal:nth-child(3) { left: 40%; animation-duration: 10s; animation-delay: 4s; width: 5px; height: 5px; opacity: 0.25; }
.petal:nth-child(4) { left: 55%; animation-duration: 14s; animation-delay: 1s; width: 7px; height: 7px; opacity: 0.3; }
.petal:nth-child(5) { left: 70%; animation-duration: 11s; animation-delay: 3s; width: 6px; height: 6px; opacity: 0.2; }
.petal:nth-child(6) { left: 85%; animation-duration: 13s; animation-delay: 5s; width: 9px; height: 9px; opacity: 0.4; }
.petal:nth-child(7) { left: 15%; animation-duration: 16s; animation-delay: 7s; width: 5px; height: 5px; opacity: 0.2; }
.petal:nth-child(8) { left: 60%; animation-duration: 9s; animation-delay: 6s; width: 7px; height: 7px; opacity: 0.3; }
.petal:nth-child(9) { left: 35%; animation-duration: 14s; animation-delay: 8s; width: 4px; height: 4px; opacity: 0.25; }
.petal:nth-child(10) { left: 80%; animation-duration: 11s; animation-delay: 2s; width: 6px; height: 6px; opacity: 0.3; }
```

**Step 2: Add petal HTML elements inside the hero section**

Add these divs inside `.hero`, before the kanji:

```html
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
<div class="petal"></div>
```

**Step 3: Verify**

Open in browser — petals should drift slowly downward across the hero. They should NOT appear below the hero section (overflow: hidden on .hero contains them). The animation should feel gentle, not busy.

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add CSS-only falling petal animation in hero"
```

---

### Task 4: "What is Kaishi?" section

**Objective:** Add the introductory section below the hero.

**Files:**
- Modify: `site/index.html`

**Step 1: Add section CSS**

```css
.section {
  padding: 80px 0;
}

.section--surface {
  background: var(--surface);
}

.section--surface-alt {
  background: var(--surface-alt);
}

.section h2 {
  font-family: 'Inter', sans-serif;
  font-weight: 600;
  font-size: clamp(20px, 3vw, 28px);
  color: var(--cream);
  margin-bottom: 20px;
}

.section p {
  font-size: clamp(15px, 1.8vw, 17px);
  line-height: 1.7;
  color: var(--muted);
}
```

**Step 2: Add HTML after the hero closing tag**

```html
<section class="section section--surface">
  <div class="section-inner">
    <h2>What is Kaishi?</h2>
    <p>
      Kaishi is a terminal UI for conversing with
      <a href="https://github.com/hermes-js/hermes-agent" style="color: var(--blossom); text-decoration: none; border-bottom: 1px solid var(--dusk-warm);">Hermes Agent</a>.
      Built in Rust with ratatui, it speaks the ACP protocol — streaming responses in real time,
      highlighting code with syntect, and managing sessions so you can pick up right where you left off.
    </p>
  </div>
</section>
```

**Step 3: Verify**

Scroll below the hero — should see a dark section with the heading and paragraph. Text should be readable and well-spaced.

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add 'What is Kaishi?' section"
```

---

### Task 5: Features section

**Objective:** Add a 4-card feature grid.

**Files:**
- Modify: `site/index.html`

**Step 1: Add feature CSS**

```css
.features {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: 16px;
  margin-top: 32px;
}

.feature-card {
  background: rgba(255, 183, 197, 0.06);
  border: 1px solid rgba(255, 183, 197, 0.08);
  border-radius: 8px;
  padding: 24px;
  transition: border-color 0.2s ease;
}

.feature-card:hover {
  border-color: rgba(255, 183, 197, 0.2);
}

.feature-card h3 {
  font-family: 'Inter', sans-serif;
  font-weight: 600;
  font-size: 15px;
  color: var(--blossom);
  margin-bottom: 8px;
}

.feature-card p {
  font-size: 14px;
  line-height: 1.6;
  color: var(--muted);
}
```

**Step 2: Add HTML**

```html
<section class="section section--surface-alt">
  <div class="section-inner">
    <h2>Features</h2>
    <div class="features">
      <div class="feature-card">
        <h3>Streaming</h3>
        <p>Real-time token flow with animated thinking indicators. Watch responses arrive live.</p>
      </div>
      <div class="feature-card">
        <h3>Syntax Highlighting</h3>
        <p>Code blocks rendered with syntect and the base16-eighties theme. Reads like an editor.</p>
      </div>
      <div class="feature-card">
        <h3>Session Management</h3>
        <p>Pick up conversations where you left off. Session picker with history replay.</p>
      </div>
      <div class="feature-card">
        <h3>File Mentions</h3>
        <p>Type @ to attach files with fuzzy autocomplete. Context without copy-paste.</p>
      </div>
    </div>
  </div>
</section>
```

**Step 3: Verify**

Should see a 4-column grid on desktop, wrapping to fewer columns on narrow viewports. Cards should have subtle borders and hover effects.

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add features section with 4-card grid"
```

---

### Task 6: Terminal preview mock

**Objective:** Build a faithful HTML/CSS reproduction of Kaishi's actual terminal UI showing a short conversation.

**Files:**
- Modify: `site/index.html`

**Step 1: Add terminal CSS**

```css
.terminal {
  background: var(--code-bg);
  border: 1px solid rgba(255, 183, 197, 0.12);
  border-radius: 10px;
  overflow: hidden;
  margin-top: 32px;
  font-family: 'JetBrains Mono', monospace;
  font-size: 13px;
  line-height: 1.6;
}

.terminal-titlebar {
  background: rgba(255, 255, 255, 0.05);
  padding: 10px 16px;
  display: flex;
  align-items: center;
  gap: 8px;
}

.terminal-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.15);
}

.terminal-title {
  font-size: 12px;
  color: var(--muted);
  margin-left: 8px;
}

.terminal-body {
  padding: 16px 20px;
}

.terminal-statusbar {
  background: rgba(255, 255, 255, 0.08);
  padding: 4px 12px;
  font-size: 12px;
  display: flex;
  justify-content: space-between;
  color: var(--muted);
}

.terminal-statusbar .model { color: var(--cream); }
.terminal-statusbar .ctx-bar { color: #4ade80; }

.terminal-messages {
  padding: 12px 0;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.t-line { padding: 1px 0; white-space: pre; }
.t-blank { height: 8px; }
.t-icon-user { color: #67e8f9; }       /* Cyan */
.t-icon-assistant { color: #c084fc; }  /* Magenta */
.t-icon-tool { color: #4ade80; }       /* Green */
.t-dim { color: #6b7280; }             /* DarkGray */
.t-text { color: var(--cream); }
.t-muted { color: var(--muted); }

.terminal-input {
  padding: 8px 0;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
}

.t-cursor {
  display: inline-block;
  width: 7px;
  height: 14px;
  background: var(--cream);
  animation: blink 1s step-end infinite;
  vertical-align: text-bottom;
}

@keyframes blink {
  50% { opacity: 0; }
}

@media (max-width: 600px) {
  .terminal { font-size: 11px; }
  .terminal-body { padding: 12px 14px; }
}
```

**Step 2: Add terminal HTML**

```html
<section class="section section--surface">
  <div class="section-inner">
    <h2>See it in action</h2>
    <div class="terminal">
      <div class="terminal-titlebar">
        <div class="terminal-dot"></div>
        <div class="terminal-dot"></div>
        <div class="terminal-dot"></div>
        <span class="terminal-title">kaishi</span>
      </div>
      <div class="terminal-body">
        <div class="terminal-statusbar">
          <span><span class="model">claude-sonnet-4-6</span> <span class="t-dim">│</span> <span class="ctx-bar">[████░░░░░░] 42%</span></span>
          <span class="t-dim">~/hermes-tui</span>
        </div>
        <div class="terminal-messages">
          <div class="t-line">  <span class="t-icon-user">❯</span> <span class="t-text">What's in this project?</span></div>
          <div class="t-blank"></div>
          <div class="t-line">  <span class="t-icon-assistant">◆</span> <span class="t-text">This is a Rust TUI built with ratatui. It speaks</span></div>
          <div class="t-line">    <span class="t-text">the ACP protocol to communicate with Hermes Agent.</span></div>
          <div class="t-blank"></div>
          <div class="t-line">    <span class="t-dim">┌─</span> <span class="t-icon-tool">✓</span> <span class="t-icon-tool" style="font-weight:bold;">search_files</span> <span class="t-dim">──────────────────</span></div>
          <div class="t-line">    <span class="t-dim">│</span> <span class="t-dim">Found 11 source files in src/</span></div>
          <div class="t-line">    <span class="t-dim">└─────────────────────────────────</span></div>
          <div class="t-blank"></div>
          <div class="t-line">  <span class="t-dim">──── 1.2k in · 247 out · 95% cached · 3s ────</span></div>
        </div>
        <div class="terminal-input">
          <div class="t-line">  <span class="t-dim">❯</span> <span class="t-cursor"></span></div>
        </div>
      </div>
    </div>
  </div>
</section>
```

**Step 3: Verify**

The terminal mock should look like a real terminal window with:
- macOS-style title bar dots
- Status bar with model name and context bar
- User message with cyan ❯
- Assistant response with magenta ◆
- Tool call in a box-drawn frame with green ✓
- Turn summary divider in dim gray
- Blinking cursor in the input area

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add terminal preview mock with accurate Kaishi rendering"
```

---

### Task 7: Get Started section and footer

**Objective:** Add the install instructions and footer.

**Files:**
- Modify: `site/index.html`

**Step 1: Add CSS**

```css
.section--bookend {
  background: linear-gradient(180deg, var(--surface-alt), var(--dusk-deep));
  text-align: center;
}

.install-block {
  display: inline-block;
  background: rgba(0, 0, 0, 0.3);
  border: 1px solid rgba(255, 183, 197, 0.12);
  border-radius: 8px;
  padding: 12px 28px;
  font-family: 'JetBrains Mono', monospace;
  font-size: 15px;
  color: var(--blossom);
  margin: 24px 0;
  cursor: default;
  user-select: all;
}

.section p.hint {
  font-size: 14px;
  color: var(--muted);
  margin-bottom: 24px;
}

.github-link {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  color: var(--blossom);
  text-decoration: none;
  font-size: 15px;
  border-bottom: 1px solid transparent;
  transition: border-color 0.2s;
}

.github-link:hover {
  border-bottom-color: var(--blossom);
}

.footer {
  padding: 32px 0;
  text-align: center;
  background: var(--dusk-deep);
}

.footer p {
  font-size: 13px;
  color: var(--muted);
}
```

**Step 2: Add HTML**

```html
<section class="section section--bookend">
  <div class="section-inner">
    <h2>Get Started</h2>
    <div class="install-block">cargo install kaishi</div>
    <p class="hint">Then just run <code style="font-family: 'JetBrains Mono', monospace; color: var(--blossom);">kaishi</code> in your terminal.</p>
    <a href="https://github.com/eva-l3n4/kaishi" class="github-link" target="_blank" rel="noopener">
      View on GitHub →
    </a>
  </div>
</section>

<footer class="footer">
  <p>Built with 🌸</p>
</footer>
```

**Step 3: Verify**

Scroll to the bottom — should see the gradient bookend with install command, GitHub link, and a minimal footer.

**Step 4: Commit**

```bash
git add site/index.html
git commit -m "feat(site): add Get Started section and footer"
```

---

### Task 8: Responsive polish and final review

**Objective:** Add responsive breakpoints, smooth scrolling, and verify the complete page.

**Files:**
- Modify: `site/index.html`

**Step 1: Add responsive CSS**

```css
html { scroll-behavior: smooth; }

@media (max-width: 768px) {
  .section { padding: 60px 0; }
  .section-inner { padding: 0 20px; }
  .features { grid-template-columns: 1fr; gap: 12px; }
  .hero { min-height: 80vh; }
}

/* Reduce petal animation for users who prefer reduced motion */
@media (prefers-reduced-motion: reduce) {
  .petal { animation: none; display: none; }
}
```

**Step 2: Full page review**

Open the page and verify:
1. Hero fills viewport with gradient, petals drift gently
2. "What is Kaishi?" section is readable
3. Feature cards display in a responsive grid
4. Terminal mock accurately represents Kaishi's UI
5. Get Started section has selectable install command
6. Footer is understated
7. Resize to mobile width — everything stacks and remains readable
8. Page loads quickly (single file, no JS)

**Step 3: Commit**

```bash
git add site/index.html
git commit -m "feat(site): responsive polish and reduced-motion support"
```

---

### Task 9: Netlify deployment setup

**Objective:** Add Netlify config and deploy instructions.

**Files:**
- Create: `netlify.toml`

**Step 1: Create Netlify config**

```toml
[build]
  publish = "site"

# No build command needed — pure static files

# Cache headers for fonts
[[headers]]
  for = "/*"
  [headers.values]
    X-Frame-Options = "DENY"
    X-Content-Type-Options = "nosniff"
```

**Step 2: Commit**

```bash
git add netlify.toml
git commit -m "feat: add Netlify deployment config"
```

**Step 3: Deploy**

1. Go to https://app.netlify.com
2. "Add new site" → "Import an existing project"
3. Connect GitHub → select `eva-l3n4/kaishi`
4. Build settings should auto-detect from `netlify.toml`:
   - Publish directory: `site`
   - Build command: (empty)
5. Deploy

The site will be available at a Netlify subdomain (e.g., `kaishi.netlify.app`).

**Step 4: Verify**

Visit the deployed URL. All fonts should load, petals should animate, layout should be responsive.

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Base HTML scaffold | Create `site/index.html` |
| 2 | Hero section | Modify `site/index.html` |
| 3 | Falling petals | Modify `site/index.html` |
| 4 | What is Kaishi? | Modify `site/index.html` |
| 5 | Features grid | Modify `site/index.html` |
| 6 | Terminal preview | Modify `site/index.html` |
| 7 | Get Started + footer | Modify `site/index.html` |
| 8 | Responsive polish | Modify `site/index.html` |
| 9 | Netlify config | Create `netlify.toml` |

All tasks modify a single file — the plan is strictly sequential. Each task builds on the previous one and can be verified independently.
