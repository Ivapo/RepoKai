use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{prelude::*, widgets::*};
use repokai_core::{
    clone_repo, create_client, fetch_readme, fetch_repos, get_authenticated_user,
    publish_local_repo, update_repo, PublishOptions, Repo, UpdateRepoOptions,
};

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

struct App {
    repos: Vec<Repo>,
    selected: usize,
    readme_content: Option<String>,
    readme_scroll: u16,
    info_field: usize,
    mode: Mode,
    status_msg: Option<String>,
    username: String,
    focused_panel: Panel,
    sort_order: SortOrder,
    repos_original: Vec<Repo>,
}

impl App {
    fn new(repos: Vec<Repo>) -> Self {
        Self {
            repos_original: repos.clone(),
            repos,
            selected: 0,
            readme_content: None,
            readme_scroll: 0,
            info_field: 0,
            mode: Mode::Normal,
            status_msg: None,
            username: String::new(),
            focused_panel: Panel::Repos,
            sort_order: SortOrder::Recent,
        }
    }

    fn selected_repo(&self) -> Option<&Repo> {
        self.repos.get(self.selected)
    }

    fn apply_sort(&mut self) {
        match self.sort_order {
            SortOrder::Recent => {
                self.repos = self.repos_original.clone();
            }
            SortOrder::Alphabetical => {
                self.repos = self.repos_original.clone();
                self.repos.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            }
        }
    }

    fn set_repos(&mut self, repos: Vec<Repo>) {
        self.repos_original = repos;
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

    let mut terminal = ratatui::init();
    let mut app = App::new(repos);
    app.username = username;

    if let Some(repo) = app.selected_repo() {
        let (o, n) = (repo.owner.clone(), repo.name.clone());
        app.readme_content = fetch_readme(&client, &o, &n).await.unwrap_or(None);
    }

    let result = run(&mut terminal, &mut app, &client).await;
    ratatui::restore();
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
            if let Event::Key(key) = event::read()? {
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
                                        if let Some(repo) = app.selected_repo() {
                                            let (o, n) = (repo.owner.clone(), repo.name.clone());
                                            app.readme_content = Some("Loading...".into());
                                            terminal.draw(|frame| ui(frame, app))?;
                                            app.readme_content =
                                                fetch_readme(client, &o, &n).await.unwrap_or(None);
                                            app.readme_scroll = 0;
                                        }
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
                                continue;
                            }
                            KeyCode::Char('r') => {
                                app.status_msg = Some("Refreshing...".into());
                                terminal.draw(|frame| ui(frame, app))?;
                                app.set_repos(fetch_repos(client).await.unwrap_or_default());
                                if app.selected >= app.repos.len() {
                                    app.selected = app.repos.len().saturating_sub(1);
                                }
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
                                        app.selected -= 1;
                                        app.readme_scroll = 0;
                                        app.info_field = 0;
                                        if let Some(repo) = app.selected_repo() {
                                            let (o, n) = (repo.owner.clone(), repo.name.clone());
                                            app.readme_content = Some("Loading...".into());
                                            terminal.draw(|frame| ui(frame, app))?;
                                            app.readme_content =
                                                fetch_readme(client, &o, &n).await.unwrap_or(None);
                                        }
                                    }
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    if app.selected + 1 < app.repos.len() {
                                        app.selected += 1;
                                        app.readme_scroll = 0;
                                        app.info_field = 0;
                                        if let Some(repo) = app.selected_repo() {
                                            let (o, n) = (repo.owner.clone(), repo.name.clone());
                                            app.readme_content = Some("Loading...".into());
                                            terminal.draw(|frame| ui(frame, app))?;
                                            app.readme_content =
                                                fetch_readme(client, &o, &n).await.unwrap_or(None);
                                        }
                                    }
                                }
                                _ => {}
                            },
                            Panel::Info => {
                                const INFO_FIELD_COUNT: usize = 7;
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
                                    app.readme_scroll = app.readme_scroll.saturating_sub(3);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    app.readme_scroll = app.readme_scroll.saturating_add(3);
                                }
                                _ => {}
                            },
                        }
                    }
                }
            }
        }
    }
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
                    app.set_repos(fetch_repos(client).await.unwrap_or_default());
                    app.apply_sort();
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
                        app.set_repos(fetch_repos(client).await.unwrap_or_default());
                        app.apply_sort();
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

