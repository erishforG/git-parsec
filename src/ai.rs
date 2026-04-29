//! AI provider abstraction for commit message generation.
//!
//! Supports OpenAI-compatible and Anthropic APIs.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::AiProvider;

// ---------------------------------------------------------------------------
// OpenAI types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Serialize)]
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMsg,
}

#[derive(Deserialize)]
struct OpenAiMsg {
    content: String,
}

// ---------------------------------------------------------------------------
// Anthropic types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a commit message from a diff using the configured AI provider.
pub async fn generate_commit_message(
    provider: &AiProvider,
    model: &str,
    api_key: &str,
    diff: &str,
    ticket: Option<&str>,
    conventional: bool,
) -> Result<String> {
    let prompt = build_prompt(diff, ticket, conventional);

    let raw = match provider {
        AiProvider::OpenAi => call_openai(model, api_key, &prompt).await?,
        AiProvider::Anthropic => call_anthropic(model, api_key, &prompt).await?,
    };

    // Clean up: strip surrounding quotes, trim
    let msg = raw.trim().trim_matches('"').trim().to_string();
    Ok(msg)
}

fn build_prompt(diff: &str, ticket: Option<&str>, conventional: bool) -> String {
    // Truncate very large diffs to avoid token limits
    let max_diff_len = 12_000;
    let truncated = if diff.len() > max_diff_len {
        format!(
            "{}\n\n... (diff truncated, {} bytes total)",
            &diff[..max_diff_len],
            diff.len()
        )
    } else {
        diff.to_string()
    };

    let mut prompt = String::from(
        "Generate a concise git commit message for the following changes. \
         Return ONLY the commit message text, nothing else.\n\n",
    );

    if conventional {
        prompt.push_str(
            "Use Conventional Commits format: type(scope): description\n\
             Valid types: feat, fix, refactor, docs, test, chore, style, perf, ci, build\n\n",
        );
    }

    if let Some(t) = ticket {
        prompt.push_str(&format!(
            "The ticket ID is {t}. Prefix the message with the ticket: [{t}] message\n\n"
        ));
    }

    prompt.push_str("Rules:\n");
    prompt.push_str("- First line: imperative mood, max 72 characters\n");
    prompt.push_str("- If needed, add a blank line then a brief body (2-3 lines max)\n");
    prompt.push_str("- Focus on WHY, not WHAT (the diff shows what)\n\n");
    prompt.push_str("Diff:\n```\n");
    prompt.push_str(&truncated);
    prompt.push_str("\n```");

    prompt
}

async fn call_openai(model: &str, api_key: &str, prompt: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&OpenAiRequest {
            model: model.to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            max_tokens: 300,
            temperature: 0.3,
        })
        .send()
        .await
        .context("Failed to call OpenAI API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("OpenAI API error ({}): {}", status, body);
    }

    let body: OpenAiResponse = resp
        .json()
        .await
        .context("Failed to parse OpenAI response")?;
    body.choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))
}

async fn call_anthropic(model: &str, api_key: &str, prompt: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&AnthropicRequest {
            model: model.to_string(),
            max_tokens: 300,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
        })
        .send()
        .await
        .context("Failed to call Anthropic API")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Anthropic API error ({}): {}", status, body);
    }

    let body: AnthropicResponse = resp
        .json()
        .await
        .context("Failed to parse Anthropic response")?;
    body.content
        .first()
        .map(|c| c.text.clone())
        .ok_or_else(|| anyhow::anyhow!("No response from Anthropic"))
}
