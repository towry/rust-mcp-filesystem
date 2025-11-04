use crate::error::ServiceError;
use crate::fs_service::{AstFileSearchResult, FileSystemService};
use rust_mcp_sdk::macros::{JsonSchema, mcp_tool};
use rust_mcp_sdk::schema::TextContent;
use rust_mcp_sdk::schema::{CallToolResult, schema_utils::CallToolError};
use std::fmt::Write;

#[mcp_tool(
    name = "search_code_ast",
    title = "Search code using AST patterns",
    description = concat!(
        "Performs structural code search using Abstract Syntax Tree (AST) pattern matching. ",
        "Unlike text search, this matches code structure, not just text. ",
        "Write patterns like ordinary code using $UPPERCASE as wildcards to match any AST node.\n\n",
        "Examples:\n",
        "- Pattern: 'function $NAME() {}' matches all no-argument functions\n",
        "- Pattern: 'if ($COND) { $BODY }' matches all if statements\n",
        "- Pattern: 'const $VAR = $VALUE' matches all const declarations\n",
        "- Pattern: 'import { $ITEMS } from \"$MODULE\"' matches named imports\n\n",
        "Supported languages: TypeScript, JavaScript, Rust, Python, Go, Java, C/C++, and more.\n",
        "Use 'fileExtensions' to filter files (e.g., [\"ts\", \"tsx\"] for TypeScript files)."
    ),
    destructive_hint = false,
    idempotent_hint = false,
    open_world_hint = false,
    read_only_hint = true
)]
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, JsonSchema)]
/// A tool for searching code using AST (Abstract Syntax Tree) pattern matching.
pub struct SearchCodeAst {
    /// The directory path to search in.
    pub path: String,
    /// The file glob pattern to match (e.g., "**/*.ts", "src/**/*.rs").
    pub pattern: String,
    /// The AST pattern to search for (e.g., "function $NAME($ARGS) { $BODY }").
    /// Use $UPPERCASE for wildcards that match any AST node.
    #[serde(rename = "astPattern")]
    pub ast_pattern: String,
    /// The programming language to parse.
    /// Supported: typescript, javascript, rust, python, go, java, cpp, c, csharp, swift, ruby, php, html, css, etc.
    pub language: String,
    #[serde(rename = "excludePatterns")]
    /// Optional list of glob patterns to exclude from the search.
    pub exclude_patterns: Option<Vec<String>>,
    #[serde(rename = "fileExtensions")]
    /// Optional list of file extensions to filter (e.g., ["ts", "tsx"]).
    pub file_extensions: Option<Vec<String>>,
    #[serde(rename = "maxLines", skip_serializing_if = "Option::is_none")]
    /// Optional: Maximum lines to show per match (default: unlimited).
    /// Useful for limiting output when matches are very large.
    pub max_lines: Option<u64>,
}

impl SearchCodeAst {
    fn format_result(&self, results: Vec<AstFileSearchResult>) -> String {
        let estimated_capacity = 4096;
        let mut output = String::with_capacity(estimated_capacity);

        for file_result in results {
            let _ = writeln!(output, "{}", file_result.file_path.display());

            for m in &file_result.matches {
                // Format: "  line:col-range: matched code"
                let _ = writeln!(
                    output,
                    "  {}:{} (bytes {}-{}):",
                    m.line_number, m.column, m.byte_range.0, m.byte_range.1
                );

                // Handle line limiting
                let lines: Vec<&str> = m.matched_code.lines().collect();
                let total_lines = lines.len();

                if let Some(max_lines) = self.max_lines {
                    let max_lines_usize = max_lines as usize;
                    if total_lines > max_lines_usize {
                        // Show first max_lines lines
                        for line in lines.iter().take(max_lines_usize) {
                            let _ = writeln!(output, "    {}", line);
                        }
                        let omitted = total_lines - max_lines_usize;
                        let _ = writeln!(output, "    ... ({} more lines omitted)", omitted);
                    } else {
                        // Show all lines if within limit
                        for line in lines {
                            let _ = writeln!(output, "    {}", line);
                        }
                    }
                } else {
                    // No limit, show all lines
                    for line in lines {
                        let _ = writeln!(output, "    {}", line);
                    }
                }
                output.push('\n');
            }

            output.push('\n');
        }

        output
    }

    pub async fn run_tool(
        params: Self,
        context: &FileSystemService,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        let exclude_patterns = params.exclude_patterns.clone();
        let file_extensions = params.file_extensions.clone();

        match context
            .search_files_ast(
                &params.path,
                &params.pattern,
                &params.ast_pattern,
                &params.language,
                exclude_patterns,
                file_extensions,
            )
            .await
        {
            Ok(results) => {
                if results.is_empty() {
                    return Ok(CallToolResult::with_error(CallToolError::new(
                        ServiceError::FromString(
                            "No AST pattern matches found in the files.".into(),
                        ),
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