fn ui(frame: &mut Frame, app: &App) {
    let main_layout = Layout::vertical([Constraint::Min(0), Constraint::Length(1)])
        .split(frame.area());

    let outer = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(main_layout[0]);

    let right = Layout::vertical([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(outer[1]);

    render_repo_list(frame, app, outer[0]);
    render_repo_info(frame, app, right[0]);
    render_readme(frame, app, right[1]);

    // Status bar
    let status_text = match &app.status_msg {
        Some(msg) => msg.clone(),
        None => format!(
            " {}  \u{2502}  q:quit  Tab:panel  j/k:nav  Enter:readme  s:sort  r:refresh  o:open  p:publish  c:clone  e:edit",
            app.username,
        ),
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

    let height = (state.fields.len() as u16) * 2 + 4;
    let area = centered_rect(60, height, frame.area());
    frame.render_widget(Clear, area);

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
                Span::styled(if focused { " \u{25c0}" } else { "" }, Style::default().fg(Color::Cyan)),
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

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD));

    frame.render_widget(Paragraph::new(lines).block(block), area);
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
    let items: Vec<ListItem> = app
        .repos
        .iter()
        .enumerate()
        .map(|(i, repo)| {
            let icon = if repo.visibility == "private" {
                "\u{f023} "
            } else {
                "  "
            };
            let name_style = Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD);
            let line = Line::from(vec![
                Span::styled(icon, Style::default().fg(Color::DarkGray)),
                Span::styled(&repo.name, name_style),
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
    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(app, Panel::Repos))
            .title(format!(" Repositories [{sort_label}] "))
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(list, area);
}

fn render_readme(frame: &mut Frame, app: &App, area: Rect) {
    let text = app
        .readme_content
        .as_deref()
        .unwrap_or("Press Enter to load README");

    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style(app, Panel::Readme))
                .title(" README ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.readme_scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_repo_info(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focused_panel == Panel::Info;

    let text = if let Some(repo) = app.selected_repo() {
        let desc = repo.description.as_deref().unwrap_or("No description");
        let lang = repo.language.as_deref().unwrap_or("Unknown");
        let star = "\u{2605}";

        let fields: Vec<(&str, Vec<Span>)> = vec![
            ("Name", vec![Span::raw(&repo.name)]),
            ("Description", vec![Span::raw(desc)]),
            ("URL", vec![Span::styled(&repo.url, Style::default().fg(Color::Blue))]),
            ("Language", vec![Span::styled(lang, Style::default().fg(Color::Yellow))]),
            ("Stars", vec![Span::styled(format!("{star} {}", repo.stars), Style::default().fg(Color::Yellow))]),
            ("Visibility", vec![Span::raw(&repo.visibility)]),
            ("Updated", vec![Span::raw(&repo.last_updated)]),
        ];

        fields
            .into_iter()
            .enumerate()
            .map(|(i, (label, value_spans))| {
                let is_selected = focused && i == app.info_field;
                let marker = if is_selected { "\u{25b6} " } else { "  " };
                let label_style = if is_selected {
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };
                let mut spans = vec![
                    Span::styled(marker, Style::default().fg(Color::Cyan)),
                    Span::styled(format!("{label:<13}"), label_style),
                ];
                spans.extend(value_spans);
                Line::from(spans)
            })
            .collect()
    } else {
        vec![Line::from("  No repository selected")]
    };

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style(app, Panel::Info))
            .title(" Info ")
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(paragraph, area);
}
