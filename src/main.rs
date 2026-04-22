#[macro_use]
mod errors;

mod cli;
mod config;
mod conflict;
mod env;
mod git;
mod github;
mod gitlab;
mod oplog;
mod output;
mod tracker;
mod worktree;

use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json_mode = cli.json;

    match cli::run(cli).await {
        Ok(()) => {}
        Err(err) => {
            let (code, typed_msg) = errors::extract_code(&err);

            if json_mode {
                let msg = if typed_msg.is_empty() {
                    format!("{err:#}")
                } else {
                    typed_msg.to_string()
                };
                let je = errors::JsonError {
                    error: true,
                    code,
                    message: msg,
                };
                let _ = serde_json::to_writer(std::io::stdout(), &je);
                println!();
            } else {
                // Display the error with full context chain
                eprintln!("error: {err:#}");
            }

            std::process::exit(code.exit_code());
        }
    }
}
