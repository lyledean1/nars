use std::fs::OpenOptions;
use std::io::{Write};

pub fn log_to_file(message: &str) {
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