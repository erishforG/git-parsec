#[macro_use]
mod errors;

mod ai;
mod bitbucket;
mod cli;
mod config;
mod conflict;
mod env;
mod execlog;
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
            // issue #303: prefer the typed ParsecError (which renders as the
            // standard `error: / caused by: / help:` 3-line format via its
            // Display impl). Fall back to the anyhow chain for untyped errors.
            let typed = errors::extract_full(&err);
            let code = typed.map(|pe| pe.code).unwrap_or(errors::ErrorCode::E999);

            if json_mode {
                let je = match typed {
                    Some(pe) => errors::JsonError {
                        error: true,
                        code: pe.code,
                        message: pe.message.clone(),
                        caused_by: pe.caused_by.clone(),
                        help: pe.help.clone(),
                    },
                    None => errors::JsonError {
                        error: true,
                        code: errors::ErrorCode::E999,
                        message: format!("{err:#}"),
                        caused_by: None,
                        help: None,
                    },
                };
                let _ = serde_json::to_writer(std::io::stdout(), &je);
                println!();
            } else {
                match typed {
                    // Typed error already includes the `error:` prefix in its
                    // Display, so print it directly (3-line format, #303).
                    Some(pe) => eprintln!("{pe}"),
                    // Untyped: keep the legacy single-line behavior.
                    None => eprintln!("error: {err:#}"),
                }
            }

            std::process::exit(code.exit_code());
        }
    }
}
