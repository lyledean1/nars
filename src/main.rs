mod ollama;
mod parser;
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::ollama::OllamaClient;
use crate::parser::{parse_code_output, ParsedCode};
use anyhow::{anyhow, Result};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt; // Add this import
use std::fs::OpenOptions;
use std::io::{Stdout, Write};
use std::{env, error::Error, fs, io};
use std::time::Instant;
use tokio::task;
use tree_sitter::{Language, Parser, Tree};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Line},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

struct Editor {
    content: String,
    cursor_position: usize,
    scroll_offset: usize,
    parser: Parser,
    tree: Option<Tree>,
    filename: Option<String>,
    status_message: String,
    status_time: std::time::Instant,
    prediction_rx: mpsc::Receiver<String>,
    current_prediction: Option<String>,
    prediction_start_position: Option<usize>,
    needs_redraw: bool,
}

impl Editor {
    fn new() -> (Self, mpsc::Sender<String>) {
        let (prediction_tx, prediction_rx) = mpsc::channel(32);
        let mut parser = Parser::new();
        parser
            .set_language(tree_sitter_rust())
            .expect("Error loading Rust grammar");

        (
            Editor {
                content: String::new(),
                cursor_position: 0,
                scroll_offset: 0,
                parser,
                tree: None,
                filename: None,
                status_message: String::new(),
                status_time: std::time::Instant::now(),
                current_prediction: None,
                prediction_start_position: None,
                prediction_rx,
                needs_redraw: false,
            },
            prediction_tx,
        )
    }

    fn save_file(&self) -> Result<()> {
        if let Some(path) = &self.filename {
            fs::write(path, &self.content)?;
            return Ok(());
        }
        Err(anyhow!("No filename specified"))
    }
    fn load_file(&mut self, path: String) -> Result<()> {
        self.content = fs::read_to_string(&path)?;
        self.filename = Some(path);
        self.cursor_position = 0;
        self.scroll_offset = 0;
        self.update_syntax_tree();
        Ok(())
    }

