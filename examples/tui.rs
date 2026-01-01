use std::io::{self, BufRead};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, Wrap,
};
use ratatui::{Frame, Terminal};

#[derive(Clone, Copy)]
enum ExampleVariant {
    Multiple(&'static [&'static str]),
}

struct Example {
    name: &'static str,
    description: &'static str,
    variants: ExampleVariant,
}

impl Example {
    const fn new(name: &'static str, description: &'static str, variants: ExampleVariant) -> Self {
        Self {
            name,
            description,
            variants,
        }
    }
}

const EXAMPLES: &[Example] = &[
    Example::new(
        "01_basics",
        "Basic workflow execution (hello, chain, context, sleep)",
        ExampleVariant::Multiple(&["hello", "chain", "context", "sleep"]),
    ),
    Example::new(
        "02_error_handling",
        "Error handling strategies (flaky, fatal, override)",
        ExampleVariant::Multiple(&["flaky", "fatal", "override"]),
    ),
    Example::new(
        "03_scheduling",
        "Time-based scheduling (cron, sleep, catchup)",
        ExampleVariant::Multiple(&["cron", "sleep", "catchup"]),
    ),
    Example::new(
        "04_concurrency",
        "Concurrency control (local, multi-worker)",
        ExampleVariant::Multiple(&["local", "multi"]),
    ),
    Example::new(
        "05_fan_out_in",
        "Parallel execution patterns (static fan-out, map-reduce)",
        ExampleVariant::Multiple(&["static", "mapreduce"]),
    ),
    Example::new(
        "06_long_running",
        "Long-running workflows (polling, timeout)",
        ExampleVariant::Multiple(&["poll", "timeout"]),
    ),
    Example::new(
        "07_idempotency",
        "Idempotency guarantees (external ID, memoization)",
        ExampleVariant::Multiple(&["external", "memo"]),
    ),
    Example::new(
        "08_saga_pattern",
        "Compensating transactions (saga, partial)",
        ExampleVariant::Multiple(&["saga", "partial"]),
    ),
    Example::new(
        "09_data_pipeline",
        "Data processing (transform, large payloads)",
        ExampleVariant::Multiple(&["transform", "large"]),
    ),
    Example::new(
        "10_multi_queue",
        "Multi-queue patterns (priority, namespace)",
        ExampleVariant::Multiple(&["priority", "namespace"]),
    ),
];

enum AppState {
    ExampleList,
    VariantList { example_index: usize },
    Running,
}

struct App {
    state: AppState,
    selected_example: usize,
    selected_variant: usize,
    status_message: String,
    log_output: Arc<Mutex<String>>,
    scroll_offset: usize,
    is_running: bool,
    run_start_time: Option<Instant>,
}

impl App {
    fn new() -> Self {
        Self {
            state: AppState::ExampleList,
            selected_example: 0,
            selected_variant: 0,
            status_message: "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string(),
            log_output: Arc::new(Mutex::new(String::new())),
            scroll_offset: 0,
            is_running: false,
            run_start_time: None,
        }
    }

    fn next_example(&mut self) {
        if self.selected_example < EXAMPLES.len() - 1 {
            self.selected_example += 1;
        }
    }

    fn previous_example(&mut self) {
        if self.selected_example > 0 {
            self.selected_example -= 1;
        }
    }

    fn next_variant(&mut self) {
        let example = &EXAMPLES[self.selected_example];
        let ExampleVariant::Multiple(variants) = example.variants;
        if self.selected_variant < variants.len() - 1 {
            self.selected_variant += 1;
        }
    }

    fn previous_variant(&mut self) {
        if self.selected_variant > 0 {
            self.selected_variant -= 1;
        }
    }

    fn select_example(&mut self) {
        let example = &EXAMPLES[self.selected_example];
        match example.variants {
            ExampleVariant::Multiple(_) => {
                self.state = AppState::VariantList {
                    example_index: self.selected_example,
                };
                self.selected_variant = 0;
                self.status_message = "Select a variant, Enter to run, 'q' to go back".to_string();
            }
        }
    }

    fn select_variant(&mut self) {
        if let AppState::VariantList { example_index } = self.state {
            let example = &EXAMPLES[example_index];
            let ExampleVariant::Multiple(variants) = example.variants;
            let variant = variants[self.selected_variant];
            self.run_example(example.name, variant);
        }
    }

    fn go_back(&mut self) {
        if matches!(self.state, AppState::VariantList { .. }) {
            self.state = AppState::ExampleList;
            self.status_message = "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string();
        }
    }

    fn run_example(&mut self, name: &'static str, variant: &str) {
        self.state = AppState::Running;
        self.status_message = format!("Running {} {}... (Press any key to stop)", name, variant);
        *self.log_output.lock().unwrap() = String::new();
        self.scroll_offset = 0;
        self.is_running = true;
        self.run_start_time = Some(Instant::now());

        let log_output = Arc::clone(&self.log_output);
        let is_running = Arc::new(Mutex::new(true));
        let variant = variant.to_string(); // Clone the variant string

        // Spawn a thread to run the command and capture output
        thread::spawn(move || {
            let output = if variant.is_empty() {
                Command::new("cargo")
                    .args(["run", "--example", name])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            } else {
                Command::new("cargo")
                    .args(["run", "--example", name, "--", &variant])
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            };

            if let Ok(mut child) = output {
                // Read stdout line by line
                #[allow(clippy::items_after_statements)]
                if let Some(stdout) = child.stdout.take() {
                    let reader = std::io::BufReader::new(stdout);
                    for line in reader.lines().map_while(Result::ok) {
                        let mut output = log_output.lock().unwrap();
                        output.push_str(&line);
                        output.push('\n');
                        drop(output);
                    }
                }

                // Wait for process to complete
                if let Ok(_status) = child.wait() {
                    *is_running.lock().unwrap() = false;
                }
            }
        });

        // Store the running flag in a place where the main loop can check it
        // For simplicity, we'll use a timeout-based approach in the main loop
    }
}

