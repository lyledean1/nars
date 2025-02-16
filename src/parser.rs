use anyhow::{anyhow, Result};
use regex::Regex;

#[derive(Debug)]
pub struct ParsedCode {
    pub code: String,
    pub language: Option<String>,
    pub is_complete: bool,
}

impl ParsedCode {
    pub fn new(code: String, language: Option<String>) -> Self {
        Self {
            code,
            language,
            is_complete: true,
        }
    }
}

pub fn parse_code_output(input: &str) -> Result<ParsedCode> {
    // Match code blocks that might be incomplete
    // This regex will match:
    // 1. Complete code blocks: ```lang\ncode```
    // 2. Incomplete blocks: ```lang\ncode
    // 3. Raw code without markers
    let code_block_regex = Regex::new(r"(?s)```(?:(\w+)\n)?(.*?)(?:```|$)")?;

    if let Some(captures) = code_block_regex.captures(input) {
        let language = captures.get(1).map(|m| m.as_str().to_string());
        let code = captures
            .get(2)
            .map(|m| m.as_str().trim().to_string())
            .ok_or(anyhow!("No code content found"))?;

        let is_complete = input.trim().ends_with("```");
        Ok(ParsedCode {
            code,
            language,
            is_complete,
        })
    } else {
        Ok(ParsedCode {
            code: input.trim().to_string(),
            language: None,
            is_complete: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_markdown_code_block() {
        let input = r#"```rust
        fn main() {
            println!("Hello");
        }
        ```"#;

        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, Some("rust".to_string()));
        assert_eq!(
            result.code.trim(),
            r#"fn main() {
            println!("Hello");
        }"#
        );
    }

    #[test]
    fn test_incomplete_code() {
        let input = r#"```rust
        fn main() {
            println!("Hello");"#;

        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, Some("rust".to_string()));
        assert_eq!(
            result.code.trim(),
            r#"fn main() {
            println!("Hello");"#
        );
    }

    #[test]
    fn test_parse_raw_code() {
        let input = "fn main() { println!(\"Hello\"); }";
        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, None);
        assert_eq!(result.code, input);
    }

    #[test]
    fn test_parse_code_block_no_language() {
        let input = r#"```
        let x = 42;
        ```"#;

        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, None);
        assert_eq!(result.code.trim(), "let x = 42;");
    }
}
