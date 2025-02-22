use tree_sitter::Language;

pub fn tree_sitter_rust() -> Language {
    unsafe {
        extern "C" {
            fn tree_sitter_rust() -> Language;
        }
        tree_sitter_rust()
    }
}
