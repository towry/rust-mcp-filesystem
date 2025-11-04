use clap::Parser;
use rust_mcp_filesystem::{cli, server, tools::FileSystemTools};

#[tokio::main]
async fn main() {
    let arguments = cli::CommandArguments::parse();

    // Handle --list-tools flag
    if arguments.list_tools {
        println!("Available MCP Filesystem Tools:\n");
        let tools = FileSystemTools::tools();
        for (idx, tool) in tools.iter().enumerate() {
            println!("{}. {} - {}",
                idx + 1,
                tool.name,
                tool.description.as_deref().unwrap_or("No description")
            );
        }
        println!("\nTotal: {} tools", tools.len());
        println!("\nNote: 'list_allowed_directories' is always enabled and cannot be disabled.");
        return;
    }

    if let Err(err) = arguments.validate() {
        eprintln!("Error: {err}");
        return;
    };

    if let Err(error) = server::start_server(arguments).await {
        eprintln!("{error}");
    }
}
