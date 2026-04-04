use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{prelude::*, widgets::*};
use repokai_core::{
    clone_repo, create_client, fetch_readme, fetch_repos, get_authenticated_user,
    publish_local_repo, update_repo, PublishOptions, Repo, UpdateRepoOptions,
};

// ---- App state ----

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
}

impl PromptField {
    fn text(label: &str, default: &str) -> Self {
        Self { label: label.into(), value: default.into(), is_bool: false, bool_val: false }
    }
    fn toggle(label: &str, default: bool) -> Self {
        Self { label: label.into(), value: String::new(), is_bool: true, bool_val: default }
    }
}

struct App {
    repos: Vec<Repo>,
    selected: usize,
    readme_content: Option<String>,
    readme_scroll: u16,
    mode: Mode,
    status_msg: Option<String>,
    username: String,
}

impl App {
    fn new(repos: Vec<Repo>) -> Self {
        Self {
            repos,
            selected: 0,
            readme_content: None,
            readme_scroll: 0,
            mode: Mode::Normal,
            status_msg: None,
            username: String::new(),
        }
    }

    fn selected_repo(&self) -> Option<&Repo> {
        self.repos.get(self.selected)
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
                        KeyCode::Tab | KeyCode::Down => {
                            state.focused = (state.focused + 1) % state.fields.len();
                        }
                        KeyCode::BackTab | KeyCode::Up => {
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
                            state.fields[state.focused].value.push(c);
                        }
                        KeyCode::Backspace if !state.fields[state.focused].is_bool => {
                            state.fields[state.focused].value.pop();
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
                    Mode::Normal => match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Up => {
                            if app.selected > 0 {
                                app.selected -= 1;
                                app.readme_scroll = 0;
                            }
                        }
                        KeyCode::Down => {
                            if app.selected + 1 < app.repos.len() {
                                app.selected += 1;
                                app.readme_scroll = 0;
                            }
                        }
                        KeyCode::Enter => {
                            if let Some(repo) = app.selected_repo() {
                                let (o, n) = (repo.owner.clone(), repo.name.clone());
                                app.readme_content = Some("Loading...".into());
                                terminal.draw(|frame| ui(frame, app))?;
                                app.readme_content =
                                    fetch_readme(client, &o, &n).await.unwrap_or(None);
                                app.readme_scroll = 0;
                            }
                        }
                        KeyCode::PageDown | KeyCode::Char('j') => {
                            app.readme_scroll = app.readme_scroll.saturating_add(3);
                        }
                        KeyCode::PageUp | KeyCode::Char('k') => {
                            app.readme_scroll = app.readme_scroll.saturating_sub(3);
                        }
                        KeyCode::Char('p') => {
                            app.mode = Mode::Prompt(PromptState {
                                kind: PromptKind::Publish,
                                fields: vec![
                                    PromptField::text("Local path", ""),
                                    PromptField::text("Repo name", ""),
                                    PromptField::text("Description", ""),
                                    PromptField::toggle("Private", false),
                                ],
                                focused: 0,
                            });
                        }
                        KeyCode::Char('c') => {
                            if let Some(repo) = app.selected_repo() {
                                let default_dest =
                                    format!("~/{}", repo.name);
                                app.mode = Mode::Prompt(PromptState {
                                    kind: PromptKind::Clone,
                                    fields: vec![PromptField::text(
                                        "Destination",
                                        &default_dest,
                                    )],
                                    focused: 0,
                                });
                            }
                        }
                        KeyCode::Char('o') => {
                            if let Some(repo) = app.selected_repo() {
                                let url = repo.url.clone();
                                let _ = std::process::Command::new("open")
                                    .arg(&url)
                                    .spawn();
                            }
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
                        }
                        _ => {}
                    },
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
            let local_path = fields[0].0.clone();
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
                    app.repos = fetch_repos(client).await.unwrap_or_default();
                    app.status_msg = Some("Published successfully!".into());
                }
                Err(e) => {
                    app.mode = Mode::Message(format!("Error: {e}"));
                }
            }
        }
        PromptKind::Clone => {
            let destination = fields[0].0.clone();
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
                        app.repos = fetch_repos(client).await.unwrap_or_default();
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
            " {}  \u{2502}  q:quit  \u{2191}\u{2193}:nav  Enter:readme  j/k:scroll  o:open  p:publish  c:clone  e:edit",
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
            lines.push(Line::from(Span::styled(
                format!("  {}", field.label),
                label_style,
            )));
            let cursor = if focused { "\u{2588}" } else { "" };
            lines.push(Line::from(vec![
                Span::raw(format!("  > {}", field.value)),
                Span::styled(cursor, Style::default().fg(Color::White)),
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

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Repositories ")
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
                .border_style(Style::default().fg(Color::DarkGray))
                .title(" README ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        )
        .wrap(Wrap { trim: false })
        .scroll((app.readme_scroll, 0));
    frame.render_widget(paragraph, area);
}

fn render_repo_info(frame: &mut Frame, app: &App, area: Rect) {
    let text = if let Some(repo) = app.selected_repo() {
        let desc = repo.description.as_deref().unwrap_or("No description");
        let lang = repo.language.as_deref().unwrap_or("Unknown");
        let star = "\u{2605}";
        vec![
            Line::from(vec![
                Span::styled("  Name        ", Style::default().fg(Color::DarkGray)),
                Span::raw(&repo.name),
            ]),
            Line::from(vec![
                Span::styled("  Description ", Style::default().fg(Color::DarkGray)),
                Span::raw(desc),
            ]),
            Line::from(vec![
                Span::styled("  URL         ", Style::default().fg(Color::DarkGray)),
                Span::styled(&repo.url, Style::default().fg(Color::Blue)),
            ]),
            Line::from(vec![
                Span::styled("  Language    ", Style::default().fg(Color::DarkGray)),
                Span::styled(lang, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  Stars       {star} "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(repo.stars.to_string(), Style::default().fg(Color::Yellow)),
            ]),
            Line::from(vec![
                Span::styled("  Visibility  ", Style::default().fg(Color::DarkGray)),
                Span::raw(&repo.visibility),
            ]),
            Line::from(vec![
                Span::styled("  Updated     ", Style::default().fg(Color::DarkGray)),
                Span::raw(&repo.last_updated),
            ]),
        ]
    } else {
        vec![Line::from("  No repository selected")]
    };

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" Info ")
            .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(paragraph, area);
}
