use std::path::Path;

use rust_mcp_sdk::{
    macros::{JsonSchema, mcp_tool},
    schema::{CallToolResult, TextContent, schema_utils::CallToolError},
};

use crate::fs_service::FileSystemService;

// read_file_lines
#[mcp_tool(
    name = "read_file_lines",
    title="Read file lines",
    description = concat!("Reads lines from a text file with flexible positioning options.",
    "By default, reads from the beginning: skips 'offset' lines (0-based) and then reads up to 'limit' lines if specified, or reads until EOF otherwise.",
    "When 'from_end' is true, reads from the file's end: 'offset' lines are skipped from the end, and 'limit' lines are read backwards (output preserves original order).",
    "Examples: offset=0,limit=10 reads first 10 lines; from_end=true,limit=10 reads last 10 lines; offset=5,limit=20 reads lines 6-25.",
    "Useful for partial reads, pagination, log tailing, or previewing sections of large text files.",
    "Only works within allowed directories."),
    destructive_hint = false,
    idempotent_hint = false,
    open_world_hint = false,
    read_only_hint = true
)]
#[derive(::serde::Deserialize, ::serde::Serialize, Clone, Debug, JsonSchema)]
pub struct ReadFileLines {
    /// The path of the file to read.
    pub path: String,
    /// Number of lines to skip from the start (0-based) or from the end (when from_end=true).
    #[serde(default)]
    pub offset: u64,
    /// Optional maximum number of lines to read after applying offset.
    pub limit: Option<u64>,
    /// If true, reads from the end of the file instead of the beginning. Default: false.
    #[serde(default)]
    pub from_end: bool,
}

impl ReadFileLines {
    pub async fn run_tool(
        params: Self,
        context: &FileSystemService,
    ) -> std::result::Result<CallToolResult, CallToolError> {
        let result = context
            .read_file_lines(
                Path::new(&params.path),
                params.offset as usize,
                params.limit.map(|v| v as usize),
                params.from_end,
            )
            .await
            .map_err(CallToolError::new)?;

        Ok(CallToolResult::text_content(vec![TextContent::from(
            result,
        )]))
    }
}
