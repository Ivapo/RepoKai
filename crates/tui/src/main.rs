use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::{prelude::*, widgets::*};
use repokai_core::{
    clone_repo, create_client, fetch_readme, fetch_repos, fetch_starred_repos,
    get_authenticated_user, publish_local_repo, update_repo, PublishOptions, Repo,
    UpdateRepoOptions,
};

mod render;

// ---- App state ----

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    Recent,
    Alphabetical,
}

impl SortOrder {
    fn toggle(self) -> Self {
        match self {
            SortOrder::Recent => SortOrder::Alphabetical,
            SortOrder::Alphabetical => SortOrder::Recent,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortOrder::Recent => "recent",
            SortOrder::Alphabetical => "a-z",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum RepoSource {
    Mine,
    Starred,
}

impl RepoSource {
    fn toggle(self) -> Self {
        match self {
            RepoSource::Mine => RepoSource::Starred,
            RepoSource::Starred => RepoSource::Mine,
        }
    }

    fn title(self) -> &'static str {
        match self {
            RepoSource::Mine => "Repositories",
            RepoSource::Starred => "Starred \u{2605}",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Repos,
    Info,
    Readme,
}

impl Panel {
    fn next(self) -> Self {
        match self {
            Panel::Repos => Panel::Info,
            Panel::Info => Panel::Readme,
            Panel::Readme => Panel::Repos,
        }
    }

    fn prev(self) -> Self {
        match self {
            Panel::Repos => Panel::Readme,
            Panel::Info => Panel::Repos,
            Panel::Readme => Panel::Info,
        }
    }
}

enum Mode {
    Normal,
    Prompt(PromptState),
    Message(String),
}

struct PromptState {
    kind: PromptKind,
    fields: Vec<PromptField>,
    focused: usize,
}

#[derive(Clone)]
enum PromptKind {
    Publish,
    Clone,
    EditDescription,
}

struct PromptField {
    label: String,
    value: String,
    is_bool: bool,
    bool_val: bool,
    is_path: bool,
    completions: Vec<String>,
    completion_index: Option<usize>,
}

impl PromptField {
    fn text(label: &str, default: &str) -> Self {
        Self {
            label: label.into(), value: default.into(),
            is_bool: false, bool_val: false,
            is_path: false, completions: Vec::new(), completion_index: None,
        }
    }
    fn path(label: &str, default: &str) -> Self {
        Self {
            label: label.into(), value: default.into(),
            is_bool: false, bool_val: false,
            is_path: true, completions: Vec::new(), completion_index: None,
        }
    }
    fn toggle(label: &str, default: bool) -> Self {
        Self {
            label: label.into(), value: String::new(),
            is_bool: true, bool_val: default,
            is_path: false, completions: Vec::new(), completion_index: None,
        }
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        // Hard-break words longer than the line width
        if word_len > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_len = 0;
            for ch in word.chars() {
                if chunk_len == width {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_len = 0;
                }
                chunk.push(ch);
                chunk_len += 1;
            }
            current = chunk;
            current_len = chunk_len;
            continue;
        }
        let needed = if current.is_empty() { word_len } else { current_len + 1 + word_len };
        if needed > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn visual_line_count<'a>(text: impl Into<Text<'a>>, width: u16) -> u16 {
    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .min(u16::MAX as usize) as u16
}

fn init_terminal() -> ratatui::DefaultTerminal {
    let terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    // ratatui's panic hook only calls ratatui::restore(), which doesn't know
    // about mouse capture; chain a hook that turns it off first (LIFO order).
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        prev(info);
    }));
    terminal
}

fn restore_terminal() {
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
}

fn home_dir() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with('~') {
        path.replacen('~', &home_dir(), 1)
    } else {
        path.to_string()
    }
}

fn compute_completions(input: &str) -> Vec<String> {
    let expanded = expand_tilde(input);
    let (dir, prefix) = if expanded.ends_with('/') {
        (expanded.clone(), String::new())
    } else {
        let p = std::path::Path::new(&expanded);
        let dir = p.parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_else(|| "/".into());
        let prefix = p.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        (dir, prefix)
    };

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let prefix_lower = prefix.to_lowercase();
    let mut matches: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .starts_with(&prefix_lower)
        })
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let full = if dir.ends_with('/') {
                format!("{dir}{name}")
            } else {
                format!("{dir}/{name}")
            };
            let full = if e.path().is_dir() {
                format!("{full}/")
            } else {
                full
            };
            // Convert back to tilde if original used it
            if input.starts_with('~') {
                Some(full.replacen(&home_dir(), "~", 1))
            } else {
                Some(full)
            }
        })
        .collect();

    matches.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    matches
}

