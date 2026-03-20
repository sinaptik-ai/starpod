use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio_stream::StreamExt;
use tracing::{debug, error, warn};

use agent_sdk::{ContentBlock, Message};
use starpod_agent::StarpodAgent;
use starpod_auth::AuthStore;
use starpod_core::{Attachment, ChatMessage};

/// Maximum Telegram message length.
const MAX_MSG_LEN: usize = 4096;

/// Message delivery config passed through teloxide DI.
#[derive(Clone)]
struct StreamConfig {
    stream_mode: String,
}

/// Agent display name for greetings.
#[derive(Clone)]
struct AgentName(String);

/// Run the Telegram bot with a pre-built agent and auth store.
///
/// Telegram users must be linked to a database user via `AuthStore::link_telegram`.
/// Unlinked users get a "not linked" message. `/start` always works and shows
/// the user's Telegram ID for linking.
pub async fn run_with_agent_and_auth(
    agent: Arc<StarpodAgent>,
    auth: Arc<AuthStore>,
    token: String,
) -> starpod_core::Result<()> {
    let stream_cfg = StreamConfig {
        stream_mode: agent.config().channels.telegram.as_ref()
            .map(|t| t.stream_mode.clone())
            .unwrap_or_else(|| "final_only".to_string()),
    };
    let agent_name = AgentName(agent.config().agent_name.clone());

    let bot = Bot::new(&token);

    let handler = Update::filter_message().endpoint(handle_message);

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![agent, auth, stream_cfg, agent_name])
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

