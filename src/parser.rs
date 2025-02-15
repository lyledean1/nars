use regex::Regex;
use anyhow::{anyhow, Result};

#[derive(Debug)]
pub struct ParsedCode {
    pub raw_code: String,
    pub language: Option<String>,
}

impl ParsedCode {
    pub fn new(raw_code: String, language: Option<String>) -> Self {
        Self { raw_code, language }
    }
}

pub fn parse_code_output(input: &str) -> Result<ParsedCode> {
    // Match code blocks with optional language specifier
    let code_block_regex = Regex::new(r"(?s)```(?:(\w+)\n)?(.*?)```")?;

    if let Some(captures) = code_block_regex.captures(input) {
        let language = captures.get(1).map(|m| m.as_str().to_string());
        let code = captures.get(2)
            .map(|m| m.as_str().trim().to_string())
            .ok_or(anyhow!("No code content found"))?;

        Ok(ParsedCode::new(code, language))
    } else {
        // If no code block is found, treat the entire input as raw code
        Ok(ParsedCode::new(input.trim().to_string(), None))
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
        assert_eq!(result.raw_code.trim(), r#"fn main() {
            println!("Hello");
        }"#);
    }

    #[test]
    fn test_parse_raw_code() {
        let input = "fn main() { println!(\"Hello\"); }";
        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, None);
        assert_eq!(result.raw_code, input);
    }

    #[test]
    fn test_parse_code_block_no_language() {
        let input = r#"```
        let x = 42;
        ```"#;

        let result = parse_code_output(input).unwrap();
        assert_eq!(result.language, None);
        assert_eq!(result.raw_code.trim(), "let x = 42;");
    }
}