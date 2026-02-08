use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{fs, thread};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Wrap,
};
use ratatui::{Frame, Terminal};

#[derive(Clone, Debug)]
struct Example {
    name: String,
    description: String,
    variants: Vec<String>,
}

impl Example {
    fn new(name: String, description: String, variants: Vec<String>) -> Self {
        Self {
            name,
            description,
            variants,
        }
    }

    fn variants(&self) -> &[String] {
        &self.variants
    }
}

fn discover_examples() -> Vec<Example> {
    let examples_dir = Path::new(file!()).parent().expect("examples directory");
    let mut examples = BTreeMap::new();

    let entries = fs::read_dir(examples_dir)
        .unwrap_or_else(|e| panic!("Failed to read examples directory: {}", e));

    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Only process directories matching pattern NN_name
        if !path.is_dir() || !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }

        let main_rs = path.join("main.rs");
        if !main_rs.exists() {
            continue;
        }

        // Extract variants from main.rs
        let variants = extract_variants(&main_rs);
        let description = extract_description(&main_rs).unwrap_or_else(|| {
            name.split_once('_')
                .map(|(_, desc)| desc.replace('_', " "))
                .unwrap_or_else(|| name.to_string())
        });

        examples.insert(
            name.to_string(),
            Example::new(name.to_string(), description, variants),
        );
    }

    examples.into_values().collect()
}

fn extract_variants(main_rs: &Path) -> Vec<String> {
    let content = fs::read_to_string(main_rs).unwrap_or_default();
    let mut variants = Vec::new();

    // Look for match statements with variant strings
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('"') && line.ends_with('"') {
            if let Some(_variant) = line.trim_matches('"').strip_prefix("=>") {
                continue;
            }
            if let Some(variant) = line.split('"').nth(1)
                && !variant.is_empty()
                && variant.len() < 30
            {
                variants.push(variant.to_string());
            }
        }
    }

    // If no variants found, default to empty
    if variants.is_empty() {
        return vec![];
    }

    // Remove duplicates while preserving order
    let mut seen = std::collections::HashSet::new();
    variants.retain(|v| seen.insert(v.clone()));

    variants
}

fn extract_description(main_rs: &Path) -> Option<String> {
    let content = fs::read_to_string(main_rs).ok()?;
    content.lines().find_map(|line| {
        line.strip_prefix("//")
            .map(|stripped| stripped.trim().to_string())
    })
}

enum AppState {
    ExampleList,
    VariantList { example_index: usize },
    Running { is_running: bool },
}

struct App {
    examples: Vec<Example>,
    state: AppState,
    selected_example: usize,
    selected_variant: usize,
    status_message: String,
    log_output: Arc<Mutex<String>>,
    scroll_offset: usize,
}

impl App {
    fn new() -> Self {
        let examples = discover_examples();
        Self {
            examples,
            state: AppState::ExampleList,
            selected_example: 0,
            selected_variant: 0,
            status_message: "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string(),
            log_output: Arc::new(Mutex::new(String::new())),
            scroll_offset: 0,
        }
    }

    fn current_variants(&self) -> &[String] {
        self.examples[self.selected_example].variants()
    }

    fn is_running(&self) -> bool {
        matches!(self.state, AppState::Running { is_running: true })
    }

    fn navigate_list(&mut self, direction: i32, max_index: usize, current: usize) -> usize {
        (current as i32 + direction).clamp(0, max_index as i32 - 1) as usize
    }

    fn select_example(&mut self) {
        self.state = AppState::VariantList {
            example_index: self.selected_example,
        };
        self.selected_variant = 0;
        self.status_message = "Select a variant, Enter to run, 'q' to go back".to_string();
    }

    fn select_variant(&mut self) {
        if let AppState::VariantList { example_index } = self.state {
            let (name, variant) = {
                let example = &self.examples[example_index];
                let variant = example.variants()[self.selected_variant].clone();
                (example.name.clone(), variant)
            };
            self.run_example(&name, &variant);
        }
    }

    fn go_back(&mut self) {
        self.state = AppState::ExampleList;
        self.status_message = "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string();
    }

    fn run_example(&mut self, name: &str, variant: &str) {
        self.state = AppState::Running { is_running: true };
        self.status_message = format!("Running {} {}... (Press any key to stop)", name, variant);
        *self.log_output.lock().unwrap() = String::new();
        self.scroll_offset = 0;

        let log_output = Arc::clone(&self.log_output);
        let name = name.to_string();
        let variant = variant.to_string();

        thread::spawn(move || {
            let result = if variant.is_empty() {
                Command::new("cargo")
                    .args(["run", "--example", &name])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            } else {
                Command::new("cargo")
                    .args(["run", "--example", &name, "--", &variant])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            };

            if let Ok(mut child) = result {
                if let Some(stdout) = child.stdout.take() {
                    let reader = BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let mut output = log_output.lock().unwrap();
                        output.push_str(&line);
                        output.push('\n');
                    }
                }
                let _ = child.wait();
            }
        });
    }

    fn stop_running(&mut self) {
        if let AppState::Running { .. } = self.state {
            self.state = AppState::Running { is_running: false };
            self.status_message = "Stopped by user - Press Enter to return".to_string();
        }
    }

    fn scroll_output(&mut self, direction: i32) {
        self.scroll_offset = (self.scroll_offset as i32 + direction).max(0) as usize;
    }
}

