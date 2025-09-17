use anyhow::Result;
use std::process::Command;

/// Common parsing utilities used across the application
pub struct ParsingUtils;

impl ParsingUtils {
    /// Parse a .pls playlist file to extract the first stream URL
    pub fn parse_pls_content(content: &str) -> Result<String> {
        for line in content.lines() {
            if line.starts_with("File1=") {
                return Ok(line.replace("File1=", ""));
            }
        }
        anyhow::bail!("No stream URL found in .pls file")
    }

    /// Fetch .pls file content using curl
    pub fn fetch_pls_content(pls_url: &str) -> Result<String> {
        let output = Command::new("curl")
            .args(["-s", pls_url])
            .output()?;

        if !output.status.success() {
            anyhow::bail!("Failed to fetch .pls file: {}", pls_url);
        }

        String::from_utf8(output.stdout)
            .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in .pls file: {}", e))
    }

    /// Get stream URL from .pls file (combines fetch and parse)
    pub fn get_stream_from_pls(pls_url: &str) -> Result<String> {
        let content = Self::fetch_pls_content(pls_url)?;
        Self::parse_pls_content(&content)
    }

    /// Determine if a URL is a .pls playlist file
    pub fn is_pls_url(url: &str) -> bool {
        url.ends_with(".pls")
    }

    /// Resolve URL to actual stream URL (handles .pls files)
    pub fn resolve_stream_url(url: &str) -> Result<String> {
        if Self::is_pls_url(url) {
            Self::get_stream_from_pls(url)
        } else {
            Ok(url.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pls_content() {
        let content = r#"[playlist]
NumberOfEntries=1
File1=http://example.com/stream.mp3
Title1=Example Stream
Length1=-1
Version=2"#;

        let result = ParsingUtils::parse_pls_content(content).unwrap();
        assert_eq!(result, "http://example.com/stream.mp3");
    }

    #[test]
    fn test_parse_pls_content_missing_file() {
        let content = r#"[playlist]
NumberOfEntries=1
Title1=Example Stream
Length1=-1
Version=2"#;

        assert!(ParsingUtils::parse_pls_content(content).is_err());
    }

    #[test]
    fn test_is_pls_url() {
        assert!(ParsingUtils::is_pls_url("http://example.com/stream.pls"));
        assert!(!ParsingUtils::is_pls_url("http://example.com/stream.mp3"));
    }

    #[test]
    fn test_resolve_stream_url_non_pls() {
        let url = "http://example.com/stream.mp3";
        let result = ParsingUtils::resolve_stream_url(url).unwrap();
        assert_eq!(result, url);
    }
}