/// Download a Telegram file by file_id and return its bytes.
async fn download_telegram_file(
    bot: &Bot,
    file_id: &str,
) -> std::result::Result<Vec<u8>, String> {
    let file = bot
        .get_file(file_id)
        .await
        .map_err(|e| format!("get_file failed: {e}"))?;

    let url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        bot.token(),
        file.path
    );
    let bytes = reqwest::get(&url)
        .await
        .map_err(|e| format!("download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("read bytes failed: {e}"))?;

    Ok(bytes.to_vec())
}

/// Infer MIME type from a filename extension.
fn mime_from_filename(name: &str) -> String {
    let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "txt" | "text" => "text/plain",
        "json" => "application/json",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
    .to_string()
}

async fn handle_message(
    bot: Bot,
    msg: TeloxideMessage,
    agent: Arc<StarpodAgent>,
    auth: Arc<AuthStore>,
    stream_cfg: StreamConfig,
    agent_name: AgentName,
) -> Result<(), teloxide::RequestError> {
    // Extract text — may come from caption on photo/document messages
    let text = msg
        .text()
        .or_else(|| msg.caption())
        .unwrap_or("")
        .to_string();

    // Extract attachments from photos and documents
    let agent_config = agent.config();
    let att_config = &agent_config.attachments;
    let mut attachments: Vec<Attachment> = Vec::new();

    // Handle photos (Telegram sends multiple sizes; pick the largest)
    if let Some(photos) = msg.photo() {
        if let Some(largest) = photos.last() {
            match download_telegram_file(&bot, &largest.file.id).await {
                Ok(bytes) => {
                    let file_name = "photo.jpg";
                    match att_config.validate(file_name, bytes.len()) {
                        Ok(()) => {
                            use base64::Engine;
                            attachments.push(Attachment {
                                file_name: file_name.to_string(),
                                mime_type: "image/jpeg".to_string(),
                                data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                            });
                        }
                        Err(reason) => warn!("{}", reason),
                    }
                }
                Err(e) => warn!(error = %e, "Failed to download Telegram photo"),
            }
        }
    }

    // Handle documents (files)
    if let Some(doc) = msg.document() {
        match download_telegram_file(&bot, &doc.file.id).await {
            Ok(bytes) => {
                let file_name = doc
                    .file_name
                    .clone()
                    .unwrap_or_else(|| "document".to_string());
                match att_config.validate(&file_name, bytes.len()) {
                    Ok(()) => {
                        let mime_type = doc
                            .mime_type
                            .as_ref()
                            .map(|m| m.to_string())
                            .unwrap_or_else(|| mime_from_filename(&file_name));
                        use base64::Engine;
                        attachments.push(Attachment {
                            file_name,
                            mime_type,
                            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                        });
                    }
                    Err(reason) => warn!("{}", reason),
                }
            }
            Err(e) => warn!(error = %e, "Failed to download Telegram document"),
        }
    }

    // If no text and no attachments, nothing to do
    if text.is_empty() && attachments.is_empty() {
        return Ok(());
    }

    let from_user = msg.from.as_ref();
    let telegram_id = from_user.map(|u| u.id.0 as i64);
    let username = from_user.and_then(|u| u.username.clone());
    let chat_id = msg.chat.id;

    // /start always works — it shows the user their Telegram ID for linking
    if text == "/start" {
        let name = &agent_name.0;
        let mut greeting = format!("Hello\\! I'm {}, your personal AI assistant\\.", escape_md(name));
        if let Some(id) = telegram_id {
            greeting.push_str(&format!("\n\nYour Telegram ID: `{}`", id));
        }
        if let Some(ref uname) = username {
            greeting.push_str(&format!("\nYour username: `{}`", escape_md(uname)));
        }
        greeting.push_str("\nAsk your admin to link your Telegram account to start chatting\\.");
        bot.send_message(chat_id, &greeting)
            .parse_mode(ParseMode::MarkdownV2)
            .await
            .or_else(|_| {
                let fallback = format!(
                    "Hello! I'm {}, your personal AI assistant.\n\nYour Telegram ID: {}\nYour username: {}\nAsk your admin to link your Telegram account to start chatting.",
                    name,
                    telegram_id.map(|id| id.to_string()).unwrap_or_default(),
                    username.as_deref().unwrap_or("(not set)")
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

    // Authenticate via Telegram link in the auth DB
    let Some(tg_id) = telegram_id else {
        bot.send_message(chat_id, "Could not determine your Telegram ID.")
            .await
            .ok();
        return Ok(());
    };

    let auth_user = match auth.authenticate_telegram(tg_id).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            debug!(
                telegram_id = tg_id,
                chat_id = %chat_id,
                "Rejected message from unlinked Telegram user"
            );
            bot.send_message(
                chat_id,
                "Your Telegram account is not linked. Ask your admin to link it, or send /start to see your Telegram ID.",
            )
            .await
            .ok();
            return Ok(());
        }
        Err(e) => {
            error!(error = %e, "Telegram auth error");
            bot.send_message(chat_id, "Authentication error. Please try again later.")
                .await
                .ok();
            return Ok(());
        }
    };

    let user_id_str = Some(auth_user.id);

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

    if stream_cfg.stream_mode == "all_messages" {
        handle_all_messages(&bot, chat_id, &agent, &text, user_id_str, attachments).await
    } else {
        handle_final_only(&bot, chat_id, &agent, &text, user_id_str, attachments).await
    }
}

/// Build a `ChatMessage` for Telegram.
fn build_chat_msg(
    text: &str,
    user_id_str: Option<String>,
    chat_id: ChatId,
    attachments: Vec<Attachment>,
) -> ChatMessage {
    ChatMessage {
        text: text.to_string(),
        user_id: user_id_str,
        channel_id: Some("telegram".into()),
        channel_session_key: Some(chat_id.0.to_string()),
        attachments,
    }
}

/// Extract text content from an assistant message's content blocks.
fn extract_assistant_text(content: &[ContentBlock]) -> String {
    let mut text = String::new();
    for block in content {
        if let ContentBlock::Text { text: t } = block {
            if !t.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
    }
    text
}

/// Final-only mode (default): stream internally, send only the last
/// assistant message when done. Each message sent as standalone.
async fn handle_final_only(
    bot: &Bot,
    chat_id: ChatId,
    agent: &Arc<StarpodAgent>,
    text: &str,
    user_id_str: Option<String>,
    attachments: Vec<Attachment>,
) -> Result<(), teloxide::RequestError> {
    let uid = user_id_str.clone();
    let chat_msg = build_chat_msg(text, user_id_str, chat_id, attachments);
    let (mut stream, session_id, _followup_tx) = match agent.chat_stream(&chat_msg).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to start stream");
            bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                .await?;
            return Ok(());
        }
    };

    let mut last_assistant_text = String::new();
    let mut all_text = String::new();
    let mut result_msg = None;

    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Assistant(assistant)) => {
                let t = extract_assistant_text(&assistant.content);
                if !t.is_empty() {
                    last_assistant_text = t.clone();
                    if !all_text.is_empty() {
                        all_text.push('\n');
                    }
                    all_text.push_str(&t);
                }
            }
            Ok(Message::Result(result)) => {
                if last_assistant_text.is_empty() {
                    if let Some(t) = &result.result {
                        last_assistant_text = t.clone();
                        all_text = t.clone();
                    }
                }
                result_msg = Some(result);
            }
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "Stream error");
                bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                    .await?;
                return Ok(());
            }
        }
    }

    // Send only the last assistant message
    if last_assistant_text.is_empty() {
        bot.send_message(chat_id, "(no response)").await.ok();
    } else {
        send_response(bot, chat_id, &last_assistant_text).await;
    }

    // Finalize (record usage, daily log)
    if let Some(ref result) = result_msg {
        agent.finalize_chat(&session_id, text, &all_text, result, uid.as_deref()).await;
    }

    Ok(())
}

