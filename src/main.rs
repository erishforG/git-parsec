mod cli;
mod config;
mod conflict;
mod git;
mod output;
mod tracker;
mod worktree;

use anyhow::Result;
use clap::Parser;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    cli::run(cli).await
}