    fn highlight_syntax(&self, window_height: usize) -> Vec<Line> {
        let mut result = Vec::new();

        // Split current content into lines
        let lines: Vec<&str> = self.content.split('\n').collect();

        // Get visible lines
        let visible_lines = lines.iter()
            .skip(self.scroll_offset)
            .take(window_height)
            .collect::<Vec<_>>();

        // Calculate prediction content if it exists
        let (prediction_lines, prediction_start_line, cursor_column) = if let (Some(pred), Some(start_pos)) =
            (&self.current_prediction, self.prediction_start_position)
        {
            // Get the line where prediction starts
            let start_line = self.content[..start_pos].chars().filter(|&c| c == '\n').count();

            // Calculate cursor column position within the line
            let line_start = self.content[..start_pos].rfind('\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);
            let cursor_column = start_pos - line_start;

            // Get the current line's content up to the cursor
            let current_line_prefix = &self.content[line_start..start_pos];

            // Find where the current line ends
            let line_end = self.content[start_pos..].find('\n')
                .map(|pos| start_pos + pos)
                .unwrap_or(self.content.len());

            // Get content after the current line
            let post_content = if line_end < self.content.len() {
                &self.content[line_end..]
            } else {
                ""
            };

            // Create prediction by combining current line prefix with prediction
            let full_content = format!("{}{}{}",
                                       current_line_prefix,
                                       pred,
                                       post_content
            );

            // Split into lines
            let pred_lines = full_content.split('\n').map(|s| s.to_string()).collect::<Vec<_>>();

            (Some(pred_lines), Some(start_line), Some(cursor_column))
        } else {
            (None, None, None)
        };

        if let Some(tree) = &self.tree {
            let root = tree.root_node();

            for (line_idx, &line) in visible_lines.iter().enumerate() {
                let absolute_line_idx = line_idx + self.scroll_offset;

                // Calculate line start and end positions in the content
                let line_start = if absolute_line_idx == 0 {
                    0
                } else {
                    lines[..absolute_line_idx].join("\n").len() + 1
                };
                let line_end = line_start + line.len();

                // Create spans for syntax highlighting
                let mut style_spans = Vec::new();
                let mut cursor = root.walk();
                let mut did_visit = false;
                cursor.reset(root);

                // Walk the syntax tree to find nodes that intersect with this line
                loop {
                    let node = cursor.node();
                    let start_byte = node.start_byte();
                    let end_byte = node.end_byte();

                    if start_byte < line_end && end_byte > line_start {
                        let style = match node.kind() {
                            // Keywords
                            "use" | "struct" | "enum" | "impl" | "fn" | "pub" | "mod" | "let"
                            | "mut" | "self" | "match" | "if" | "else" | "for" | "while"
                            | "loop" | "return" | "break" | "continue" | "const" | "static"
                            | "type" | "where" | "unsafe" | "async" | "await" | "move" | "ref" => {
                                Some(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                            }

                            // Module-level items
                            "use_declaration" | "mod_item" => {
                                Some(Style::default().fg(Color::Cyan))
                            }

                            // Types
                            "type_identifier" | "primitive_type" => {
                                Some(Style::default().fg(Color::Green))
                            }

                            // Functions
                            "function_item" => {
                                let name_node = node.child_by_field_name("name");
                                if let Some(name) = name_node {
                                    if start_byte == name.start_byte() && end_byte == name.end_byte() {
                                        Some(Style::default()
                                            .fg(Color::Blue)
                                            .add_modifier(Modifier::BOLD))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }

                            // Variables and identifiers
                            "identifier" => Some(Style::default().fg(Color::White)),

                            // Literals
                            "string_literal" | "raw_string_literal" => {
                                Some(Style::default().fg(Color::Yellow))
                            }
                            "integer_literal" | "float_literal" => {
                                Some(Style::default().fg(Color::Magenta))
                            }

                            // Comments
                            "line_comment" | "block_comment" => Some(
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC),
                            ),

                            // Operators and punctuation
                            ":" | "::" | "->" | "=>" | "=" | "+" | "-" | "*" | "/" | "%" | "&"
                            | "|" | "^" | "!" | "." => Some(Style::default().fg(Color::Yellow)),

                            _ => None,
                        };

                        if let Some(style) = style {
                            let node_start = start_byte.max(line_start);
                            let node_end = end_byte.min(line_end);
                            style_spans.push((node_start, node_end, style));
                        }
                    }

                    if !did_visit {
                        if cursor.goto_first_child() {
                            did_visit = false;
                            continue;
                        }
                        did_visit = true;
                    }

                    if cursor.goto_next_sibling() {
                        did_visit = false;
                        continue;
                    }

                    if !cursor.goto_parent() {
                        break;
                    }
                    did_visit = true;
                }

                // Create final spans for the line
                style_spans.sort_by_key(|&(start, _, _)| start);
                let mut spans = Vec::new();
                let mut current_pos = line_start;

                // Add styled spans
                for (start, end, style) in style_spans {
                    if start > current_pos {
                        spans.push(Span::raw(self.content[current_pos..start].to_string()));
                    }
                    if start >= current_pos {
                        spans.push(Span::styled(
                            self.content[start..end].to_string(),
                            style
                        ));
                        current_pos = end;
                    }
                }

                // Add any remaining unstyled text
                if current_pos < line_end {
                    spans.push(Span::raw(self.content[current_pos..line_end].to_string()));
                }

                // Handle prediction overlay with cursor position awareness
                if let (Some(pred_lines), Some(start_line), Some(cursor_col)) =
                    (&prediction_lines, prediction_start_line, cursor_column)
                {
                    if absolute_line_idx == start_line {
                        // This is the line where prediction starts
                        let mut new_spans = Vec::new();

                        // Keep the content up to cursor
                        let line_content = line.to_string();
                        if cursor_col > 0 {
                            new_spans.push(Span::raw(line_content[..cursor_col].to_string()));
                        }

                        if let Some(pred_line) = pred_lines.get(absolute_line_idx) {
                            // Add prediction after cursor
                            if cursor_col < pred_line.len() {
                                new_spans.push(Span::styled(
                                    pred_line[cursor_col..].to_string(),
                                    Style::default()
                                        .fg(Color::DarkGray)
                                        .add_modifier(Modifier::ITALIC),
                                ));
                            }
                        }

                        spans = new_spans;
                    } else if absolute_line_idx > start_line && absolute_line_idx < pred_lines.len() {
                        // These are additional prediction lines
                        spans = vec![Span::styled(
                            pred_lines[absolute_line_idx].to_string(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        )];
                    }
                }

                result.push(Line::from(spans));
            }

            // Add any additional prediction lines that extend beyond the current content
            if let (Some(pred_lines), Some(start_line), _) = (&prediction_lines, prediction_start_line, cursor_column) {
                let current_visible_end = self.scroll_offset + visible_lines.len();
                for idx in current_visible_end..pred_lines.len() {
                    if idx - self.scroll_offset >= window_height {
                        break;
                    }
                    result.push(Line::from(vec![
                        Span::styled(
                            pred_lines[idx].to_string(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        )
                    ]));
                }
            }
        } else {
            // No syntax tree - handle plain text with predictions
            let max_lines = if let Some(pred_lines) = &prediction_lines {
                pred_lines.len().max(lines.len())
            } else {
                lines.len()
            };

            for line_idx in self.scroll_offset..max_lines.min(self.scroll_offset + window_height) {
                let mut spans = Vec::new();

                if let (Some(pred_lines), Some(start_line), Some(cursor_col)) =
                    (&prediction_lines, prediction_start_line, cursor_column)
                {
                    if line_idx == start_line {
                        // Line where prediction starts
                        if line_idx < lines.len() {
                            let line = lines[line_idx];
                            if cursor_col > 0 {
                                spans.push(Span::raw(line[..cursor_col.min(line.len())].to_string()));
                            }

                            // Add prediction after cursor
                            if let Some(pred_line) = pred_lines.get(line_idx) {
                                if cursor_col < pred_line.len() {
                                    spans.push(Span::styled(
                                        pred_line[cursor_col..].to_string(),
                                        Style::default()
                                            .fg(Color::DarkGray)
                                            .add_modifier(Modifier::ITALIC),
                                    ));
                                }
                            }
                        }
                    } else if line_idx > start_line && line_idx < pred_lines.len() {
                        // Additional prediction lines
                        spans.push(Span::styled(
                            pred_lines[line_idx].to_string(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    } else if line_idx < lines.len() {
                        spans.push(Span::raw(lines[line_idx].to_string()));
                    }
                } else if line_idx < lines.len() {
                    spans.push(Span::raw(lines[line_idx].to_string()));
                }

                result.push(Line::from(spans));
            }
        }

        result
    }
    fn accept_prediction(&mut self) {
        if let (Some(pred), Some(start_pos)) = (
            self.current_prediction.take(),
            self.prediction_start_position.take(),
        ) {
            // Get the line start position
            let line_start = self.content[..start_pos]
                .rfind('\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);

            // Get the line end position
            let line_end = self.content[line_start..]
                .find('\n')
                .map(|pos| line_start + pos)
                .unwrap_or(self.content.len());

            // Replace the entire line content with the prediction
            self.content.replace_range(line_start..line_end, &pred);

            // Move cursor to end of prediction
            self.cursor_position = line_start + pred.len();

            // Update syntax highlighting
            self.update_syntax_tree();
            self.current_prediction = None;

            log_to_file(&format!("accepted prediction: {}", pred));
        }
    }

    // Modify get_latest_prediction to update the stored prediction
    fn get_latest_prediction(&mut self) -> bool {
        let mut got_any = false;
        while let Ok(pred) = self.prediction_rx.try_recv() {
            log_to_file(format!("got prediction from channel {}", pred).as_str());
            self.current_prediction = Some(pred);
            self.prediction_start_position = Some(self.cursor_position);
            self.needs_redraw = true;
            got_any = true;
        }
        got_any
    }
    fn get_current_line(&self) -> usize {
        self.content[..self.cursor_position]
            .chars()
            .filter(|&c| c == '\n' || c == '\t')
            .count()
    }

    fn ensure_cursor_visible(&mut self, window_height: usize) {
        let current_line = self.get_current_line();

        // If cursor is above visible area, scroll up
        if current_line < self.scroll_offset {
            self.scroll_offset = current_line;
        }

        // If cursor is below visible area, scroll down
        if current_line >= self.scroll_offset + window_height {
            self.scroll_offset = current_line - window_height + 1;
        }
    }

    fn get_current_line_content(&self) -> String {
        let line_start = self.content[..self.cursor_position]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);

        let line_end = self.content[self.cursor_position..]
            .find('\n')
            .map(|pos| self.cursor_position + pos)
            .unwrap_or(self.content.len());

        self.content[line_start..line_end].to_string()
    }

    fn insert_char(&mut self, c: char, cursor_position: usize) {
        if c == '\n' {
            self.current_prediction = None;
            self.prediction_start_position = None;
        }
        if c == '\t' {
            // Insert 4 spaces instead of a tab character
            for _ in 0..4 {
                self.content.insert(self.cursor_position, ' ');
                self.cursor_position += 1;
            }
        } else {
            self.content.insert(self.cursor_position, c);
            self.cursor_position += cursor_position;
        }
        self.update_syntax_tree();
    }

    fn delete_char(&mut self) {
        if self.cursor_position > 0 {
            self.content.remove(self.cursor_position - 1);
            self.cursor_position -= 1;
            self.update_syntax_tree();
        }
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_position < self.content.len() {
            self.cursor_position += 1;
        }
    }

    fn move_cursor_up(&mut self) {
        self.current_prediction = None;
        self.prediction_start_position = None;
        // Get current line's start position
        let current_line_start = self.content[..self.cursor_position]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);

        // Get position within current line
        let line_offset = self.cursor_position - current_line_start;

        // Find start of previous line
        if let Some(prev_line_start) = self.content[..current_line_start.saturating_sub(1)]
            .rfind('\n')
            .map(|pos| pos + 1)
        {
            // Get length of previous line
            let prev_line_length = self.content
                [prev_line_start..current_line_start.saturating_sub(1)]
                .chars()
                .count();

            // Move cursor to same offset in previous line, or end of line if shorter
            self.cursor_position = prev_line_start + line_offset.min(prev_line_length);
        } else if current_line_start > 0 {
            // We're on the second line, move to first line
            self.cursor_position = line_offset.min(current_line_start - 1);
        }
    }

    fn move_cursor_down(&mut self) {
        let current_line_start = self.content[..self.cursor_position]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);
        let current_line_end = self.content[self.cursor_position..]
            .find('\n')
            .map(|pos| self.cursor_position + pos)
            .unwrap_or(self.content.len());

        let line_offset = self.cursor_position - current_line_start;
        if self.content.len() <= current_line_end + 1 {
            return;
        }
        if let Some(next_line_end) = self.content[current_line_end + 1..]
            .find('\n')
            .map(|pos| current_line_end + 1 + pos)
            .or_else(|| {
                if current_line_end < self.content.len() {
                    Some(self.content.len())
                } else {
                    None
                }
            })
        {
            let next_line_length = next_line_end - (current_line_end + 1);
            self.cursor_position = (current_line_end + 1) + line_offset.min(next_line_length);
        }
    }

    fn update_syntax_tree(&mut self) {
        self.tree = self.parser.parse(&self.content, self.tree.as_ref());
        if let Some(tree) = &self.tree {
            log_to_file("Syntax tree generated successfully");
            let root = tree.root_node();
            log_to_file(&format!("Root node type: {}", root.kind()));

            // Log first few nodes for debugging
            let mut cursor = root.walk();
            let mut count = 0;
            while cursor.goto_first_child() || cursor.goto_next_sibling() {
                let node = cursor.node();
                log_to_file(&format!(
                    "Node {}: kind={}, text={:?}",
                    count,
                    node.kind(),
                    self.content[node.start_byte()..node.end_byte()].to_string()
                ));
                count += 1;
                if count >= 10 {
                    break;
                } // Log first 10 nodes only
            }
        } else {
            log_to_file("Failed to generate syntax tree");
        }
    }
}

fn tree_sitter_rust() -> Language {
    unsafe {
        extern "C" {
            fn tree_sitter_rust() -> Language;
        }
        tree_sitter_rust()
    }
}

async fn run_editor(client: Arc<OllamaClient>, filename: Option<String>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (mut editor, prediction_tx) = Editor::new();

    let mut status_message = String::new();
    let mut status_time = std::time::Instant::now();

    // Load file if specified
    if let Some(path) = filename {
        editor.load_file(path)?;
    }

    loop {
        let window_height = terminal.size()?.height as usize - 2; // Account for borders
        editor.ensure_cursor_visible(window_height);

        redraw_editor(&mut terminal, &mut editor, &mut status_message, status_time)?;
        if let Event::Key(key) = event::read()? {
            editor.get_latest_prediction();
            match key.code {
                KeyCode::Char('s') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                    match editor.save_file() {
                        Ok(_) => {
                            status_message = String::from("File saved successfully!");
                            status_time = std::time::Instant::now();
                        }
                        Err(e) => {
                            status_message = format!("Error saving file: {}", e);
                            status_time = std::time::Instant::now();
                        }
                    }
                }
                KeyCode::Tab => {
                    if editor.current_prediction.is_some() {
                        editor.accept_prediction();
                    } else {
                        let content = editor.get_current_line_content();
                        stream_prediction_background(
                            client.clone(),
                            content,
                            prediction_tx.clone(),
                        )
                        .await;
                    }
                }
                KeyCode::Esc => {
                    editor.current_prediction = None;
                    editor.prediction_start_position = None;
                    break;
                }
                KeyCode::Char(c) => {
                    editor.current_prediction = None;
                    editor.prediction_start_position = None;
                    editor.insert_char(c, 1);
                }
                // KeyCode::Tab => editor.insert_char('\t', 4),
                KeyCode::Enter => editor.insert_char('\n', 1),
                KeyCode::Backspace => editor.delete_char(),
                KeyCode::Left => editor.move_cursor_left(),
                KeyCode::Right => editor.move_cursor_right(),
                KeyCode::Up => editor.move_cursor_up(),
                KeyCode::Down => editor.move_cursor_down(),
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn redraw_editor(terminal: &mut Terminal<CrosstermBackend<Stdout>>, mut editor: &mut Editor, mut status_message: &mut String, mut status_time: Instant) -> Result<()> {
    terminal.draw(|f| {
        editor.get_latest_prediction();
        log_to_file(format!("latest prediction {}", status_message).as_str());
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
            .split(f.size());

        let title = editor
            .filename
            .as_ref()
            .map(|f| format!("edoc - {}", f))
            .unwrap_or_else(|| "edoc".to_string());

        let window_height = chunks[0].height as usize - 2; // Account for borders
        let mut styled_lines = editor.highlight_syntax(window_height);

        // Add cursor indicator
        let current_line_number = editor.content[..editor.cursor_position]
            .chars()
            .filter(|&c| c == '\n')
            .count();

        // Only show cursor if the line is currently visible
        if current_line_number >= editor.scroll_offset
            && current_line_number < editor.scroll_offset + window_height
        {
            if let Some(line) = styled_lines.get_mut(current_line_number - editor.scroll_offset)
            {
                // Calculate cursor position within the line
                let line_start = editor.content[..editor.cursor_position]
                    .rfind('\n')
                    .map(|pos| pos + 1)
                    .unwrap_or(0);
                let cursor_offset = editor.cursor_position - line_start;

                // Create a new list of spans with the cursor
                let mut new_spans = Vec::new();
                let mut current_pos = 0;

                for span in line.spans.iter() {
                    let span_len = span.content.len();
                    if current_pos + span_len > cursor_offset && current_pos <= cursor_offset {
                        // Split this span to insert the cursor
                        let cursor_rel_pos = cursor_offset - current_pos;
                        if cursor_rel_pos > 0 {
                            new_spans.push(Span::styled(
                                span.content[..cursor_rel_pos].to_string(),
                                span.style,
                            ));
                        }
                        // Add the cursor
                        new_spans.push(Span::styled(
                            "|".to_string(),
                            Style::default()
                                .fg(Color::Rgb(169, 183, 198))
                                .add_modifier(Modifier::RAPID_BLINK),
                        ));
                        if cursor_rel_pos < span_len {
                            new_spans.push(Span::styled(
                                span.content[cursor_rel_pos..].to_string(),
                                span.style,
                            ));
                        }
                    } else {
                        new_spans.push(span.clone());
                    }
                    current_pos += span_len;
                }

                // If cursor is at the end of the line
                if cursor_offset >= current_pos {
                    new_spans.push(Span::styled(
                        "|".to_string(),
                        Style::default()
                            .fg(Color::Rgb(169, 183, 198))
                            .add_modifier(Modifier::RAPID_BLINK),
                    ));
                }

                *line = Line::from(new_spans);
            }
        }

        let paragraph = Paragraph::new(styled_lines)
            .block(
                Block::default().borders(Borders::ALL).title(title).style(
                    Style::default()
                        .bg(Color::Rgb(43, 43, 43))
                        .fg(Color::Rgb(169, 183, 198)),
                ),
            )
            .style(Style::default().bg(Color::Rgb(43, 43, 43)));

        f.render_widget(paragraph, chunks[0]);

        // Add status bar
        if !status_message.is_empty()
            && status_time.elapsed() < std::time::Duration::from_secs(5)
        {
            let status_bar = Paragraph::new(Line::from(status_message.as_str()))
                .style(Style::default().fg(Color::White).bg(Color::Black));
            f.render_widget(status_bar, chunks[1]);
        }
    })?;
    Ok(())
}

fn log_to_file(message: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("editor_debug.log")
    {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        if let Err(e) = writeln!(file, "[{}] {}", timestamp, message) {
            eprintln!("Failed to write to log file: {}", e);
        }
    }
}

async fn stream_prediction(
    client: Arc<OllamaClient>,
    prediction_tx: mpsc::Sender<String>,
    line: String,
) -> Result<String> {
    let prompt = format!("Complete the code on this line, returning only the raw code without any formatting, comments, or extra text. Example input: 'let x = '  Example output: 'let x = Some(42);'. Here is the code {}", line);
    log_to_file(&prompt);
    let mut stream = client
        .stream_generate("qwen2.5-coder:7b", prompt.as_str())
        .await?;
    let mut pred = "".to_string();
    let mut output = ParsedCode{
        code: "".to_string(),
        language: None,
        is_complete: false,
    };

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(text) => {
                pred = format!("{}{}", pred, text);
                log_to_file(format!("Next chunk {}", pred).as_str());
                // refactor as this is not needed or return this?
                output = parse_code_output(&pred)?;
                match prediction_tx.send(format!("{}", pred)).await {
                    Ok(_) => {
                        log_to_file(format!("Send pred to channel {}", pred).as_str());

                    }
                    Err(e) => {
                        eprintln!("Failed to send prediction: {}", e);
                    }
                }
            }
            Err(e) => eprintln!("Error: {}", e),
        }
    }
    log_to_file(&pred);
    Ok(output.code)
}

async fn stream_prediction_background(
    client: Arc<OllamaClient>,
    content: String,
    prediction_tx: mpsc::Sender<String>,
) {
    task::spawn(async move {
        match stream_prediction(client, prediction_tx, content).await {
            Err(e) => {
                log_to_file(format!("Prediction error: {}", e.to_string().as_str()).as_str());
            }
            _ => {
            }
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    let client = Arc::new(OllamaClient::new());
    let args: Vec<String> = env::args().collect();
    let filename = args.get(1).cloned();

    run_editor(client, filename).await
}
