use tree_sitter::Language;

pub fn tree_sitter_zig() -> Language {
    unsafe {
        extern "C" {
            fn tree_sitter_zig() -> Language;
        }
        tree_sitter_zig()
    }
}