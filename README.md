# RepoKai

A GitHub repository browser and manager with both a terminal UI and a desktop GUI.

## Features

- Browse your GitHub repositories with a 3-panel layout
- View README files (raw markdown or rendered HTML in the GUI)
- View repo info: description, language, stars, visibility, last updated
- Publish a local git repo to GitHub
- Clone repos to your machine
- Edit repo description and visibility
- Open repos in your browser
- 4 visual themes: Dark, Light, 3.1 (retro DOS), TUI (Tokyo Night)

## Prerequisites

- **Git** must be installed and available in your PATH
- **GitHub authentication** is required. Either:
  - Set the `GITHUB_TOKEN` environment variable with a personal access token, or
  - Log in with the [GitHub CLI](https://cli.github.com): `gh auth login`

## Running

### Terminal UI

```bash
cargo run -p repokai-tui
```

### Desktop GUI

```bash
# Build the frontend first
cd crates/gui/ui && bun install && bun run build && cd -

# Run the app
cargo run -p repokai-gui
```

For hot-reload during development:

```bash
cd crates/gui && cargo tauri dev
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Up/Down | Navigate repo list |
| Enter | Load README |
| j/k | Scroll README |
| o | Open repo in browser |
| p | Publish local repo |
| c | Clone selected repo |
| e | Edit repo settings |
| t | Cycle theme (GUI) |
| q/Esc | Quit (TUI) |

## License

MIT