fn layout_chunks(area: ratatui::layout::Rect) -> Vec<ratatui::layout::Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area)
        .to_vec()
}

fn draw_header(title: &str, f: &mut Frame, area: ratatui::layout::Rect) {
    let header = Paragraph::new(title)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn draw_status(message: &str, f: &mut Frame, area: ratatui::layout::Rect) {
    let status = Paragraph::new(message)
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    f.render_widget(status, area);
}

fn selected_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn draw_example_list(f: &mut Frame, app: &App) {
    let chunks = layout_chunks(f.area());

    draw_header("Kagzi Examples Runner", f, chunks[0]);

    let items: Vec<ListItem> = app
        .examples
        .iter()
        .enumerate()
        .map(|(i, example)| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    &example.name,
                    selected_style(i == app.selected_example),
                )),
                Line::from(Span::styled(
                    &example.description,
                    Style::default().fg(Color::Gray),
                )),
            ])
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title("Examples").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(list, chunks[1]);

    draw_status(&app.status_message, f, chunks[2]);
}

fn draw_variant_list(f: &mut Frame, app: &App) {
    let chunks = layout_chunks(f.area());

    if let AppState::VariantList { example_index } = app.state {
        let example = &app.examples[example_index];

        draw_header(&format!("Variants for: {}", example.name), f, chunks[0]);

        let items: Vec<ListItem> = example
            .variants()
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                ListItem::new(Span::styled(
                    variant.as_str(),
                    selected_style(i == app.selected_variant),
                ))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Variants").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        f.render_widget(list, chunks[1]);

        draw_status(&app.status_message, f, chunks[2]);
    }
}

fn draw_running(f: &mut Frame, app: &mut App) {
    let chunks = layout_chunks(f.area());
    let is_running = app.is_running();

    let status = Paragraph::new(app.status_message.as_str())
        .style(
            Style::default()
                .fg(if is_running {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[0]);

    let log_output = app.log_output.lock().unwrap();
    let log_text: Vec<Line> = log_output
        .lines()
        .skip(app.scroll_offset)
        .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
        .collect();

    let log = Paragraph::new(log_text)
        .block(
            Block::default()
                .title(if is_running {
                    "Output (Running...)"
                } else {
                    "Output (Completed)"
                })
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(log, chunks[1]);

    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    let line_count = log_output.lines().count();
    let mut scrollbar_state = ScrollbarState::new(app.scroll_offset).content_length(line_count);
    f.render_stateful_widget(
        scrollbar,
        chunks[1].inner(Margin {
            vertical: 0,
            horizontal: 1,
        }),
        &mut scrollbar_state,
    );
    drop(log_output);

    let help_text = if is_running {
        "Press any key to stop | ↑↓: Scroll output"
    } else {
        "Press Enter to return | ↑↓: Scroll output | q: Quit"
    };
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(help, chunks[2]);

    if is_running {
        let log_output = app.log_output.lock().unwrap();
        let line_count = log_output.lines().count();
        let visible_lines = chunks[1].height.saturating_sub(2) as usize;
        if line_count > visible_lines {
            app.scroll_offset = line_count.saturating_sub(visible_lines);
        }
    }
}

fn draw_ui(f: &mut Frame, app: &mut App) {
    match app.state {
        AppState::ExampleList => draw_example_list(f, app),
        AppState::VariantList { .. } => draw_variant_list(f, app),
        AppState::Running { .. } => draw_running(f, app),
    }
}

fn quit(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_input_example_list(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') => return true,
        KeyCode::Down | KeyCode::Char('j') => {
            app.selected_example = app.navigate_list(1, app.examples.len(), app.selected_example)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected_example = app.navigate_list(-1, app.examples.len(), app.selected_example)
        }
        KeyCode::Enter => app.select_example(),
        _ => {}
    }
    false
}

fn handle_input_variant_list(app: &mut App, key: KeyCode) -> bool {
    let len = app.current_variants().len();
    match key {
        KeyCode::Char('q') => app.go_back(),
        KeyCode::Down | KeyCode::Char('j') => {
            app.selected_variant = app.navigate_list(1, len, app.selected_variant)
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.selected_variant = app.navigate_list(-1, len, app.selected_variant)
        }
        KeyCode::Enter => app.select_variant(),
        _ => {}
    }
    false
}

fn handle_input_running(app: &mut App, key: KeyCode) -> bool {
    let is_running = app.is_running();
    match key {
        KeyCode::Char('q') => return true,
        KeyCode::Enter if !is_running => {
            app.state = AppState::ExampleList;
            app.status_message = "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string();
        }
        KeyCode::Down | KeyCode::Char('j') if !is_running => app.scroll_output(1),
        KeyCode::Up | KeyCode::Char('k') if !is_running => app.scroll_output(-1),
        _ if is_running => app.stop_running(),
        _ => {}
    }
    false
}

fn run_app() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new();

    loop {
        terminal.draw(|f| draw_ui(f, &mut app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Ok(Event::Key(key)) = event::read() else {
            continue;
        };

        let should_quit = match app.state {
            AppState::ExampleList => handle_input_example_list(&mut app, key.code),
            AppState::VariantList { .. } => handle_input_variant_list(&mut app, key.code),
            AppState::Running { .. } => handle_input_running(&mut app, key.code),
        };

        if should_quit {
            return quit(&mut terminal);
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    run_app()
}
