use clap::{Parser, arg, command};

#[derive(Parser, Debug)]
#[command(name =  env!("CARGO_PKG_NAME"))]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "A lightning-fast, asynchronous, and lightweight MCP server designed for efficient handling of various filesystem operations",
long_about = None)]
pub struct CommandArguments {
    #[arg(
        short = 'w',
        long,
        action = clap::ArgAction::SetTrue,
        value_parser = clap::value_parser!(bool),
        help = "Enables write mode for the app, allowing both reading and writing. Defaults to disabled.",
        env = "ALLOW_WRITE"
    )]
    pub allow_write: bool,

    #[arg(
        short = 't',
        long,
        help = "Enables dynamic directory access control via Roots from the MCP client side. Defaults to disabled.\nWhen enabled, MCP clients that support Roots can dynamically update the allowed directories.\nAny directories provided by the client will completely replace the initially configured allowed directories on the server.",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::value_parser!(bool),
        env = "ENABLE_ROOTS"
    )]
    pub enable_roots: bool,

    #[arg(
        long,
        help = "Comma-separated list of tools to enable. Use 'all' to enable all tools. Tools are specified by their snake_case names.",
        long_help = "Specify which tools to enable using comma-separated tool names.\nUse '--tools all' to enable all available tools.\nThe 'list_allowed_directories' tool is always enabled.\n\nExamples:\n  --tools all\n  --tools read_text_file,get_file_info,write_file\n  --tools read_text_file,read_multiple_text_files,list_directory",
        env = "TOOLS"
    )]
    pub tools: Option<String>,

    #[arg(
        long,
        help = "List all available tools and exit",
        action = clap::ArgAction::SetTrue,
        value_parser = clap::value_parser!(bool)
    )]
    pub list_tools: bool,

    #[arg(
        help = "List of directories that are permitted for the operation. It is required when 'enable-roots' is not provided OR client does not support Roots.",
        long_help = concat!("Provide a space-separated list of directories that are permitted for the operation.\nThis list allows multiple directories to be provided.\n\nExample:  ", env!("CARGO_PKG_NAME"), " /path/to/dir1 /path/to/dir2 /path/to/dir3"),
        required = false
    )]
    pub allowed_directories: Vec<String>,
}

impl CommandArguments {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enable_roots && self.allowed_directories.is_empty() {
            return Err(format!(
                " <ALLOWED_DIRECTORIES> is required when `--enable-roots` is not provided.\n Run `{} --help` to view the usage instructions.",
                env!("CARGO_PKG_NAME")
            ));
        }
        Ok(())
    }
}