const INFO_FIELD_COUNT: usize = 8;
const README_HPAD: u16 = 2;

#[derive(Default, Clone, Copy)]
struct PanelAreas {
    repos: Rect,
    info: Rect,
    readme: Rect,
}

struct App {
    repos: Vec<Repo>,
    selected: usize,
    repo_offset: usize,
    readme_content: Option<String>,
    readme_scroll: u16,
    readme_line_count: u16,
    readme_rendered: Option<Text<'static>>,
    readme_rendered_width: u16,
    readme_loading: bool,
    info_field: usize,
    mode: Mode,
    status_msg: Option<String>,
    username: String,
    focused_panel: Panel,
    sort_order: SortOrder,
    source: RepoSource,
    mine: Vec<Repo>,
    starred: Option<Vec<Repo>>,
    areas: PanelAreas,
    // Display row -> info field index, rebuilt each render (the description
    // wraps, so a field can span several rows).
    info_rows: Vec<usize>,
}

impl App {
    fn new(repos: Vec<Repo>) -> Self {
        Self {
            mine: repos.clone(),
            starred: None,
            source: RepoSource::Mine,
            repos,
            selected: 0,
            repo_offset: 0,
            readme_content: None,
            readme_scroll: 0,
            readme_line_count: 0,
            readme_rendered: None,
            readme_rendered_width: 0,
            readme_loading: false,
            info_field: 0,
            mode: Mode::Normal,
            status_msg: None,
            username: String::new(),
            focused_panel: Panel::Repos,
            sort_order: SortOrder::Recent,
            areas: PanelAreas::default(),
            info_rows: Vec::new(),
        }
    }

    fn selected_repo(&self) -> Option<&Repo> {
        self.repos.get(self.selected)
    }

    fn repos_viewport(&self) -> u16 {
        self.areas.repos.height.saturating_sub(2)
    }

    fn clamp_repo_offset(&mut self) {
        let vh = self.repos_viewport().max(1) as usize;
        self.repo_offset = self.repo_offset.min(self.repos.len().saturating_sub(vh));
    }

    fn ensure_selected_visible(&mut self) {
        let vh = self.repos_viewport() as usize;
        if vh == 0 {
            return;
        }
        if self.selected < self.repo_offset {
            self.repo_offset = self.selected;
        } else if self.selected >= self.repo_offset + vh {
            self.repo_offset = self.selected + 1 - vh;
        }
    }

    fn scroll_repo_list(&mut self, delta: i32) {
        let vh = self.repos_viewport().max(1) as usize;
        let max = self.repos.len().saturating_sub(vh);
        let next = (self.repo_offset as i64 + delta as i64).clamp(0, max as i64);
        self.repo_offset = next as usize;
    }

    fn panel_at(&self, x: u16, y: u16) -> Option<Panel> {
        let pos = Position::new(x, y);
        if self.areas.repos.contains(pos) {
            Some(Panel::Repos)
        } else if self.areas.info.contains(pos) {
            Some(Panel::Info)
        } else if self.areas.readme.contains(pos) {
            Some(Panel::Readme)
        } else {
            None
        }
    }