/// All-messages mode: send each assistant text message as a standalone
/// Telegram message as it arrives. Tool-use messages are excluded.
async fn handle_all_messages(
    bot: &Bot,
    chat_id: ChatId,
    agent: &Arc<StarpodAgent>,
    text: &str,
    user_id_str: Option<String>,
    attachments: Vec<Attachment>,
) -> Result<(), teloxide::RequestError> {
    let uid = user_id_str.clone();
    let chat_msg = build_chat_msg(text, user_id_str, chat_id, attachments);
    let (mut stream, session_id, _followup_tx) = match agent.chat_stream(&chat_msg).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "Failed to start stream");
            bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                .await?;
            return Ok(());
        }
    };

    let mut all_text = String::new();
    let mut result_msg = None;

    while let Some(msg_result) = stream.next().await {
        match msg_result {
            Ok(Message::Assistant(assistant)) => {
                let t = extract_assistant_text(&assistant.content);
                if !t.is_empty() {
                    if !all_text.is_empty() {
                        all_text.push('\n');
                    }
                    all_text.push_str(&t);
                    // Send immediately as standalone message
                    send_response(bot, chat_id, &t).await;
                }
            }
            Ok(Message::Result(result)) => {
                if all_text.is_empty() {
                    if let Some(t) = &result.result {
                        all_text = t.clone();
                        send_response(bot, chat_id, t).await;
                    }
                }
                result_msg = Some(result);
            }
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "Stream error");
                bot.send_message(chat_id, format!("Sorry, an error occurred: {}", e))
                    .await?;
                return Ok(());
            }
        }
    }

    if all_text.is_empty() {
        bot.send_message(chat_id, "(no response)").await.ok();
    }

    // Finalize (record usage, daily log)
    if let Some(ref result) = result_msg {
        agent.finalize_chat(&session_id, text, &all_text, result, uid.as_deref()).await;
    }

    Ok(())
}

/// Send a (possibly long) response as one or more messages.
/// Converts markdown to Telegram HTML first, with plain-text fallback.
async fn send_response(bot: &Bot, chat_id: ChatId, text: &str) {
    let html = markdown_to_telegram_html(text);
    let chunks = split_message(&html, MAX_MSG_LEN);
    for chunk in &chunks {
        let sent = bot
            .send_message(chat_id, chunk)
            .parse_mode(ParseMode::Html)
            .await;
        if sent.is_err() {
            // Fallback: send as plain text
            bot.send_message(chat_id, chunk).await.ok();
        }
    }
}

