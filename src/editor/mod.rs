mod languages;

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::editor::languages::rust::tree_sitter_rust;
use crate::editor::languages::zig::tree_sitter_zig;
use crate::logger::log_to_file;
use crate::models::Predictor;
use anyhow::{anyhow, Result};
use ratatui::crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use std::io::Stdout;
use std::{fs, io};
use tree_sitter::{Parser, Tree, TreeCursor};

pub struct Editor {
    content: String,
    cursor_position: usize,
    scroll_offset: usize,
    parser: Parser,
    tree: Option<Tree>,
    filename: Option<String>,
    prediction_rx: mpsc::Receiver<String>,
    current_prediction: Option<String>,
    prediction_start_position: Option<usize>,
}

impl Editor {
    pub fn new(path: String) -> (Self, mpsc::Sender<String>) {
        let (prediction_tx, prediction_rx) = mpsc::channel(32);
        let mut parser = Parser::new();
        let filename = path.split(".").last().unwrap_or("rs");
        match filename {
            "zig" => {
                log_to_file("Loading Zig LSP");
                parser
                    .set_language(tree_sitter_zig())
                    .expect("Error loading Zig grammar");
            }
            _ => {
                log_to_file("Defaulting to Rust LSP");
                parser
                    .set_language(tree_sitter_rust())
                    .expect("Error loading Rust grammar");
            }
        }
        (
            Editor {
                content: String::new(),
                cursor_position: 0,
                scroll_offset: 0,
                parser,
                tree: None,
                filename: None,
                current_prediction: None,
                prediction_start_position: None,
                prediction_rx,
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
    pub fn load_file(&mut self, path: String) -> Result<()> {
        self.content = fs::read_to_string(&path)?;
        self.filename = Some(path);
        self.cursor_position = 0;
        self.scroll_offset = 0;
        self.update_syntax_tree();
        Ok(())
    }

    fn highlight_syntax(&self, window_height: usize) -> Vec<Line> {
        let mut result = Vec::new();
        let mut lines: Vec<&str> = self.content.split('\n').collect();
        let visible_lines = lines
            .iter()
            .skip(self.scroll_offset)
            .take(window_height)
            .collect::<Vec<_>>();

        let (prediction_lines, prediction_start_line, cursor_column) =
            self.get_updated_line_with_prediction();

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

                let mut style_spans = Vec::new();
                let mut cursor = root.walk();
                let mut did_visit = false;
                cursor.reset(root);

                did_visit = Self::visit_tree_syntax(line_start, line_end, &mut style_spans, &mut cursor);

                style_spans.sort_by_key(|&(start, _, _)| start);
                let mut spans = Vec::new();
                let mut current_pos = line_start;

                for (start, end, style) in style_spans {
                    if start > current_pos {
                        spans.push(Span::raw(self.content[current_pos..start].to_string()));
                    }
                    if start >= current_pos {
                        spans.push(Span::styled(self.content[start..end].to_string(), style));
                        current_pos = end;
                    }
                }

                if current_pos < line_end {
                    spans.push(Span::raw(self.content[current_pos..line_end].to_string()));
                }

                // we add the prediction to the file here
                if let (Some(pred_lines), Some(start_line), Some(_)) =
                    (&prediction_lines, prediction_start_line, cursor_column)
                {
                    if line_idx == start_line {
                        log_to_file(format!("start index {} end {}", line_idx, start_line).as_str());
                        if let Some(pred_line) = pred_lines.get(start_line) {
                            let diff_string = find_difference(
                                self.get_current_line_content().as_str(),
                                pred_line.as_str(),
                            );
                            spans.push(Span::styled(
                                diff_string,
                                Style::default()
                                    .fg(Color::LightBlue)
                                    .add_modifier(Modifier::ITALIC),
                            ));
                        }
                    }
                }

                result.push(Line::from(spans));
            }

            if let (Some(pred_lines), _, _) =
                (&prediction_lines, prediction_start_line, cursor_column)
            {
                let current_visible_end = self.scroll_offset + visible_lines.len();
                for (idx, _) in pred_lines.iter().enumerate().skip(current_visible_end) {
                    if idx - self.scroll_offset >= window_height {
                        break;
                    }

                    result.push(Line::from(vec![Span::styled(
                        pred_lines[idx].to_string(),
                        Style::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }
        } else
        {
            // high light syntax without tree
            let max_lines = if let Some(pred_lines) = &prediction_lines {
                pred_lines.len().max(lines.len())
            } else {
                lines.len()
            };

            for line_idx in self.scroll_offset..max_lines.min(self.scroll_offset + window_height) {
                let mut spans = Vec::new();

                if line_idx < lines.len() {
                    spans.push(Span::raw(lines[line_idx].to_string()));

                    if let (Some(pred_lines), Some(start_line), _) =
                        (&prediction_lines, prediction_start_line, cursor_column)
                    {
                        if line_idx == start_line {
                            // Add prediction after existing content
                            if let Some(pred_line) = pred_lines.get(line_idx) {
                                if lines[line_idx].len() < pred_line.len() {
                                    spans.push(Span::styled(
                                        pred_line[lines[line_idx].len()..].to_string(),
                                        Style::default()
                                            .fg(Color::DarkGray)
                                            .add_modifier(Modifier::ITALIC),
                                    ));
                                }
                            }
                        }
                    }
                } else if let (Some(pred_lines), Some(start_line), _) =
                    (&prediction_lines, prediction_start_line, cursor_column)
                {
                    if line_idx > start_line && line_idx < pred_lines.len() {
                        spans.push(Span::styled(
                            pred_lines[line_idx].to_string(),
                            Style::default()
                                .fg(Color::DarkGray)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    }
                }

                result.push(Line::from(spans));
            }
        }

        result
    }

    fn visit_tree_syntax(line_start: usize, line_end: usize, style_spans: &mut Vec<(usize, usize, Style)>, cursor: &mut TreeCursor) -> bool {
        let mut did_visit = false;
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
                    | "type" | "where" | "unsafe" | "async" | "await" | "move" | "ref"
                    | "defer" | "try" | "var" => {
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
                            if start_byte == name.start_byte()
                                && end_byte == name.end_byte()
                            {
                                Some(
                                    Style::default()
                                        .fg(Color::Blue)
                                        .add_modifier(Modifier::BOLD),
                                )
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

            if !did_visit && cursor.goto_first_child() {
                did_visit = false;
                continue;
            }

            if cursor.goto_next_sibling() {
                did_visit = false;
                continue;
            }

            if !cursor.goto_parent() {
                return false;
            }
            did_visit = true;
        }
    }

    fn get_updated_line_with_prediction(&self) -> (Option<Vec<String>>, Option<usize>, Option<usize>) {
        if let (Some(pred), Some(start_pos)) =
            (&self.current_prediction, self.prediction_start_position)
        {
            let start_line = self.content[..start_pos]
                .chars()
                .filter(|&c| c == '\n')
                .count();

            let line_start = self.content[..start_pos]
                .rfind('\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);
            let cursor_column = start_pos - line_start;

            let current_line = &self.content[line_start..start_pos];

            let line_end = self.content[start_pos..]
                .find('\n')
                .map(|pos| start_pos + pos)
                .unwrap_or(self.content.len());

            // Get content after the current line
            let post_content = if line_end < self.content.len() {
                &self.content[line_end..]
            } else {
                ""
            };

            let new_prediction = if let Some(stripped) = pred.strip_prefix(current_line) {
                stripped
            } else {
                pred
            };

            let full_content = format!("{}{}{}", current_line, new_prediction, post_content);

            let pred_lines = full_content
                .split('\n')
                .map(|s| s.to_string())
                .collect::<Vec<_>>();

            (Some(pred_lines), Some(start_line), Some(cursor_column))
        } else {
            (None, None, None)
        }
    }

    fn accept_prediction(&mut self) {
        if let (Some(pred), Some(start_pos)) = (
            self.current_prediction.take(),
            self.prediction_start_position.take(),
        ) {
            let line_start = self.content[..start_pos]
                .rfind('\n')
                .map(|pos| pos + 1)
                .unwrap_or(0);

            let line_end = self.content[line_start..]
                .find('\n')
                .map(|pos| line_start + pos)
                .unwrap_or(self.content.len());

            let original_line = &self.content[line_start..line_end];
            if pred.len() > original_line.len() {
                let new_content = format!("{}{}", original_line, &pred[original_line.len()..]);
                self.content
                    .replace_range(line_start..line_end, &new_content);
            }
            self.cursor_position = line_start + pred.len();
            self.update_syntax_tree();
            self.current_prediction = None;
            log_to_file(&format!("accepted prediction: {}", pred));
        }
    }

    fn get_latest_prediction(&mut self) {
        log_to_file("checking latest prediction");
        while let Ok(pred) = self.prediction_rx.try_recv() {
            log_to_file(format!("got prediction from channel {}", pred).as_str());
            self.current_prediction = Some(pred);
            self.prediction_start_position = Some(self.cursor_position);
        }
    }
    fn get_current_line(&self) -> usize {
        self.content[..self.cursor_position]
            .chars()
            .filter(|&c| c == '\n' || c == '\t')
            .count()
    }

    fn ensure_cursor_visible(&mut self, window_height: usize) {
        let current_line = self.get_current_line();
        if current_line < self.scroll_offset {
            self.scroll_offset = current_line;
        }
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

    fn clear_current_line(&mut self) {
        let line_start = self.content[..self.cursor_position]
            .rfind('\n')
            .map(|pos| pos + 1)
            .unwrap_or(0);

        let line_end = self.content[self.cursor_position..]
            .find('\n')
            .map(|pos| self.cursor_position + pos + 1)
            .unwrap_or(self.content.len());

        // todo fix: updating position to avoid overflow when rmeoving lines
        self.content.replace_range(line_start..line_end, "");
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
            .or({
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

pub async fn run(mut editor: Editor, mut predictor: Arc<Predictor>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    loop {
        let window_height = terminal.size()?.height as usize - 2;
        editor.ensure_cursor_visible(window_height);
        editor.get_latest_prediction();
        redraw_editor(&mut terminal, &mut editor)?;
        if event::poll(std::time::Duration::from_millis(10))? {
            // return true to exit, else continue
            if let Ok(true) = handle_key_bindings(&mut editor, &mut predictor) {
                break;
            };
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn handle_key_bindings(editor: &mut Editor, predictor: &mut Arc<Predictor>) -> Result<bool> {
    if let Event::Key(key) = event::read()? {
        match key.code {
            KeyCode::Char('s') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                editor.save_file()?;
            }
            KeyCode::Char('k') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                editor.clear_current_line();
            }
            KeyCode::Tab => {
                if editor.current_prediction.is_some() {
                    editor.accept_prediction();
                } else {
                    let content = editor.get_current_line_content();
                    predictor.clone().stream_prediction_background(content);
                }
            }
            KeyCode::Esc => {
                editor.current_prediction = None;
                editor.prediction_start_position = None;
                return Ok(true);
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
    Ok(false)
}

fn redraw_editor(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    editor: &mut Editor,
) -> Result<()> {
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)].as_ref())
            .split(f.area());

        let title = editor
            .filename
            .as_ref()
            .map(|f| format!("nars - {}", f))
            .unwrap_or_else(|| "nars".to_string());

        let window_height = chunks[0].height as usize - 2; // Account for borders

        // Calculate the maximum line number width
        let total_lines = editor.content.matches('\n').count() + 1;
        let line_num_width = total_lines.to_string().len() + 2; // calculate this based off number of total lines

        // Create a horizontal split for line numbers and content
        let horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(line_num_width as u16),
                Constraint::Min(1),
            ])
            .split(chunks[0]);

        let mut styled_lines = editor.highlight_syntax(window_height);
        let mut line_numbers = Vec::new();

        for i in 0..styled_lines.len() + 1 {
            if i == 0 {
                line_numbers.push(Line::from(vec![Span::styled(
                    " ".repeat(line_num_width),
                    Style::default().fg(Color::DarkGray),
                )]));
            } else {
                let line_num = editor.scroll_offset + i;
                line_numbers.push(Line::from(vec![Span::styled(
                    format!("{:>width$} ", line_num, width = line_num_width - 1),
                    Style::default().fg(Color::DarkGray),
                )]));
            }
        }

        // Add cursor indicator
        let current_line_number = editor.content[..editor.cursor_position]
            .chars()
            .filter(|&c| c == '\n')
            .count();

        // Only show cursor if the line is currently visible
        if current_line_number >= editor.scroll_offset
            && current_line_number < editor.scroll_offset + window_height
        {
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
                                .fg(Color::LightYellow)
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

        // Render line numbers
        let line_numbers_widget = Paragraph::new(line_numbers)
            .block(Block::default().borders(Borders::RIGHT))
            .style(Style::default().bg(Color::Black));

        // Render main content
        let paragraph = Paragraph::new(styled_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .style(Style::default().bg(Color::Black).fg(Color::White)),
            )
            .style(Style::default().bg(Color::Black));

        f.render_widget(line_numbers_widget, horizontal_chunks[0]);
        f.render_widget(paragraph, horizontal_chunks[1]);
    })?;
    Ok(())
}

fn find_difference(s1: &str, s2: &str) -> String {
    if !s2.starts_with(s1) {
        return String::new(); // Return empty string if they don't match
    }

    s2[s1.len()..].to_string() // Return the remainder of s2 after s1's length
}
