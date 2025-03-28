// First, create a build.rs file in your project root:
// build.rs
use std::path::PathBuf;

fn main() {
    let dir: PathBuf = ["tree-sitter-rust", "src"].iter().collect();

    cc::Build::new()
        .include(&dir)
        .file(dir.join("parser.c"))
        .file(dir.join("scanner.c"))
        .compile("tree-sitter-rust");

    let dir: PathBuf = ["tree-sitter-zig", "src"].iter().collect();

    cc::Build::new()
        .include(&dir)
        .file(dir.join("parser.c"))
        .compile("tree-sitter-zig");
}