/// Escape HTML special characters.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert standard markdown to Telegram-compatible HTML.
///
/// Handles: fenced code blocks, inline code, bold, italic, strikethrough,
/// headings, and links. Falls back gracefully for unsupported syntax.
fn markdown_to_telegram_html(input: &str) -> String {
    // Phase 1: extract code blocks and inline code into placeholders
    let mut placeholders: Vec<String> = Vec::new();
    let mut text = input.to_string();

    // Fenced code blocks: ```lang\n...\n```
    loop {
        let Some(start) = text.find("```") else {
            break;
        };
        let after_fence = start + 3;
        let rest = &text[after_fence..];
        // Skip optional language tag (up to first newline)
        let content_start = rest
            .find('\n')
            .map(|p| after_fence + p + 1)
            .unwrap_or(after_fence);
        let Some(end_offset) = text[content_start..].find("```") else {
            break;
        };
        let end = content_start + end_offset;
        let code = text[content_start..end].trim_end_matches('\n');
        let html = format!("<pre>{}</pre>", escape_html(code));
        let ph = format!("\x02PH{}\x02", placeholders.len());
        placeholders.push(html);
        text = format!("{}{}{}", &text[..start], ph, &text[end + 3..]);
    }

    // Inline code: `...`
    let mut buf = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('`') {
        buf.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            let code = &after[..end];
            let html = format!("<code>{}</code>", escape_html(code));
            let ph = format!("\x02PH{}\x02", placeholders.len());
            placeholders.push(html);
            buf.push_str(&ph);
            rest = &after[end + 1..];
        } else {
            buf.push('`');
            rest = after;
        }
    }
    buf.push_str(rest);
    text = buf;

    // Phase 2: escape HTML in remaining text
    text = escape_html(&text);

    // Phase 3: convert markdown formatting

    // Bold: **text** → <b>text</b>  (must run before italic)
    let mut buf = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("**") {
        buf.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("**") {
            buf.push_str("<b>");
            buf.push_str(&after[..end]);
            buf.push_str("</b>");
            rest = &after[end + 2..];
        } else {
            buf.push_str("**");
            rest = after;
        }
    }
    buf.push_str(rest);
    text = buf;

    // Strikethrough: ~~text~~ → <s>text</s>
    let mut buf = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("~~") {
        buf.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("~~") {
            buf.push_str("<s>");
            buf.push_str(&after[..end]);
            buf.push_str("</s>");
            rest = &after[end + 2..];
        } else {
            buf.push_str("~~");
            rest = after;
        }
    }
    buf.push_str(rest);
    text = buf;

    // Italic: *text* → <i>text</i>  (after bold has been removed)
    let mut buf = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('*') {
        buf.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        // Bullet point: * at line start followed by space
        let at_line_start = start == 0 || rest.as_bytes()[start - 1] == b'\n';
        if at_line_start && after.starts_with(' ') {
            buf.push('*');
            rest = after;
            continue;
        }
        // Not italic if followed by whitespace or end
        if after.is_empty() || after.starts_with(' ') || after.starts_with('\n') {
            buf.push('*');
            rest = after;
            continue;
        }
        if let Some(end) = after.find('*') {
            if end > 0 && after.as_bytes()[end - 1] != b' ' {
                buf.push_str("<i>");
                buf.push_str(&after[..end]);
                buf.push_str("</i>");
                rest = &after[end + 1..];
            } else {
                buf.push('*');
                rest = after;
            }
        } else {
            buf.push('*');
            rest = after;
        }
    }
    buf.push_str(rest);
    text = buf;

    // Headings: # at line start → bold
    let lines: Vec<&str> = text.split('\n').collect();
    let mut new_lines: Vec<String> = Vec::with_capacity(lines.len());
    for line in &lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let content = trimmed.trim_start_matches('#').trim_start();
            new_lines.push(format!("<b>{}</b>", content));
        } else {
            new_lines.push(line.to_string());
        }
    }
    text = new_lines.join("\n");

    // Links: [text](url) → <a href="url">text</a>
    let mut buf = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('[') {
        buf.push_str(&rest[..start]);
        let after_bracket = &rest[start + 1..];
        if let Some(bracket_end) = after_bracket.find(']') {
            let link_text = &after_bracket[..bracket_end];
            let after_close = &after_bracket[bracket_end + 1..];
            if after_close.starts_with('(') {
                if let Some(paren_end) = after_close[1..].find(')') {
                    let url = &after_close[1..1 + paren_end];
                    // Restore &amp; in URLs back to &
                    let url = url.replace("&amp;", "&");
                    buf.push_str(&format!("<a href=\"{}\">{}</a>", url, link_text));
                    rest = &after_close[1 + paren_end + 1..];
                    continue;
                }
            }
            buf.push('[');
            rest = after_bracket;
        } else {
            buf.push('[');
            rest = after_bracket;
        }
    }
    buf.push_str(rest);
    text = buf;

    // Phase 4: restore placeholders
    for (i, html) in placeholders.iter().enumerate() {
        // Placeholders were HTML-escaped in phase 2, so \x02 became... no,
        // \x02 is not an HTML special char, so it survives escape_html unchanged.
        let ph = format!("\x02PH{}\x02", i);
        text = text.replace(&ph, html);
    }

    text
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

        // Find a safe byte boundary for slicing
        let mut safe_max = max_len;
        while safe_max > 0 && !remaining.is_char_boundary(safe_max) { safe_max -= 1; }

        // Find a good split point (newline before max_len)
        let split_at = remaining[..safe_max]
            .rfind('\n')
            .unwrap_or(safe_max);

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

    #[test]
    fn test_md_to_html_plain() {
        assert_eq!(markdown_to_telegram_html("hello world"), "hello world");
    }

    #[test]
    fn test_md_to_html_bold() {
        assert_eq!(
            markdown_to_telegram_html("this is **bold** text"),
            "this is <b>bold</b> text"
        );
    }

    #[test]
    fn test_md_to_html_italic() {
        assert_eq!(
            markdown_to_telegram_html("this is *italic* text"),
            "this is <i>italic</i> text"
        );
    }

    #[test]
    fn test_md_to_html_code_block() {
        assert_eq!(
            markdown_to_telegram_html("before\n```rust\nfn main() {}\n```\nafter"),
            "before\n<pre>fn main() {}</pre>\nafter"
        );
    }

    #[test]
    fn test_md_to_html_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("use `foo()` here"),
            "use <code>foo()</code> here"
        );
    }

    #[test]
    fn test_md_to_html_heading() {
        assert_eq!(
            markdown_to_telegram_html("## My Heading\nsome text"),
            "<b>My Heading</b>\nsome text"
        );
    }

    #[test]
    fn test_md_to_html_link() {
        assert_eq!(
            markdown_to_telegram_html("click [here](https://example.com)"),
            "click <a href=\"https://example.com\">here</a>"
        );
    }

    #[test]
    fn test_md_to_html_escapes_html() {
        assert_eq!(
            markdown_to_telegram_html("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
    }

    #[test]
    fn test_md_to_html_code_preserves_special() {
        assert_eq!(
            markdown_to_telegram_html("run `x < 5 && y > 3`"),
            "run <code>x &lt; 5 &amp;&amp; y &gt; 3</code>"
        );
    }

    #[test]
    fn test_md_to_html_bullet_not_italic() {
        let input = "* item one\n* item two";
        let html = markdown_to_telegram_html(input);
        assert!(!html.contains("<i>"), "Bullet points should not become italic");
        assert!(html.contains("* item one"));
    }

    #[test]
    fn test_md_to_html_strikethrough() {
        assert_eq!(
            markdown_to_telegram_html("this is ~~deleted~~ text"),
            "this is <s>deleted</s> text"
        );
    }

    #[test]
    fn test_split_message_multibyte_safe() {
        // Create a string of multi-byte chars that forces a split mid-character
        // Each "🌟" is 4 bytes. 10 of them = 40 bytes.
        let emoji_str = "🌟".repeat(10);
        assert_eq!(emoji_str.len(), 40);

        // Split at 6 — would fall inside the second emoji (bytes 4-7)
        let chunks = split_message(&emoji_str, 6);
        // Should not panic and each chunk should be valid UTF-8
        for chunk in &chunks {
            assert!(chunk.len() <= 6 || chunk == "🌟"); // single emoji is 4 bytes, fits
        }
        // Reassembled should equal original
        let reassembled: String = chunks.join("");
        assert_eq!(reassembled, emoji_str);
    }

    #[test]
    fn test_split_message_ascii() {
        let text = "Hello World";
        let chunks = split_message(text, 5);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.len() <= 5);
        }
    }
}
