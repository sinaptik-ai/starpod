use std::collections::HashSet;
use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio_stream::StreamExt;
use tracing::{debug, error, info, warn};

use agent_sdk::{ContentBlock, Message};
use orion_agent::OrionAgent;
use orion_core::{ChatMessage, OrionConfig};

/// Maximum Telegram message length.
const MAX_MSG_LEN: usize = 4096;

/// Allowed user IDs (empty = no one can chat, only /start works to show user ID).
#[derive(Clone)]
struct AllowedUsers(Arc<HashSet<u64>>);

/// Streaming config passed through teloxide DI.
#[derive(Clone)]
struct StreamConfig {
    stream_mode: String,
    edit_throttle_ms: u64,
}

/// Agent display name for greetings.
#[derive(Clone)]
struct AgentName(String);

/// Run the Telegram bot.
///
/// This takes ownership of the agent so it can be shared with the gateway
/// if both are running. Pass `Arc<OrionAgent>` directly via `run_with_agent`.
pub async fn run(config: OrionConfig, token: String) -> orion_core::Result<()> {
    let allowed = config.resolved_telegram_allowed_users().to_vec();
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
        warn!("Telegram bot has no allowed_users configured — no one can chat. Send /start to the bot to get your user ID, then add it to [telegram] allowed_users in config.toml");
    } else {
        info!(
            allowed_users = ?allowed_users,
            "Telegram bot restricted to {} user(s)",
            allowed_users.len()
        );
    }

    let allowed = AllowedUsers(Arc::new(allowed_users.into_iter().collect()));
    let stream_cfg = StreamConfig {
        stream_mode: agent.config().telegram.stream_mode.clone(),
        edit_throttle_ms: agent.config().telegram.edit_throttle_ms,
    };
    let agent_name = AgentName(agent.config().identity.display_name().to_string());

    let bot = Bot::new(&token);

    let handler = Update::filter_message().endpoint(handle_message);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![agent, allowed, stream_cfg, agent_name])
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
    stream_cfg: StreamConfig,
    agent_name: AgentName,
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
        let name = &agent_name.0;
        let mut greeting = format!("Hello\\! I'm {}, your personal AI assistant\\.", escape_md(name));
        if let Some(id) = user_id {
            greeting.push_str(&format!("\n\nYour user ID: `{}`", id));
            greeting.push_str("\nAdd this to `\\[telegram\\] allowed_users` in your config to start chatting\\.");
        }
        bot.send_message(chat_id, &greeting)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .or_else(|_| {
                let fallback = format!(
                    "Hello! I'm {}, your personal AI assistant.\n\nYour user ID: {}\nAdd this to [telegram] allowed_users in your config to start chatting.",
                    name,
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

    if stream_cfg.stream_mode == "edit_in_place" {
        handle_streaming(&bot, chat_id, &agent, &text, user_id_str, stream_cfg.edit_throttle_ms).await
    } else {
        handle_blocking(&bot, chat_id, &agent, &text, user_id_str).await
    }
}

/// Blocking mode: wait for full response, then send.
async fn handle_blocking(
    bot: &Bot,
    chat_id: ChatId,
    agent: &Arc<OrionAgent>,
    text: &str,
    user_id_str: Option<String>,
) -> Result<(), teloxide::RequestError> {
    let chat_msg = ChatMessage {
        text: text.to_string(),
        user_id: user_id_str,
        channel_id: Some("telegram".into()),
        channel_session_key: Some(chat_id.0.to_string()),
        attachments: Vec::new(),
    };

    match agent.chat(chat_msg).await {
        Ok(response) => {
            send_response(bot, chat_id, &response.text).await;
        }
        Err(e) => {
            error!(error = %e, "Chat error");
            bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                .await?;
        }
    }

    Ok(())
}

/// Streaming mode: send a placeholder, then edit in-place as tokens arrive.
async fn handle_streaming(
    bot: &Bot,
    chat_id: ChatId,
    agent: &Arc<OrionAgent>,
    text: &str,
    _user_id_str: Option<String>,
    throttle_ms: u64,
) -> Result<(), teloxide::RequestError> {
    // Send placeholder
    let placeholder = bot.send_message(chat_id, "...").await?;
    let msg_id = placeholder.id;

    // Start streaming
    let chat_msg = ChatMessage {
        text: text.to_string(),
        user_id: _user_id_str.clone(),
        channel_id: Some("telegram".into()),
        channel_session_key: Some(chat_id.0.to_string()),
        attachments: Vec::new(),
    };
    let stream_result = agent.chat_stream(&chat_msg).await;
    let (mut stream, session_id) = match stream_result {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to start stream");
            bot.edit_message_text(chat_id, msg_id, format!("Error: {}", e))
                .await
                .ok();
            return Ok(());
        }
    };

    let mut accumulated = String::new();
    let mut last_edit = tokio::time::Instant::now();
    let throttle = tokio::time::Duration::from_millis(throttle_ms);
    let mut result_msg = None;

    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Assistant(assistant)) => {
                for block in &assistant.content {
                    if let ContentBlock::Text { text } = block {
                        accumulated.push_str(text);
                    }
                }

                // Throttled edit
                if last_edit.elapsed() >= throttle && !accumulated.is_empty() {
                    let display = truncate_for_telegram(&accumulated);
                    bot.edit_message_text(chat_id, msg_id, &display)
                        .await
                        .ok();
                    last_edit = tokio::time::Instant::now();
                }
            }
            Ok(Message::Result(result)) => {
                if accumulated.is_empty() {
                    if let Some(text) = &result.result {
                        accumulated = text.clone();
                    }
                }
                result_msg = Some(result);
            }
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "Stream error");
                bot.edit_message_text(chat_id, msg_id, format!("Error: {}", e))
                    .await
                    .ok();
                return Ok(());
            }
        }
    }

    // Final edit with complete response
    if accumulated.is_empty() {
        bot.edit_message_text(chat_id, msg_id, "(no response)")
            .await
            .ok();
    } else if accumulated.len() <= MAX_MSG_LEN {
        // Try markdown first, fall back to plain text
        if bot
            .edit_message_text(chat_id, msg_id, &accumulated)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .is_err()
        {
            bot.edit_message_text(chat_id, msg_id, &accumulated)
                .await
                .ok();
        }
    } else {
        // Response too long for a single message — delete placeholder and send as chunks
        bot.delete_message(chat_id, msg_id).await.ok();
        send_response(bot, chat_id, &accumulated).await;
    }

    // Finalize (record usage, daily log)
    if let Some(ref result) = result_msg {
        agent.finalize_chat(&session_id, text, &accumulated, result).await;
    }

    Ok(())
}

