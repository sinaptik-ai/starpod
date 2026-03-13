use std::collections::HashSet;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{debug, error, info, warn};

use orion_agent::OrionAgent;
use orion_core::{ChatMessage, OrionConfig};

/// Maximum Telegram message length.
const MAX_MSG_LEN: usize = 4096;

/// Allowed user IDs (empty = no one can chat, only /start works to show user ID).
#[derive(Clone)]
struct AllowedUsers(Arc<HashSet<u64>>);

/// Run the Telegram bot.
///
/// This takes ownership of the agent so it can be shared with the gateway
/// if both are running. Pass `Arc<OrionAgent>` directly via `run_with_agent`.
pub async fn run(config: OrionConfig, token: String) -> orion_core::Result<()> {
    let allowed = config.telegram_allowed_users.clone();
    let agent = Arc::new(OrionAgent::new(config).await?);
    run_with_agent_filtered(agent, token, allowed).await
}

/// Run the Telegram bot with a pre-built agent (for sharing with the gateway).
pub async fn run_with_agent(agent: Arc<OrionAgent>, token: String) -> orion_core::Result<()> {
    run_with_agent_filtered(agent, token, Vec::new()).await
}

/// Run the Telegram bot with a pre-built agent and an allow-list.
pub async fn run_with_agent_filtered(
    agent: Arc<OrionAgent>,
    token: String,
    allowed_users: Vec<u64>,
) -> orion_core::Result<()> {
    if allowed_users.is_empty() {
        warn!("Telegram bot has no allowed_users configured — no one can chat. Send /start to the bot to get your user ID, then add it to telegram_allowed_users in config.toml");
    } else {
        info!(
            allowed_users = ?allowed_users,
            "Telegram bot restricted to {} user(s)",
            allowed_users.len()
        );
    }

    let allowed = AllowedUsers(Arc::new(allowed_users.into_iter().collect()));

    let bot = Bot::new(&token);

    let handler = Update::filter_message().endpoint(handle_message);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![agent, allowed])
        .default_handler(|_| async {})
        .error_handler(LoggingErrorHandler::with_custom_text(
            "Error in telegram handler",
        ))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: TeloxideMessage,
    agent: Arc<OrionAgent>,
    allowed: AllowedUsers,
) -> Result<(), teloxide::RequestError> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()), // Ignore non-text messages
    };

    let user_id = msg.from.as_ref().map(|u| u.id.0);
    let chat_id = msg.chat.id;
    let user_id_str = user_id.map(|id| id.to_string());

    // /start always works — it shows the user their ID for config setup
    if text == "/start" {
        let mut greeting = "Hello\\! I'm Orion, your personal AI assistant\\.".to_string();
        if let Some(id) = user_id {
            greeting.push_str(&format!("\n\nYour user ID: `{}`", id));
            greeting.push_str("\nAdd this to `telegram_allowed_users` in your config to start chatting\\.");
        }
        bot.send_message(chat_id, &greeting)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .or_else(|_| {
                let fallback = format!(
                    "Hello! I'm Orion, your personal AI assistant.\n\nYour user ID: {}\nAdd this to telegram_allowed_users in your config to start chatting.",
                    user_id.map(|id| id.to_string()).unwrap_or_default()
                );
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(
                        bot.send_message(chat_id, fallback).send()
                    )
                })
            })
            .ok();
        return Ok(());
    }

    // Check allow-list (empty = no one allowed)
    let is_allowed = user_id
        .map(|id| allowed.0.contains(&id))
        .unwrap_or(false);
    if !is_allowed {
        debug!(
            user_id = ?user_id,
            chat_id = %chat_id,
            "Rejected message from unauthorized user"
        );
        bot.send_message(
            chat_id,
            "Sorry, you're not authorized to use this bot. Ask the bot owner to add your user ID to the allow-list.",
        )
        .await
        .ok();
        return Ok(());
    }

    debug!(
        user_id = ?user_id_str,
        chat_id = %chat_id,
        text = %text,
        "Telegram message received"
    );

    // Show typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
        .await
        .ok();

    let chat_msg = ChatMessage {
        text,
        user_id: user_id_str,
        channel_id: Some("telegram".into()),
        attachments: Vec::new(),
    };

    match agent.chat(chat_msg).await {
        Ok(response) => {
            // Split long messages at line boundaries
            let chunks = split_message(&response.text, MAX_MSG_LEN);
            for chunk in chunks {
                bot.send_message(chat_id, &chunk)
                    .parse_mode(ParseMode::MarkdownV2)
                    .await
                    .or_else(|_| {
                        // Fallback: send as plain text if markdown parsing fails
                        tokio::task::block_in_place(|| {
                            tokio::runtime::Handle::current().block_on(
                                bot.send_message(chat_id, &chunk).send()
                            )
                        })
                    })
                    .ok();
            }
        }
        Err(e) => {
            error!(error = %e, "Chat error");
            bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Split a message into chunks that fit within Telegram's limit.
/// Tries to split at line boundaries.
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find a good split point (newline before max_len)
        let split_at = remaining[..max_len]
            .rfind('\n')
            .unwrap_or(max_len);

        let (chunk, rest) = remaining.split_at(split_at);
        chunks.push(chunk.to_string());

        // Skip the newline if we split on one
        remaining = rest.strip_prefix('\n').unwrap_or(rest);
    }

    chunks
}

// Re-export for use in CLI
use teloxide::types::Message as TeloxideMessage;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("Hello world", 4096);
        assert_eq!(chunks, vec!["Hello world"]);
    }

    #[test]
    fn test_split_message_long() {
        let line = "x".repeat(100);
        let text: String = (0..50).map(|_| line.as_str()).collect::<Vec<_>>().join("\n");
        let chunks = split_message(&text, 4096);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 4096);
        }
    }

    #[test]
    fn test_split_message_no_newlines() {
        let text = "x".repeat(5000);
        let chunks = split_message(&text, 4096);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4096);
        assert_eq!(chunks[1].len(), 904);
    }
}
