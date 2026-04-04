# RepoKai

A GitHub repository browser with both terminal and desktop interfaces.

## Workspace Structure

Cargo workspace with three crates under `crates/`:

```
crates/core/   — shared library (GitHub API)
crates/tui/    — terminal UI (ratatui)
crates/gui/    — desktop app (Tauri v2)
```

## Crate Responsibilities

### core (`repokai-core`)
- Authenticates with GitHub via `GITHUB_TOKEN` env var using octocrab
- Fetches the authenticated user's repositories with pagination
- Fetches README content (base64-decoded from GitHub API)
- Exports: `Repo` struct, `create_client()`, `get_authenticated_user()`, `fetch_repos()`, `fetch_readme()`
- `Repo` fields: owner, name, description, url, language, stars, visibility, last_updated, readme

### tui (`repokai-tui`)
- Terminal UI with 3-panel layout: left repo list (30%), top-right README (65%), bottom-right info (35%)
- Uses ratatui + crossterm backend
- Navigation: Arrow keys to browse, Enter to load README, j/k or PageUp/PageDown to scroll README, q/Esc to quit
- Styled after PanEx TUI: green active border, cyan titles, DarkGray inactive borders, blue folder names

### gui (`repokai-gui`)
- Tauri v2 desktop app with the same 3-panel layout
- Frontend in `crates/gui/ui/` — plain HTML/CSS/JS, no bundler
- PanEx "TUI" theme: Tokyo Night palette (`#0f0f17` bg, `#7aa2f7` accent), monospace font, sharp corners, border-breaking panel titles
- Tauri commands: `get_repos`, `get_readme` — thin wrappers around core
- Navigation: Arrow keys + click, same keyboard nav as TUI

## Running

```bash
# Terminal UI
GITHUB_TOKEN=ghp_... cargo run -p repokai-tui

# Desktop GUI
GITHUB_TOKEN=ghp_... cargo run -p repokai-gui
```

## Coding Conventions

- Rust 2021 edition, workspace version/edition inherited
- Async/await throughout (tokio runtime for TUI, Tauri runtime for GUI)
- Errors: `thiserror` in core, `.map_err(|e| e.to_string())` at Tauri command boundaries
- GUI frontend is vanilla JS — no frameworks, no build step
- All colors/styles reference the PanEx design system (see `~/.claude/themes/`)
- Keep core free of UI concerns — it only does data fetching
