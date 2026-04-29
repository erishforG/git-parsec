use std::path::Path;

use anyhow::{Context, Result};
use colored::Colorize;

use crate::config::ParsecConfig;
use crate::git;
use crate::output::{self, Mode};

pub async fn config_init(mode: Mode) -> Result<()> {
    let config = ParsecConfig::init_interactive()?;
    config.save()?;

    output::print_config_init(mode);
    Ok(())
}

pub async fn config_show(mode: Mode) -> Result<()> {
    let config = ParsecConfig::load()?;

    output::print_config_show(&config, mode);
    Ok(())
}

pub async fn root(repo_path: &Path) -> Result<()> {
    let repo_root = git::get_main_repo_root(repo_path)?;
    print!("{}", repo_root.display());
    Ok(())
}

pub async fn init_shell(shell: &str) -> Result<()> {
    let script = match shell {
        "bash" => INIT_SHELL_BASH,
        _ => INIT_SHELL_ZSH,
    };
    print!("{}", script);
    Ok(())
}

pub async fn init_install(shell: &str, yes: bool) -> Result<()> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let (config_path, eval_line) = match shell {
        "bash" => (
            home.join(".bashrc"),
            "eval \"$(parsec init bash)\"".to_string(),
        ),
        _ => (
            home.join(".zshrc"),
            "eval \"$(parsec init zsh)\"".to_string(),
        ),
    };

    // Check if already installed
    if config_path.exists() {
        let existing = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read {}", config_path.display()))?;
        if existing.contains("parsec init") {
            println!(
                "{}",
                format!(
                    "Shell integration already present in {}. Nothing to do.",
                    config_path.display()
                )
                .yellow()
            );
            return Ok(());
        }
    }

    if !yes {
        if crate::env::is_agent() {
            anyhow::bail!("Interactive confirmation is not available in agent mode. Use `parsec config shell --yes` to skip prompts.");
        }
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Add shell integration to {}?",
                config_path.display()
            ))
            .default(true)
            .interact()
            .context("Failed to read confirmation")?;

        if !confirmed {
            println!("{}", "Skipped.".dimmed());
            return Ok(());
        }
    }

    // Append the eval line with a comment
    let append = format!("\n# parsec shell integration\n{}\n", eval_line);
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&config_path)
        .with_context(|| format!("Failed to open {} for writing", config_path.display()))?;
    file.write_all(append.as_bytes())
        .with_context(|| format!("Failed to write to {}", config_path.display()))?;

    println!(
        "{}",
        format!(
            "Shell integration added to {}. Run `source {}` or restart your shell.",
            config_path.display(),
            config_path.display()
        )
        .green()
    );
    Ok(())
}

pub async fn config_shell(shell: &str, _mode: Mode) -> Result<()> {
    let script = match shell {
        "bash" => SHELL_INTEGRATION_BASH,
        _ => SHELL_INTEGRATION_ZSH,
    };
    print!("{}", script);
    Ok(())
}

const SHELL_INTEGRATION_ZSH: &str = r#"
# parsec shell integration - add to ~/.zshrc
# eval "$(parsec config shell zsh)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        command parsec "$@"
    fi
}
"#;

const SHELL_INTEGRATION_BASH: &str = r#"
# parsec shell integration - add to ~/.bashrc
# eval "$(parsec config shell bash)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        command parsec "$@"
    fi
}
"#;

const INIT_SHELL_ZSH: &str = r#"
# parsec shell integration - add to ~/.zshrc
# eval "$(parsec init zsh)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        # Save repo root before merge (CWD may be deleted after)
        local saved_root=""
        if [[ "$1" == "merge" ]]; then
            saved_root=$(command parsec root 2>/dev/null)
        fi
        command parsec "$@"
        local exit_code=$?
        # After merge, if CWD was deleted (worktree cleaned up), cd to main repo
        if [[ "$1" == "merge" && $exit_code -eq 0 ]] && [[ ! -d "$(pwd)" ]]; then
            if [[ -n "$saved_root" && -d "$saved_root" ]]; then
                cd "$saved_root"
                echo "  cd $saved_root"
            fi
        fi
        return $exit_code
    fi
}
"#;

const INIT_SHELL_BASH: &str = r#"
# parsec shell integration - add to ~/.bashrc
# eval "$(parsec init bash)"
function parsec() {
    if [[ "$1" == "switch" && -n "$2" ]]; then
        local dir
        dir=$(command parsec switch "${@:2}" 2>&1)
        if [[ $? -eq 0 && -d "$dir" ]]; then
            cd "$dir"
        else
            echo "$dir" >&2
            return 1
        fi
    else
        # Save repo root before merge (CWD may be deleted after)
        local saved_root=""
        if [[ "$1" == "merge" ]]; then
            saved_root=$(command parsec root 2>/dev/null)
        fi
        command parsec "$@"
        local exit_code=$?
        # After merge, if CWD was deleted (worktree cleaned up), cd to main repo
        if [[ "$1" == "merge" && $exit_code -eq 0 ]] && [[ ! -d "$(pwd)" ]]; then
            if [[ -n "$saved_root" && -d "$saved_root" ]]; then
                cd "$saved_root"
                echo "  cd $saved_root"
            fi
        fi
        return $exit_code
    fi
}
"#;

pub async fn config_man(dir: &Path) -> Result<()> {
    use clap::CommandFactory;
    let cmd = crate::cli::Cli::command();
    let man = clap_mangen::Man::new(cmd);
    let mut buf = Vec::new();
    man.render(&mut buf)?;

    let man1_dir = dir.join("man1");
    std::fs::create_dir_all(&man1_dir)
        .with_context(|| format!("Failed to create directory {}", man1_dir.display()))?;

    let path = man1_dir.join("parsec.1");
    std::fs::write(&path, buf)
        .with_context(|| format!("Failed to write man page to {}", path.display()))?;

    println!("Man page installed to {}", path.display());
    println!("Try: man parsec");
    Ok(())
}

pub async fn config_completions(shell: clap_complete::Shell) -> Result<()> {
    use clap::CommandFactory;
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(shell, &mut cmd, "parsec", &mut std::io::stdout());
    Ok(())
}

pub async fn config_schema() -> Result<()> {
    let schema = include_str!("../../../schema/parsec-config.schema.json");
    println!("{}", schema);
    Ok(())
}
