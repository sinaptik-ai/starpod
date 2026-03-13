use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{debug, error, info};

use orion_agent::OrionAgent;
use orion_core::{ChatMessage, OrionConfig};

/// Maximum Telegram message length.
const MAX_MSG_LEN: usize = 4096;

/// Run the Telegram bot.
///
/// This takes ownership of the agent so it can be shared with the gateway
/// if both are running. Pass `Arc<OrionAgent>` directly via `run_with_agent`.
pub async fn run(config: OrionConfig, token: String) -> orion_core::Result<()> {
    let agent = Arc::new(OrionAgent::new(config).await?);
    run_with_agent(agent, token).await
}

/// Run the Telegram bot with a pre-built agent (for sharing with the gateway).
pub async fn run_with_agent(agent: Arc<OrionAgent>, token: String) -> orion_core::Result<()> {
    info!("Starting Telegram bot");

    let bot = Bot::new(&token);

    let handler = Update::filter_message().endpoint(handle_message);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![agent])
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
) -> Result<(), teloxide::RequestError> {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()), // Ignore non-text messages
    };

    let user_id = msg.from.as_ref().map(|u| u.id.0.to_string());
    let chat_id = msg.chat.id;

    debug!(
        user_id = ?user_id,
        chat_id = %chat_id,
        text = %text,
        "Telegram message received"
    );

    // Handle /start command
    if text == "/start" {
        bot.send_message(chat_id, "Hello! I'm Orion, your personal AI assistant. Send me a message to get started.")
            .await?;
        return Ok(());
    }

    // Show typing indicator
    bot.send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
        .await
        .ok();

    let chat_msg = ChatMessage {
        text,
        user_id,
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