fn draw_example_list(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.area());

    // Header
    let header = Paragraph::new("Kagzi Examples Runner")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Example list
    let items: Vec<ListItem> = EXAMPLES
        .iter()
        .enumerate()
        .map(|(i, example)| {
            let style = if i == app.selected_example {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(vec![
                Line::from(Span::styled(example.name, style)),
                Line::from(Span::styled(
                    example.description,
                    Style::default().fg(Color::Gray),
                )),
            ])
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().title("Examples").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(list, chunks[1]);

    // Status bar
    let status = Paragraph::new(app.status_message.as_str())
        .style(Style::default().fg(Color::Green))
        .block(Block::default().borders(Borders::ALL))
        .wrap(Wrap { trim: true });
    f.render_widget(status, chunks[2]);
}

fn draw_variant_list(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.area());

    if let AppState::VariantList { example_index } = app.state {
        let example = &EXAMPLES[example_index];

        // Header
        let header = Paragraph::new(format!("Variants for: {}", example.name))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(header, chunks[0]);

        // Variant list
        let ExampleVariant::Multiple(variants) = example.variants;
        let items: Vec<ListItem> = variants
            .iter()
            .enumerate()
            .map(|(i, variant)| {
                let style = if i == app.selected_variant {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(Span::styled(*variant, style))
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().title("Variants").borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::BOLD));
        f.render_widget(list, chunks[1]);

        // Status bar
        let status = Paragraph::new(app.status_message.as_str())
            .style(Style::default().fg(Color::Green))
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: true });
        f.render_widget(status, chunks[2]);
    }
}

fn draw_running(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(f.area());

    // Status
    let status = Paragraph::new(app.status_message.as_str())
        .style(
            Style::default()
                .fg(if app.is_running {
                    Color::Yellow
                } else {
                    Color::Green
                })
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(status, chunks[0]);

    // Log output with scrolling
    let log_output = app.log_output.lock().unwrap();
    let log_text: Vec<Line> = log_output
        .lines()
        .skip(app.scroll_offset)
        .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::White))))
        .collect();

    let log = Paragraph::new(log_text)
        .block(
            Block::default()
                .title(if app.is_running {
                    "Output (Running...)"
                } else {
                    "Output (Completed)"
                })
                .borders(Borders::ALL),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(log, chunks[1]);

    // Scrollbar
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
    let line_count = log_output.lines().count();
    let mut scrollbar_state =
        ratatui::widgets::ScrollbarState::new(app.scroll_offset).content_length(line_count);
    let scrollbar_area = chunks[1].inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    f.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    drop(log_output); // Release the lock

    // Help bar
    let help_text = if app.is_running {
        "Press any key to stop | ↑↓: Scroll output"
    } else {
        "Press Enter to return | ↑↓: Scroll output | q: Quit"
    };
    let help = Paragraph::new(help_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(help, chunks[2]);

    // Auto-scroll to bottom if running
    if app.is_running {
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
        AppState::Running => draw_running(f, app),
    }
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

        if event::poll(Duration::from_millis(100))? {
            let Ok(Event::Key(key)) = event::read() else {
                continue;
            };
            match app.state {
                AppState::ExampleList => match key.code {
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;
                        return Ok(());
                    }
                    KeyCode::Down | KeyCode::Char('j') => app.next_example(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous_example(),
                    KeyCode::Enter => app.select_example(),
                    _ => {}
                },
                AppState::VariantList { .. } => match key.code {
                    KeyCode::Char('q') => app.go_back(),
                    KeyCode::Down | KeyCode::Char('j') => app.next_variant(),
                    KeyCode::Up | KeyCode::Char('k') => app.previous_variant(),
                    KeyCode::Enter => app.select_variant(),
                    _ => {}
                },
                AppState::Running => match key.code {
                    KeyCode::Char('q') => {
                        disable_raw_mode()?;
                        execute!(
                            terminal.backend_mut(),
                            LeaveAlternateScreen,
                            DisableMouseCapture
                        )?;
                        terminal.show_cursor()?;
                        return Ok(());
                    }
                    KeyCode::Enter => {
                        if !app.is_running {
                            app.state = AppState::ExampleList;
                            app.status_message =
                                "Use ↑↓ to navigate, Enter to select, 'q' to quit".to_string();
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if !app.is_running {
                            app.scroll_offset = app.scroll_offset.saturating_add(1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if !app.is_running {
                            app.scroll_offset = app.scroll_offset.saturating_sub(1);
                        }
                    }
                    _ => {
                        // Any other key stops the running process
                        if app.is_running {
                            app.is_running = false;
                            app.status_message =
                                "Stopped by user - Press Enter to return".to_string();
                        }
                    }
                },
            }
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    run_app()
}