    fn repo_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let inner = self.areas.repos.inner(Margin::new(1, 1));
        if !inner.contains(Position::new(x, y)) {
            return None;
        }
        let index = self.repo_offset + (y - inner.y) as usize;
        (index < self.repos.len()).then_some(index)
    }

    fn info_row_at(&self, x: u16, y: u16) -> Option<usize> {
        let inner = self.areas.info.inner(Margin::new(1, 1));
        if !inner.contains(Position::new(x, y)) {
            return None;
        }
        let row = (y - inner.y) as usize;
        self.info_rows.get(row).copied()
    }

    fn readme_viewport(&self) -> u16 {
        self.areas.readme.height.saturating_sub(2)
    }

    fn max_readme_scroll(&self) -> u16 {
        // Leave one blank row below the last line as an end-of-content marker.
        self.readme_line_count
            .saturating_sub(self.readme_viewport().max(1).saturating_sub(1))
    }

    fn readme_scroll_by(&mut self, delta: i32) {
        let next = (self.readme_scroll as i32 + delta).clamp(0, self.max_readme_scroll() as i32);
        self.readme_scroll = next as u16;
    }

    fn invalidate_readme_layout(&mut self) {
        self.readme_rendered = None;
        self.readme_rendered_width = 0;
    }

    fn ensure_readme_rendered(&mut self, inner_width: u16) {
        if inner_width == 0 || self.readme_rendered_width == inner_width {
            return;
        }
        match self.readme_content.as_deref() {
            Some(content) if !self.readme_loading => {
                let rendered = render::render(content, inner_width);
                self.readme_line_count = visual_line_count(rendered.clone(), inner_width);
                self.readme_rendered = Some(rendered);
            }
            other => {
                // Placeholders ("Loading...", "Press Enter to load README")
                // stay on the plain-text path.
                let text = other.unwrap_or("Press Enter to load README");
                self.readme_line_count = visual_line_count(text, inner_width);
                self.readme_rendered = None;
            }
        }
        self.readme_rendered_width = inner_width;
        self.readme_scroll = self.readme_scroll.min(self.max_readme_scroll());
    }

    fn apply_sort(&mut self) {
        let base = match self.source {
            RepoSource::Mine => &self.mine,
            RepoSource::Starred => self.starred.as_deref().unwrap_or(&[]),
        };
        self.repos = base.to_vec();
        if self.sort_order == SortOrder::Alphabetical {
            self.repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        }
    }

    fn set_mine(&mut self, repos: Vec<Repo>) {
        self.mine = repos;
        self.apply_sort();
    }

    fn set_starred(&mut self, repos: Vec<Repo>) {
        self.starred = Some(repos);
        self.apply_sort();
    }
}

// ---- Main ----

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = create_client().await?;
    let username = get_authenticated_user(&client).await?;
    let repos = fetch_repos(&client).await?;

    if repos.is_empty() {
        eprintln!("No repositories found.");
        return Ok(());
    }

    let mut terminal = init_terminal();
    let mut app = App::new(repos);
    app.username = username;

    if let Some(repo) = app.selected_repo() {
        let (o, n) = (repo.owner.clone(), repo.name.clone());
        app.readme_content = fetch_readme(&client, &o, &n).await.unwrap_or(None);
    }

    let result = run(&mut terminal, &mut app, &client).await;
    restore_terminal();
    result
}

// ---- Event loop ----