/// Send a (possibly long) response as one or more messages.
async fn send_response(bot: &Bot, chat_id: ChatId, text: &str) {
    let chunks = split_message(text, MAX_MSG_LEN);
    for chunk in chunks {
        bot.send_message(chat_id, &chunk)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .or_else(|_| {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(bot.send_message(chat_id, &chunk).send())
                })
            })
            .ok();
    }
}

/// Truncate text for Telegram's limit during streaming (no markdown).
fn truncate_for_telegram(text: &str) -> String {
    if text.len() <= MAX_MSG_LEN {
        text.to_string()
    } else {
        format!("{}...", &text[..MAX_MSG_LEN - 3])
    }
}

/// Escape special characters for MarkdownV2.
fn escape_md(text: &str) -> String {
    let special = ['_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'];
    let mut result = String::with_capacity(text.len());
    for c in text.chars() {
        if special.contains(&c) {
            result.push('\\');
        }
        result.push(c);
    }
    result
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

/// Send a plain-text notification to a list of Telegram users.
///
/// Used by the cron scheduler to deliver job results.
pub async fn send_notification(token: &str, user_ids: &[u64], text: &str) {
    let bot = Bot::new(token);
    for &uid in user_ids {
        let chat_id = ChatId(uid as i64);
        bot.send_message(chat_id, text).await.ok();
    }
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

    #[test]
    fn test_escape_md() {
        assert_eq!(escape_md("hello.world!"), "hello\\.world\\!");
        assert_eq!(escape_md("no specials"), "no specials");
    }
}
