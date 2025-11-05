use crate::error::ServiceError;
use crate::fs_service::{FileSearchResult, FileSystemService};
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::TextContent;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::fmt::Write;
#[mcp_tool(
    name = "search_files_content",
    title="Search files content",
    description = concat!("Searches for text or regex patterns in the content of files. ",
                          "The 'pattern' parameter uses GLOB syntax to filter which files to search (e.g., '*.rs' for Rust files). ",
                          "The 'query' parameter is the search term: literal text by default, or regex when 'is_regex' is true. ",
                          "Note: 'query' does NOT use glob syntax - use standard regex patterns like '.*match' instead of '*match'. ",
                          "Returns detailed matches with file path, line number, column number and a preview of matched text. ",
                          "Optional 'min_bytes' and 'max_bytes' arguments can be used to filter files by size. ",
                          "Ideal for finding specific code, comments, or text when you don't know their exact location."),
    destructive_hint = false,
    idempotent_hint = false,
    open_world_hint = false,
    read_only_hint = true
)]
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, JsonSchema)]

/// A tool for searching content of one or more files based on a path and pattern.
pub struct SearchFilesContent {
    /// The file or directory path to search in.
    pub path: String,
    /// File glob pattern to filter which files to search (e.g., "*.rs", "*.{js,ts}"). This does NOT affect the content search.
    pub pattern: String,
    /// Search term to find in file contents. Use literal text (default) or regex patterns when is_regex=true (e.g., 'TODO', '^function\\s+', '.*Error'). NOT glob syntax.
    pub query: String,
    /// Whether the query is a regular expression. If false, treats query as plain text. If true, treats query as regex pattern. (Default: false)
    pub is_regex: Option<bool>,
    #[serde(rename = "excludePatterns")]
    /// Optional list of patterns to exclude from the search.
    pub exclude_patterns: Option<Vec<String>>,
    /// Minimum file size (in bytes) to include in the search (optional).
    pub min_bytes: Option<u64>,
    /// Maximum file size (in bytes) to include in the search (optional).
    pub max_bytes: Option<u64>,
}

impl SearchFilesContent {
    fn format_result(&self, results: Vec<FileSearchResult>) -> String {
        // TODO: improve capacity estimation
        let estimated_capacity = 2048;

        let mut output = String::with_capacity(estimated_capacity);

        for file_result in results {
            // Push file path
            let _ = writeln!(output, "{}", file_result.file_path.display());

            // Push each match line
            for m in &file_result.matches {
                // Format: "  line:col: text snippet"
                let _ = writeln!(
                    output,
                    "  {}:{}: {}",
                    m.line_number, m.start_pos, m.line_text
                );
            }

            // double spacing
            output.push('\n');
        }

        output
    }
    pub async fn run_tool(
        params: Self,
        context: &FileSystemService,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        let is_regex = params.is_regex.unwrap_or_default();
        match context
            .search_files_content(
                &params.path,
                &params.pattern,
                &params.query,
                is_regex,
                params.exclude_patterns.to_owned(),
                params.min_bytes,
                params.max_bytes,
            )
            .await
        {
            Ok(results) => {
                if results.is_empty() {
                    return Ok(CallToolResult::with_error(CallToolError::new(
                        ServiceError::FromString("No matches found in the files content.".into()),
                    )));
                }
                Ok(CallToolResult::text_content(vec![TextContent::from(
                    params.format_result(results),
                )]))
            }
            Err(err) => Ok(CallToolResult::with_error(CallToolError::new(err))),
        }
    }
}