async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &repokai_core::Octocrab,
) -> Result<(), Box<dyn std::error::Error>> {
    loop {
        terminal.draw(|frame| ui(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Mouse(mouse) => {
                    handle_mouse(terminal, app, client, mouse).await?;
                }
                Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match &mut app.mode {
                    Mode::Message(_) => {
                        app.mode = Mode::Normal;
                    }
                    Mode::Prompt(state) => match key.code {
                        KeyCode::Esc => {
                            app.mode = Mode::Normal;
                        }
                        KeyCode::Tab => {
                            let field = &mut state.fields[state.focused];
                            if field.is_path {
                                if field.completions.is_empty() || field.completion_index.is_none() {
                                    let matches = compute_completions(&field.value);
                                    if matches.is_empty() {
                                        // No completions — move to next field
                                        state.focused = (state.focused + 1) % state.fields.len();
                                    } else if matches.len() == 1 && matches[0] == field.value {
                                        // Already the only match — move to next field
                                        field.completions.clear();
                                        field.completion_index = None;
                                        state.focused = (state.focused + 1) % state.fields.len();
                                    } else {
                                        field.completions = matches;
                                        field.completion_index = Some(0);
                                        field.value = field.completions[0].clone();
                                    }
                                } else if let Some(idx) = field.completion_index {
                                    let next = (idx + 1) % field.completions.len();
                                    field.completion_index = Some(next);
                                    field.value = field.completions[next].clone();
                                }
                            } else {
                                state.focused = (state.focused + 1) % state.fields.len();
                            }
                        }
                        KeyCode::BackTab => {
                            let field = &mut state.fields[state.focused];
                            if field.is_path && !field.completions.is_empty() {
                                if let Some(idx) = field.completion_index {
                                    let prev = if idx == 0 { field.completions.len() - 1 } else { idx - 1 };
                                    field.completion_index = Some(prev);
                                    field.value = field.completions[prev].clone();
                                }
                            } else {
                                if state.focused > 0 {
                                    state.focused -= 1;
                                } else {
                                    state.focused = state.fields.len() - 1;
                                }
                            }
                        }
                        KeyCode::Down => {
                            state.focused = (state.focused + 1) % state.fields.len();
                        }
                        KeyCode::Up => {
                            if state.focused > 0 {
                                state.focused -= 1;
                            } else {
                                state.focused = state.fields.len() - 1;
                            }
                        }
                        KeyCode::Char(' ') if state.fields[state.focused].is_bool => {
                            state.fields[state.focused].bool_val =
                                !state.fields[state.focused].bool_val;
                        }
                        KeyCode::Char(c) if !state.fields[state.focused].is_bool => {
                            let field = &mut state.fields[state.focused];
                            field.value.push(c);
                            field.completions.clear();
                            field.completion_index = None;
                        }
                        KeyCode::Backspace if !state.fields[state.focused].is_bool => {
                            let field = &mut state.fields[state.focused];
                            if field.is_path && !field.value.is_empty() {
                                // Smart backspace: go up a directory if at end with trailing slash
                                if field.value.ends_with('/') && field.value.len() > 1 {
                                    let trimmed = field.value.trim_end_matches('/');
                                    if let Some(pos) = trimmed.rfind('/') {
                                        field.value = trimmed[..=pos].to_string();
                                    } else {
                                        field.value.pop();
                                    }
                                } else {
                                    field.value.pop();
                                }
                                field.completions.clear();
                                field.completion_index = None;
                            } else {
                                field.value.pop();
                            }
                        }
                        KeyCode::Enter => {
                            let kind = state.kind.clone();
                            let fields: Vec<(String, bool)> = state
                                .fields
                                .iter()
                                .map(|f| {
                                    if f.is_bool {
                                        (f.bool_val.to_string(), f.bool_val)
                                    } else {
                                        (f.value.clone(), false)
                                    }
                                })
                                .collect();
                            app.mode = Mode::Normal;
                            handle_prompt_submit(terminal, app, client, kind, fields).await?;
                        }
                        _ => {}
                    },
                    Mode::Normal => {
                        // Global keys (work regardless of panel)
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Tab => {
                                app.focused_panel = app.focused_panel.next();
                                continue;
                            }
                            KeyCode::BackTab => {
                                app.focused_panel = app.focused_panel.prev();
                                continue;
                            }
                            KeyCode::Enter => {
                                match app.focused_panel {
                                    Panel::Repos => {
                                        select_repo(terminal, app, client, app.selected).await?;
                                    }
                                    Panel::Info => {
                                        // URL field — open in browser
                                        if app.info_field == 2 {
                                            if let Some(repo) = app.selected_repo() {
                                                let url = repo.url.clone();
                                                let _ = std::process::Command::new("open")
                                                    .arg(&url)
                                                    .spawn();
                                            }
                                        }
                                    }
                                    Panel::Readme => {}
                                }
                                continue;
                            }
                            KeyCode::Char('p') => {
                                app.mode = Mode::Prompt(PromptState {
                                    kind: PromptKind::Publish,
                                    fields: vec![
                                        PromptField::path("Local path", "~/"),
                                        PromptField::text("Repo name", ""),
                                        PromptField::text("Description", ""),
                                        PromptField::toggle("Private", false),
                                    ],
                                    focused: 0,
                                });
                                continue;
                            }
                            KeyCode::Char('c') => {
                                if let Some(repo) = app.selected_repo() {
                                    let default_dest = format!("~/{}", repo.name);
                                    app.mode = Mode::Prompt(PromptState {
                                        kind: PromptKind::Clone,
                                        fields: vec![PromptField::path(
                                            "Destination",
                                            &default_dest,
                                        )],
                                        focused: 0,
                                    });
                                }
                                continue;
                            }
                            KeyCode::Char('s') => {
                                app.sort_order = app.sort_order.toggle();
                                app.apply_sort();
                                app.selected = 0;
                                app.repo_offset = 0;
                                continue;
                            }
                            KeyCode::Char('v') => {
                                app.source = app.source.toggle();
                                if app.source == RepoSource::Starred && app.starred.is_none() {
                                    app.status_msg = Some("Loading starred repos...".into());
                                    terminal.draw(|frame| ui(frame, app))?;
                                    app.starred =
                                        Some(fetch_starred_repos(client).await.unwrap_or_default());
                                    app.status_msg = None;
                                }
                                app.apply_sort();
                                app.repo_offset = 0;
                                if app.repos.is_empty() {
                                    app.selected = 0;
                                    app.readme_content = None;
                                    app.invalidate_readme_layout();
                                } else {
                                    select_repo(terminal, app, client, 0).await?;
                                }
                                continue;
                            }
                            KeyCode::Char('r') => {
                                app.status_msg = Some("Refreshing...".into());
                                terminal.draw(|frame| ui(frame, app))?;
                                match app.source {
                                    RepoSource::Mine => {
                                        app.set_mine(fetch_repos(client).await.unwrap_or_default());
                                    }
                                    RepoSource::Starred => {
                                        app.set_starred(
                                            fetch_starred_repos(client).await.unwrap_or_default(),
                                        );
                                    }
                                }
                                if app.selected >= app.repos.len() {
                                    app.selected = app.repos.len().saturating_sub(1);
                                }
                                app.clamp_repo_offset();
                                app.ensure_selected_visible();
                                app.status_msg = None;
                                continue;
                            }
                            KeyCode::Char('o') => {
                                if let Some(repo) = app.selected_repo() {
                                    let url = repo.url.clone();
                                    let _ = std::process::Command::new("open")
                                        .arg(&url)
                                        .spawn();
                                }
                                continue;
                            }
                            KeyCode::Char('e') => {
                                if let Some(repo) = app.selected_repo() {
                                    let desc =
                                        repo.description.as_deref().unwrap_or("").to_string();
                                    let is_private = repo.visibility == "private";
                                    app.mode = Mode::Prompt(PromptState {
                                        kind: PromptKind::EditDescription,
                                        fields: vec![
                                            PromptField::text("Description", &desc),
                                            PromptField::toggle("Private", is_private),
                                        ],
                                        focused: 0,
                                    });
                                }
                                continue;
                            }
                            _ => {}
                        }

                        // Panel-specific j/k and arrow keys
                        match app.focused_panel {
                            Panel::Repos => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => {
                                    if app.selected > 0 {
                                        select_repo(terminal, app, client, app.selected - 1)
                                            .await?;
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if app.selected + 1 < app.repos.len() {
                                        select_repo(terminal, app, client, app.selected + 1)
                                            .await?;
                                    }
                                }
                                _ => {}
                            },
                            Panel::Info => {
                                match key.code {
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        if app.info_field > 0 {
                                            app.info_field -= 1;
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if app.info_field + 1 < INFO_FIELD_COUNT {
                                            app.info_field += 1;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Panel::Readme => match key.code {
                                KeyCode::Up | KeyCode::Char('k') => {
                                    app.readme_scroll_by(-3);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.readme_scroll_by(3);
                                }
                                KeyCode::PageUp => {
                                    let page = app.readme_viewport().saturating_sub(1) as i32;
                                    app.readme_scroll_by(-page.max(1));
                                }
                                KeyCode::PageDown => {
                                    let page = app.readme_viewport().saturating_sub(1) as i32;
                                    app.readme_scroll_by(page.max(1));
                                }
                                _ => {}
                            },
                        }
                    }
                }
                }
                _ => {}
            }
        }
    }
}

async fn select_repo(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &repokai_core::Octocrab,
    index: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    app.selected = index;
    app.readme_scroll = 0;
    app.info_field = 0;
    app.ensure_selected_visible();
    if let Some(repo) = app.selected_repo() {
        let (o, n) = (repo.owner.clone(), repo.name.clone());
        app.readme_content = Some("Loading...".into());
        app.readme_loading = true;
        app.invalidate_readme_layout();
        terminal.draw(|frame| ui(frame, app))?;
        app.readme_content = fetch_readme(client, &o, &n).await.unwrap_or(None);
        app.readme_loading = false;
        app.invalidate_readme_layout();
    }
    Ok(())
}

async fn handle_mouse(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &repokai_core::Octocrab,
    mouse: MouseEvent,
) -> Result<(), Box<dyn std::error::Error>> {
    match &app.mode {
        Mode::Prompt(_) => return Ok(()),
        Mode::Message(_) => {
            if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                app.mode = Mode::Normal;
            }
            return Ok(());
        }
        Mode::Normal => {}
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => match app.panel_at(mouse.column, mouse.row) {
            Some(Panel::Repos) => app.scroll_repo_list(-1),
            Some(Panel::Readme) => app.readme_scroll_by(-3),
            _ => {}
        },
        MouseEventKind::ScrollDown => match app.panel_at(mouse.column, mouse.row) {
            Some(Panel::Repos) => app.scroll_repo_list(1),
            Some(Panel::Readme) => app.readme_scroll_by(3),
            _ => {}
        },
        MouseEventKind::Down(MouseButton::Left) => {
            let Some(panel) = app.panel_at(mouse.column, mouse.row) else {
                return Ok(());
            };
            app.focused_panel = panel;
            match panel {
                Panel::Repos => {
                    if let Some(index) = app.repo_row_at(mouse.column, mouse.row) {
                        if index != app.selected {
                            select_repo(terminal, app, client, index).await?;
                        }
                    }
                }
                Panel::Info => {
                    if let Some(row) = app.info_row_at(mouse.column, mouse.row) {
                        app.info_field = row;
                    }
                }
                Panel::Readme => {}
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_prompt_submit(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    client: &repokai_core::Octocrab,
    kind: PromptKind,
    fields: Vec<(String, bool)>,
) -> Result<(), Box<dyn std::error::Error>> {
    match kind {
        PromptKind::Publish => {
            let local_path = expand_tilde(&fields[0].0);
            let name = fields[1].0.clone();
            let description = fields[2].0.clone();
            let private = fields[3].1;

            if local_path.is_empty() || name.is_empty() {
                app.mode = Mode::Message("Path and name are required".into());
                return Ok(());
            }

            app.status_msg = Some("Publishing...".into());
            terminal.draw(|frame| ui(frame, app))?;

            match publish_local_repo(
                client,
                &PublishOptions { local_path, name, description, private },
            )
            .await
            {
                Ok(_) => {
                    app.set_mine(fetch_repos(client).await.unwrap_or_default());
                    app.status_msg = Some("Published successfully!".into());
                }
                Err(e) => {
                    app.mode = Mode::Message(format!("Error: {e}"));
                }
            }
        }
        PromptKind::Clone => {
            let destination = expand_tilde(&fields[0].0);
            if destination.is_empty() {
                app.mode = Mode::Message("Destination is required".into());
                return Ok(());
            }

            if let Some(repo) = app.selected_repo() {
                let url = format!("{}.git", repo.url);
                app.status_msg = Some("Cloning...".into());
                terminal.draw(|frame| ui(frame, app))?;

                match clone_repo(&url, &destination) {
                    Ok(()) => {
                        app.status_msg = Some(format!("Cloned to {destination}"));
                    }
                    Err(e) => {
                        app.mode = Mode::Message(format!("Error: {e}"));
                    }
                }
            }
        }
        PromptKind::EditDescription => {
            let description = fields[0].0.clone();
            let private = fields[1].1;

            if let Some(repo) = app.selected_repo() {
                let (o, n) = (repo.owner.clone(), repo.name.clone());
                app.status_msg = Some("Saving...".into());
                terminal.draw(|frame| ui(frame, app))?;

                match update_repo(
                    client,
                    &o,
                    &n,
                    &UpdateRepoOptions {
                        description: Some(description),
                        private: Some(private),
                    },
                )
                .await
                {
                    Ok(()) => {
                        match app.source {
                            RepoSource::Mine => {
                                app.set_mine(fetch_repos(client).await.unwrap_or_default());
                            }
                            RepoSource::Starred => {
                                app.set_starred(
                                    fetch_starred_repos(client).await.unwrap_or_default(),
                                );
                            }
                        }
                        app.status_msg = Some("Updated!".into());
                    }
                    Err(e) => {
                        app.mode = Mode::Message(format!("Error: {e}"));
                    }
                }
            }
        }
    }
    app.status_msg = None;
    Ok(())
}

// ---- UI rendering ----

fn border_style(app: &App, panel: Panel) -> Style {
    if app.focused_panel == panel {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn ui(frame: &mut Frame, app: &mut App) {
    let main_layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let outer = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(main_layout[0]);

    let right = Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer[1]);

    app.areas = PanelAreas { repos: outer[0], info: right[0], readme: right[1] };
    app.clamp_repo_offset();
    app.ensure_readme_rendered(
        app.areas.readme.width.saturating_sub(2 + 2 * README_HPAD),
    );

    render_repo_list(frame, app, app.areas.repos);
    render_repo_info(frame, app, app.areas.info);
    render_readme(frame, app, app.areas.readme);

    // Status bar
    let status_text = match &app.status_msg {
        Some(msg) => msg.clone(),
        None => {
            let toggle_label = match app.source {
                RepoSource::Mine => "v:starred",
                RepoSource::Starred => "v:my repos",
            };
            format!(
                " {}  \u{2502}  q:quit  Tab:panel  j/k:nav  Enter:readme  {toggle_label}  s:sort  r:refresh  o:open  p:publish  c:clone  e:edit",
                app.username,
            )
        }
    };
    let status_style = if app.status_msg.is_some() {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(Paragraph::new(status_text).style(status_style), main_layout[1]);

    // Draw prompt or message overlay
    match &app.mode {
        Mode::Prompt(state) => render_prompt(frame, state),
        Mode::Message(msg) => render_message(frame, msg),
        Mode::Normal => {}
    }
}

fn render_prompt(frame: &mut Frame, state: &PromptState) {
    let title = match state.kind {
        PromptKind::Publish => " Publish to GitHub ",
        PromptKind::Clone => " Clone Repository ",
        PromptKind::EditDescription => " Edit Repository ",
    };

    // Fix the horizontal footprint first so wrapped line counts use the real
    // inner width; the popup then grows vertically to fit long values.
    let h_area = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(60),
        Constraint::Percentage(20),
    ])
    .split(frame.area())[1];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));
    for (i, field) in state.fields.iter().enumerate() {
        let focused = i == state.focused;
        let label_style = if focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        if field.is_bool {
            let check = if field.bool_val { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", field.label), label_style),
                Span::styled(check, Style::default().fg(Color::White)),
                Span::styled(
                    if focused { "  Space:toggle" } else { "" },
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        } else {
            let mut label_line = vec![Span::styled(
                format!("  {}", field.label),
                label_style,
            )];
            if focused && field.is_path && field.completion_index.is_some() {
                let idx = field.completion_index.unwrap() + 1;
                let total = field.completions.len();
                label_line.push(Span::styled(
                    format!(" ({idx}/{total})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            lines.push(Line::from(label_line));
            let cursor = if focused { "\u{2588}" } else { "" };
            let hint = if focused && field.is_path {
                "  Tab:complete  Bksp:up dir"
            } else {
                ""
            };
            lines.push(Line::from(vec![
                Span::raw(format!("  > {}", field.value)),
                Span::styled(cursor, Style::default().fg(Color::White)),
                Span::styled(hint, Style::default().fg(Color::DarkGray)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Enter:confirm  Tab:next  Esc:cancel",
        Style::default().fg(Color::DarkGray),
    )));

    let inner_width = h_area.width.saturating_sub(2);
    let height =
        (visual_line_count(lines.clone(), inner_width) + 2).min(frame.area().height);
    let area = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(h_area)[1];
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    frame.render_widget(
        Paragraph::new(lines).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_message(frame: &mut Frame, msg: &str) {
    let area = centered_rect(50, 5, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(" Message ")
        .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));

    let text = vec![
        Line::from(""),
        Line::from(format!("  {msg}")),
        Line::from(Span::styled(
            "  Press any key to dismiss",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(Paragraph::new(text).block(block), area);
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ])
    .split(area);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}

fn render_repo_list(frame: &mut Frame, app: &App, area: Rect) {
    // Available width inside borders
    let inner_width = area.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = app
        .repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let repo_icon = "\u{f0cd0} "; // repo icon
            let lock = if repo.visibility == "private" { " \u{f023}" } else { "" };
            let name = &repo.name;

            // repo_icon (2) + name + padding + lock (2 or 0)
            let used = 2 + name.len() + lock.len();
            let padding = if inner_width > used { inner_width - used } else { 0 };

            let name_style = Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD);
            let line = Line::from(vec![
                Span::styled(repo_icon, Style::default().fg(Color::Rgb(255, 191, 0))),
                Span::styled(name, name_style),
                Span::raw(" ".repeat(padding)),
                Span::styled(lock, Style::default().fg(Color::DarkGray)),
            ]);
            let style = if i == app.selected {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let sort_label = app.sort_order.label();
    let source_title = app.source.title();
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(app, Panel::Repos))
            .title(format!(" {source_title} [{sort_label}] "))
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    // Selection is styled manually above; keep ListState's selected as None so
    // the offset is honored verbatim (a Some selection would snap the viewport
    // to it every frame, fighting view-only mouse scrolling).
    let mut state = ListState::default().with_offset(app.repo_offset);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_readme(frame: &mut Frame, app: &App, area: Rect) {
    let paragraph = match &app.readme_rendered {
        Some(rendered) => Paragraph::new(rendered.clone()),
        None => Paragraph::new(
            app.readme_content
                .as_deref()
                .unwrap_or("Press Enter to load README"),
        ),
    };
    let paragraph = paragraph
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(app, Panel::Readme))
                .padding(Padding::horizontal(README_HPAD))
                .title(" README ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.readme_scroll, 0));
    frame.render_widget(paragraph, area);

    if app.readme_line_count > app.readme_viewport() {
        let mut scrollbar_state = ScrollbarState::new(app.max_readme_scroll() as usize)
            .position(app.readme_scroll as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .style(Style::default().fg(Color::DarkGray)),
            area.inner(Margin { horizontal: 0, vertical: 1 }),
            &mut scrollbar_state,
        );
    }
}

fn render_repo_info(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focused_panel == Panel::Info;
    let inner_width = area.width.saturating_sub(2) as usize;
    // Every field line starts with marker (2) + label column (13)
    let value_width = inner_width.saturating_sub(15).max(10);

    let mut info_rows: Vec<usize> = Vec::new();
    let text = if let Some(repo) = app.selected_repo().cloned() {
        let desc = repo.description.as_deref().unwrap_or("No description").to_string();
        let lang = repo.language.as_deref().unwrap_or("Unknown").to_string();
        let license = repo.license.as_deref().unwrap_or("None").to_string();
        let star = "\u{2605}";

        let fields: Vec<(&str, Vec<Span>)> = vec![
            ("Name", vec![Span::raw(repo.name)]),
            ("Description", Vec::new()), // wrapped below
            ("URL", vec![Span::styled(repo.url, Style::default().fg(Color::Blue))]),
            ("Language", vec![Span::styled(lang, Style::default().fg(Color::Yellow))]),
            ("License", vec![Span::raw(license)]),
            ("Stars", vec![Span::styled(format!("{star} {}", repo.stars), Style::default().fg(Color::Yellow))]),
            ("Visibility", vec![Span::raw(repo.visibility)]),
            ("Updated", vec![Span::raw(repo.last_updated)]),
        ];

        let mut lines: Vec<Line> = Vec::new();
        for (i, (label, value_spans)) in fields.into_iter().enumerate() {
            let is_selected = focused && i == app.info_field;
            let marker = if is_selected { "\u{25b6} " } else { "  " };
            let label_style = if is_selected {
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let head = vec![
                Span::styled(marker, Style::default().fg(Color::Cyan)),
                Span::styled(format!("{label:<13}"), label_style),
            ];
            if label == "Description" {
                for (j, seg) in wrap_text(&desc, value_width).into_iter().enumerate() {
                    let mut spans = if j == 0 {
                        head.clone()
                    } else {
                        // Hanging indent: align continuation under the value column
                        vec![Span::raw(" ".repeat(15))]
                    };
                    spans.push(Span::raw(seg));
                    lines.push(Line::from(spans));
                    info_rows.push(i);
                }
            } else {
                let mut spans = head;
                spans.extend(value_spans);
                lines.push(Line::from(spans));
                info_rows.push(i);
            }
        }
        lines
    } else {
        vec![Line::from("  No repository selected")]
    };
    app.info_rows = info_rows;

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(app, Panel::Info))
            .title(" Info ")
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(paragraph, area);
}
