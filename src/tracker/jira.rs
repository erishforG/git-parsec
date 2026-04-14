use anyhow::{Context, Result, bail};
use super::{Ticket, TicketTracker};

pub struct JiraTracker {
    base_url: String,
    email: Option<String>,
}

impl JiraTracker {
    pub fn new(base_url: &str, email: Option<&str>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            email: email.map(String::from),
        }
    }
}

impl TicketTracker for JiraTracker {
    fn fetch_ticket(&self, id: &str) -> Result<Ticket> {
        let token = std::env::var("PARSEC_JIRA_TOKEN")
            .context("PARSEC_JIRA_TOKEN environment variable not set")?;

        let url = format!("{}/rest/api/3/issue/{}", self.base_url, id);

        // For Jira Cloud: Basic auth with email:api_token
        // Authorization: Basic base64(email:token)
        let auth_header = if let Some(ref email) = self.email {
            let credentials = format!("{}:{}", email, token);
            let encoded = base64_encode(&credentials);
            format!("Basic {}", encoded)
        } else {
            format!("Bearer {}", token)
        };

        // Shell out to curl (sync, no tokio needed)
        let output = std::process::Command::new("curl")
            .args([
                "-s",
                "-H", &format!("Authorization: {}", auth_header),
                "-H", "Content-Type: application/json",
                &url,
            ])
            .output()
            .context("Failed to execute curl")?;

        if !output.status.success() {
            bail!(
                "Failed to fetch Jira ticket {}: {}",
                id,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let body: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("Failed to parse Jira response")?;

        let title = body["fields"]["summary"]
            .as_str()
            .unwrap_or("Untitled")
            .to_string();

        let status = body["fields"]["status"]["name"]
            .as_str()
            .map(String::from);

        let assignee = body["fields"]["assignee"]["displayName"]
            .as_str()
            .map(String::from);

        Ok(Ticket {
            id: id.to_string(),
            title,
            status,
            assignee,
            url: Some(format!("{}/browse/{}", self.base_url, id)),
        })
    }
}

// Simple base64 encoding without external dependency
fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}
