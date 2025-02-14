use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{env, error::Error, fs, io};
use tree_sitter::{Language, Parser, Tree};
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::fs::OpenOptions;
use std::io::Write;

struct Editor {
    content: String,
    cursor_position: usize,
    scroll_offset: usize,
    parser: Parser,
    tree: Option<Tree>,
    filename: Option<String>,
}

impl Editor {
    fn new() -> Self {
        let mut parser = Parser::new();
        parser.set_language(tree_sitter_rust())
            .expect("Error loading Rust grammar");

        Editor {
            content: String::new(),
            cursor_position: 0,
            scroll_offset: 0,
            parser,
            tree: None,
            filename: None,
        }
    }

    fn save_file(&self) -> Result<(), Box<dyn Error>> {
        if let Some(path) = &self.filename {
            fs::write(path, &self.content)?;
            Ok(())
        } else {
            Err("No filename specified".into())
        }
    }
    fn load_file(&mut self, path: String) -> Result<(), Box<dyn Error>> {
        self.content = fs::read_to_string(&path)?;
        self.filename = Some(path);
        self.cursor_position = 0;
        self.scroll_offset = 0;
        self.update_syntax_tree();
        Ok(())
    }

    fn get_current_line(&self) -> usize {
        self.content[..self.cursor_position].chars().filter(|&c| c == '\n').count()
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

    fn insert_char(&mut self, c: char) {
        self.content.insert(self.cursor_position, c);
        self.cursor_position += 1;
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
            let prev_line_length = self.content[prev_line_start..current_line_start.saturating_sub(1)]
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
        // Get current line's start and end positions
        let current_line_start = self.content[..self.cursor_position]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);
        let current_line_end = self.content[self.cursor_position..]
            .find('\n')
            .map(|pos| self.cursor_position + pos)
            .unwrap_or(self.content.len());

        // Get position within current line
        let line_offset = self.cursor_position - current_line_start;

        // Find start and end of next line
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
            // Move cursor to same offset in next line, or end of line if shorter
            let next_line_length = next_line_end - (current_line_end + 1);
            self.cursor_position = (current_line_end + 1) + line_offset.min(next_line_length);
        }
    }

    fn update_syntax_tree(&mut self) {
        self.tree = self.parser.parse(&self.content, self.tree.as_ref());

        // Debug: Log tree generation status
        if let Some(tree) = &self.tree {
            log_to_file("Syntax tree generated successfully");
            let root = tree.root_node();
            log_to_file(&format!("Root node type: {}", root.kind()));

            // Log first few nodes for debugging
            let mut cursor = root.walk();
            let mut count = 0;
            while cursor.goto_first_child() || cursor.goto_next_sibling() {
                let node = cursor.node();
                log_to_file(&format!("Node {}: kind={}, text={:?}",
                                     count,
                                     node.kind(),
                                     self.content[node.start_byte()..node.end_byte()].to_string()
                ));
                count += 1;
                if count >= 10 { break; }  // Log first 10 nodes only
            }
        } else {
            log_to_file("Failed to generate syntax tree");
        }
    }

    fn highlight_syntax(&self, window_height: usize) -> Vec<Spans> {
        let lines: Vec<&str> = self.content.split('\n').collect();
        let visible_lines = lines.iter()
            .skip(self.scroll_offset)
            .take(window_height)
            .collect::<Vec<_>>();
        let mut result = Vec::new();

        if let Some(tree) = &self.tree {
            let root = tree.root_node();

            for (line_idx, &line) in visible_lines.iter().enumerate() {
                let line_start = if line_idx + self.scroll_offset == 0 {
                    0
                } else {
                    lines[..line_idx + self.scroll_offset].join("\n").len() + 1
                };
                let line_end = line_start + line.len();

                // Create a vector of (start_byte, end_byte, style) tuples for this line
                let mut style_spans = Vec::new();
                let mut cursor = root.walk();

                // Traverse the tree
                let mut did_visit = false;
                cursor.reset(root);

                loop {
                    let node = cursor.node();
                    let start_byte = node.start_byte();
                    let end_byte = node.end_byte();

                    // Only process nodes that intersect with the current line
                    if start_byte < line_end && end_byte > line_start {
                        let style = match node.kind() {
                            // Keywords
                            "use" | "struct" | "enum" | "impl" | "fn" | "pub" | "mod" | "let" | "mut" | "self" | "match" |
                            "if" | "else" | "for" | "while" | "loop" | "return" | "break" | "continue" | "const" | "static" |
                            "type" | "where" | "unsafe" | "async" | "await" | "move" | "ref" =>
                                Some(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),

                            // Module-level items
                            "use_declaration" | "mod_item" =>
                                Some(Style::default().fg(Color::Cyan)),

                            // Types
                            "type_identifier" | "primitive_type" =>
                                Some(Style::default().fg(Color::Green)),

                            // Functions
                            "function_item" => {
                                // Only color the function name, not its entire body
                                let name_node = node.child_by_field_name("name");
                                if let Some(name) = name_node {
                                    if start_byte == name.start_byte() && end_byte == name.end_byte() {
                                        Some(Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD))
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }

                            // Variables and identifiers
                            "identifier" =>
                                Some(Style::default().fg(Color::White)),

                            // Literals
                            "string_literal" | "raw_string_literal" =>
                                Some(Style::default().fg(Color::Yellow)),
                            "integer_literal" | "float_literal" =>
                                Some(Style::default().fg(Color::Magenta)),

                            // Comments
                            "line_comment" | "block_comment" =>
                                Some(Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)),

                            // Operators and punctuation
                            ":" | "::" | "->" | "=>" | "=" | "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "!" | "." =>
                                Some(Style::default().fg(Color::Yellow)),

                            _ => None
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

                // Sort spans by start position and handle overlaps
                style_spans.sort_by_key(|&(start, _, _)| start);
                let mut spans = Vec::new();
                let mut current_pos = line_start;

                for (start, end, style) in style_spans {
                    // Add unstyled text before this span if needed
                    if start > current_pos {
                        spans.push(Span::raw(
                            self.content[current_pos..start].to_string()
                        ));
                    }

                    // Only add the styled span if it starts after our current position
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
                    spans.push(Span::raw(
                        self.content[current_pos..line_end].to_string()
                    ));
                }

                result.push(Spans::from(spans));
            }
        } else {
            result = visible_lines
                .iter()
                .map(|&line| Spans::from(vec![Span::raw(line.to_string())]))
                .collect();
        }

        result
    }
}

fn tree_sitter_rust() -> Language {
    unsafe {
        extern "C" { fn tree_sitter_rust() -> Language; }
        tree_sitter_rust()
    }
}

fn run_editor(filename: Option<String>) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut editor = Editor::new();
    let mut status_message = String::new();
    let mut status_time = std::time::Instant::now();

    // Load file if specified
    if let Some(path) = filename {
        editor.load_file(path)?;
    }

    loop {
        let window_height = terminal.size()?.height as usize - 2; // Account for borders
        editor.ensure_cursor_visible(window_height);

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),
                    Constraint::Length(1),
                ].as_ref())
                .split(f.size());

            let title = editor.filename.as_ref()
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
            if current_line_number >= editor.scroll_offset &&
                current_line_number < editor.scroll_offset + window_height {
                if let Some(line) = styled_lines.get_mut(current_line_number - editor.scroll_offset) {
                    // Calculate cursor position within the line
                    let line_start = editor.content[..editor.cursor_position]
                        .rfind('\n')
                        .map(|pos| pos + 1)
                        .unwrap_or(0);
                    let cursor_offset = editor.cursor_position - line_start;

                    // Create a new list of spans with the cursor
                    let mut new_spans = Vec::new();
                    let mut current_pos = 0;

                    for span in line.0.iter() {
                        let span_len = span.content.len();
                        if current_pos + span_len > cursor_offset && current_pos <= cursor_offset {
                            // Split this span to insert the cursor
                            let cursor_rel_pos = cursor_offset - current_pos;
                            if cursor_rel_pos > 0 {
                                new_spans.push(Span::styled(
                                    span.content[..cursor_rel_pos].to_string(),
                                    span.style
                                ));
                            }
                            // Add the cursor
                            new_spans.push(Span::styled(
                                "█",
                                Style::default()
                                    .fg(Color::Rgb(169, 183, 198))
                                    .add_modifier(Modifier::SLOW_BLINK)
                            ));
                            if cursor_rel_pos < span_len {
                                new_spans.push(Span::styled(
                                    span.content[cursor_rel_pos..].to_string(),
                                    span.style
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
                            "█",
                            Style::default()
                                .fg(Color::Rgb(169, 183, 198))
                                .add_modifier(Modifier::SLOW_BLINK)
                        ));
                    }

                    *line = Spans(new_spans);
                }
            }

            let paragraph = Paragraph::new(styled_lines)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(Style::default()
                        .bg(Color::Rgb(43, 43, 43))
                        .fg(Color::Rgb(169, 183, 198))
                    ))
                .style(Style::default().bg(Color::Rgb(43, 43, 43)));

            f.render_widget(paragraph, chunks[0]);

            // Add status bar
            if !status_message.is_empty() && status_time.elapsed() < std::time::Duration::from_secs(5) {
                let status_bar = Paragraph::new(status_message.as_str())
                    .style(Style::default().fg(Color::White).bg(Color::Black));
                f.render_widget(status_bar, chunks[1]);
            }
        })?;

        if let Event::Key(key) = event::read()? {
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
                KeyCode::Char(c) => editor.insert_char(c),
                KeyCode::Enter => editor.insert_char('\n'),
                KeyCode::Backspace => editor.delete_char(),
                KeyCode::Left => editor.move_cursor_left(),
                KeyCode::Right => editor.move_cursor_right(),
                KeyCode::Up => editor.move_cursor_up(),
                KeyCode::Down => editor.move_cursor_down(),
                KeyCode::Esc => break,
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
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

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let filename = args.get(1).cloned();

    run_editor(filename)
}