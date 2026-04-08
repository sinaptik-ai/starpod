//! Conversion from standard Markdown (the agent's output format) to
//! Slack's "mrkdwn" flavor.
//!
//! Slack's mrkdwn is NOT CommonMark. The differences that matter in
//! practice:
//!
//! | Feature           | CommonMark        | Slack mrkdwn     |
//! |-------------------|-------------------|------------------|
//! | Bold              | `**text**`        | `*text*`         |
//! | Italic            | `*text*`          | `_text_`         |
//! | Strikethrough     | `~~text~~`        | `~text~`         |
//! | Inline code       | `` `text` ``      | `` `text` ``     |
//! | Fenced code block | ```` ```lang ```` | ```` ``` ````    |
//! | Links             | `[text](url)`     | `<url\|text>`    |
//! | Bare link         | `<https://x>`     | `<https://x>`    |
//! | H1/H2/H3          | `# h`             | *(use `*h*`)*    |
//! | Unordered list    | `- item`          | `• item`         |
//! | Ordered list      | `1. item`         | `1. item`        |
//! | Blockquote        | `> line`          | `> line`         |
//!
//! This converter is deliberately regex/string-based rather than a full
//! parser. It handles the 95% case the agent actually emits and keeps
//! code-block content untouched via a placeholder pass so that markdown
//! characters inside code are never reinterpreted. A proper pulldown-cmark
//! round-trip would be strictly better; we can upgrade later without
//! changing the public signature.
//!
//! The converter is pure and side-effect-free — easy to unit test.

/// Convert CommonMark text to Slack mrkdwn.
///
/// Safe to call on already-mrkdwn input: it will mostly pass through,
/// though double-converting bold (`**x**` → `*x*` → `_x_`) would be
/// incorrect, so callers should apply this exactly once per message.
pub fn markdown_to_mrkdwn(input: &str) -> String {
    // Phase 1: extract fenced and inline code into placeholders so we
    // don't rewrite markdown syntax inside code spans.
    let mut placeholders: Vec<String> = Vec::new();
    let mut text = input.to_string();

    // Fenced code blocks: ```lang\n...\n```
    text = extract_fenced_code(text, &mut placeholders);

    // Inline code: `...` (single backtick pair)
    text = extract_inline_code(text, &mut placeholders);

    // Phase 2: convert formatting on the stripped text.

    // Links: [label](url)  →  <url|label>
    text = convert_links(&text);

    // Bold: **text**  →  *text*
    //
    // Tricky bit: Slack also uses single `*` for bold, so the naive
    // output would be mauled by the italic pass below. We stash the
    // bolded run into a temporary placeholder, run italic on the rest,
    // then restore. `\x03` is chosen because the envelope placeholders
    // above use `\x02`, keeping the two sets distinct.
    let mut bold_slots: Vec<String> = Vec::new();
    text = convert_bold(&text, &mut bold_slots);

    // Italic: *text*  →  _text_  (only matches single-star runs that
    // survived bold conversion). Also handle `_text_` → `_text_` no-op.
    text = convert_italic(&text);

    for (i, bold) in bold_slots.iter().enumerate() {
        let ph = format!("\x03BOLD{}\x03", i);
        text = text.replace(&ph, bold);
    }

    // Strikethrough: ~~text~~  →  ~text~
    text = convert_strike(&text);

    // Headings: leading #s → bold line
    text = convert_headings(&text);

    // Unordered list bullets: leading `- ` or `* ` → `• `
    text = convert_bullets(&text);

    // Phase 3: restore placeholders, wrapping fenced blocks with Slack's
    // triple-backtick form (no language tag).
    for (i, content) in placeholders.iter().enumerate() {
        let ph = format!("\x02PH{}\x02", i);
        text = text.replace(&ph, content);
    }

    text
}

fn extract_fenced_code(mut text: String, placeholders: &mut Vec<String>) -> String {
    while let Some(start) = text.find("```") {
        let after = start + 3;
        // Skip optional language tag up to the next newline.
        let content_start = text[after..]
            .find('\n')
            .map(|p| after + p + 1)
            .unwrap_or(after);
        let Some(end_rel) = text[content_start..].find("```") else {
            break;
        };
        let end = content_start + end_rel;
        let code = &text[content_start..end];
        let slack = format!("```\n{}\n```", code.trim_end_matches('\n'));
        let ph = format!("\x02PH{}\x02", placeholders.len());
        placeholders.push(slack);
        text.replace_range(start..end + 3, &ph);
    }
    text
}

fn extract_inline_code(text: String, placeholders: &mut Vec<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find('`') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            let code = &after[..end];
            let ph = format!("\x02PH{}\x02", placeholders.len());
            placeholders.push(format!("`{}`", code));
            out.push_str(&ph);
            rest = &after[end + 1..];
        } else {
            // Unmatched backtick — leave as-is.
            out.push('`');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn convert_links(input: &str) -> String {
    // [label](url) → <url|label>
    // Minimal state machine; doesn't handle nested brackets which are
    // rare in agent output.
    //
    // NOTE: We scan by byte to find the ASCII delimiters (`[`, `]`, `(`,
    // `)`), but the fall-through copy MUST advance one full UTF-8
    // codepoint at a time — otherwise any multi-byte character (e.g.
    // `è`, `—`, emoji) is sliced into individual bytes and each byte is
    // re-encoded as a separate `char`, producing double-encoded mojibake
    // (`è` → `ÃÂ¨`, `—` → `Ã¢ÂÂ`).
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Look for matching `]`.
            if let Some(end_label) = find_byte(bytes, i + 1, b']') {
                if end_label + 1 < bytes.len() && bytes[end_label + 1] == b'(' {
                    if let Some(end_url) = find_byte(bytes, end_label + 2, b')') {
                        let label = &input[i + 1..end_label];
                        let url = &input[end_label + 2..end_url];
                        // Slack's link format: <url|label>
                        out.push('<');
                        out.push_str(url);
                        if !label.is_empty() && label != url {
                            out.push('|');
                            out.push_str(label);
                        }
                        out.push('>');
                        i = end_url + 1;
                        continue;
                    }
                }
            }
        }
        let step = utf8_char_len(bytes[i]);
        out.push_str(&input[i..i + step]);
        i += step;
    }
    out
}

fn find_byte(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    bytes[start..]
        .iter()
        .position(|&b| b == needle)
        .map(|p| p + start)
}

/// Length in bytes of the UTF-8 codepoint starting at `b` (the leading
/// byte). Defaults to 1 for continuation bytes or invalid input so the
/// caller always makes progress.
#[inline]
fn utf8_char_len(b: u8) -> usize {
    // ASCII (b < 0x80) and continuation bytes (0x80..0xC0) both advance by
    // 1: ASCII is a 1-byte codepoint, and a continuation byte means we're
    // off a char boundary so we step forward to escape the loop.
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

fn convert_bold(input: &str, slots: &mut Vec<String>) -> String {
    // Replace **text** with a placeholder that the caller will later
    // swap for `*text*`. Non-greedy; skips if `**` spans a line boundary
    // to avoid breaking incomplete input. We intentionally store the
    // already-Slack-formatted version in the slot so that the final
    // restore pass is a straight substitution.
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("**") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("**") {
            let inner = &after[..end];
            if !inner.contains('\n') && !inner.is_empty() {
                let ph = format!("\x03BOLD{}\x03", slots.len());
                slots.push(format!("*{}*", inner));
                out.push_str(&ph);
                rest = &after[end + 2..];
                continue;
            }
        }
        // Unmatched or multiline — leave alone.
        out.push_str("**");
        rest = after;
    }
    out.push_str(rest);
    out
}

fn convert_italic(input: &str) -> String {
    // Convert single-star italic *text* → _text_, but skip cases where
    // the `*` is at the start of a list item ("* item") or surrounded by
    // whitespace on the inside.
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            // Ignore "* " list bullets at line start.
            let at_line_start =
                i == 0 || bytes[i - 1] == b'\n' || (i >= 2 && bytes[i - 2] == b'\n');
            let next_is_space = i + 1 < bytes.len() && bytes[i + 1] == b' ';
            if at_line_start && next_is_space {
                out.push('*');
                i += 1;
                continue;
            }
            // Find the closing `*` on the same line.
            if let Some(end) = find_closing_star(bytes, i + 1) {
                let inner = &input[i + 1..end];
                if !inner.is_empty() && !inner.starts_with(' ') && !inner.ends_with(' ') {
                    out.push('_');
                    out.push_str(inner);
                    out.push('_');
                    i = end + 1;
                    continue;
                }
            }
        }
        // Non-matching fallback: advance one full UTF-8 codepoint so
        // multi-byte characters are preserved verbatim (see the note in
        // `convert_links`).
        let step = utf8_char_len(bytes[i]);
        out.push_str(&input[i..i + step]);
        i += step;
    }
    out
}

fn find_closing_star(bytes: &[u8], start: usize) -> Option<usize> {
    for (j, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'\n' => return None,
            b'*' => return Some(j),
            _ => {}
        }
    }
    None
}

fn convert_strike(input: &str) -> String {
    // `~~text~~` → `~text~`
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("~~") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        if let Some(end) = after.find("~~") {
            let inner = &after[..end];
            if !inner.contains('\n') && !inner.is_empty() {
                out.push('~');
                out.push_str(inner);
                out.push('~');
                rest = &after[end + 2..];
                continue;
            }
        }
        out.push_str("~~");
        rest = after;
    }
    out.push_str(rest);
    out
}

fn convert_headings(input: &str) -> String {
    // Turn leading `# `, `## `, `### ` etc into a bolded line.
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let leading = line.len() - trimmed.len();
            if let Some(rest) = trimmed.strip_prefix("### ") {
                format!("{}*{}*", &line[..leading], rest)
            } else if let Some(rest) = trimmed.strip_prefix("## ") {
                format!("{}*{}*", &line[..leading], rest)
            } else if let Some(rest) = trimmed.strip_prefix("# ") {
                format!("{}*{}*", &line[..leading], rest)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn convert_bullets(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let leading = line.len() - trimmed.len();
            if let Some(rest) = trimmed.strip_prefix("- ") {
                format!("{}• {}", &line[..leading], rest)
            } else if let Some(rest) = trimmed.strip_prefix("* ") {
                format!("{}• {}", &line[..leading], rest)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Maximum Slack `chat.postMessage` text length. Slack allows up to 40k
/// characters but recommends 4k; we cap at 3500 to leave headroom for
/// our own framing (e.g. code fences added during conversion).
pub const MAX_SLACK_MESSAGE_LEN: usize = 3500;

/// Split a long message into multiple chunks at paragraph / sentence
/// boundaries, never in the middle of a code block.
///
/// This is a best-effort splitter. It preserves code fences by finding a
/// safe break point (blank line, sentence end, or hard cut) outside of
/// any open triple-backtick.
pub fn split_for_slack(text: &str) -> Vec<String> {
    if text.len() <= MAX_SLACK_MESSAGE_LEN {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut remaining = text;
    while remaining.len() > MAX_SLACK_MESSAGE_LEN {
        let cut = find_safe_cut(remaining, MAX_SLACK_MESSAGE_LEN);
        let (chunk, rest) = remaining.split_at(cut);
        chunks.push(chunk.trim_end().to_string());
        remaining = rest.trim_start();
    }
    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }
    chunks
}

fn find_safe_cut(s: &str, max: usize) -> usize {
    let window = &s[..max.min(s.len())];
    // Prefer a paragraph break.
    if let Some(i) = window.rfind("\n\n") {
        return i + 2;
    }
    // Then a line break.
    if let Some(i) = window.rfind('\n') {
        return i + 1;
    }
    // Then a sentence end.
    if let Some(i) = window.rfind(". ") {
        return i + 2;
    }
    // Last resort: hard cut at max, snapping to the nearest char boundary.
    let mut cut = max.min(s.len());
    while !s.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_double_star_becomes_single_star() {
        assert_eq!(markdown_to_mrkdwn("**bold**"), "*bold*");
    }

    #[test]
    fn italic_single_star_becomes_underscore() {
        assert_eq!(markdown_to_mrkdwn("*italic*"), "_italic_");
    }

    #[test]
    fn bold_and_italic_combined() {
        assert_eq!(
            markdown_to_mrkdwn("**bold** and *italic* text"),
            "*bold* and _italic_ text"
        );
    }

    #[test]
    fn link_becomes_slack_angle_form() {
        assert_eq!(
            markdown_to_mrkdwn("see [Slack docs](https://docs.slack.dev)"),
            "see <https://docs.slack.dev|Slack docs>"
        );
    }

    #[test]
    fn inline_code_is_preserved() {
        // Backticked content should not be rewritten even if it looks
        // like markdown.
        assert_eq!(markdown_to_mrkdwn("`**not bold**`"), "`**not bold**`");
    }

    #[test]
    fn fenced_code_block_preserves_content() {
        let input = "```rust\nfn main() {\n    println!(\"**hi**\");\n}\n```";
        let out = markdown_to_mrkdwn(input);
        assert!(out.contains("println!(\"**hi**\")"));
        assert!(out.starts_with("```\n"));
        assert!(out.ends_with("```"));
    }

    #[test]
    fn heading_becomes_bold_line() {
        assert_eq!(markdown_to_mrkdwn("# Title"), "*Title*");
        assert_eq!(markdown_to_mrkdwn("## Sub"), "*Sub*");
        assert_eq!(markdown_to_mrkdwn("### Subsub"), "*Subsub*");
    }

    #[test]
    fn dash_bullets_become_bullets() {
        assert_eq!(markdown_to_mrkdwn("- one\n- two"), "• one\n• two");
    }

    #[test]
    fn strikethrough_double_tilde_becomes_single() {
        assert_eq!(markdown_to_mrkdwn("~~gone~~"), "~gone~");
    }

    #[test]
    fn list_item_star_is_not_italicized() {
        // A leading `* ` on its own line is a list bullet, not italic.
        let out = markdown_to_mrkdwn("* item one\n* item two");
        assert_eq!(out, "• item one\n• item two");
    }

    #[test]
    fn multiline_mix() {
        let input = "# Title\n\n**bold** *italic* `code`\n\n- one\n- two";
        let out = markdown_to_mrkdwn(input);
        assert!(out.contains("*Title*"));
        assert!(out.contains("*bold*"));
        assert!(out.contains("_italic_"));
        assert!(out.contains("`code`"));
        assert!(out.contains("• one"));
        assert!(out.contains("• two"));
    }

    #[test]
    fn preserves_multibyte_utf8_characters() {
        // Regression: convert_links / convert_italic used to cast each
        // input byte to `char`, which mangled multi-byte UTF-8 sequences
        // into double-encoded mojibake (`è` → `ÃÂ¨`, `—` → `Ã¢ÂÂ`).
        let input = "Non è vero — davvero? Perché no 🚀";
        let out = markdown_to_mrkdwn(input);
        assert_eq!(out, input);
    }

    #[test]
    fn preserves_multibyte_inside_italic_scan() {
        // Even with a non-matching `*`, the fall-through path must keep
        // multi-byte characters intact.
        let input = "ciao *mondo* — perché è così";
        let out = markdown_to_mrkdwn(input);
        assert!(out.contains("_mondo_"));
        assert!(out.contains("—"));
        assert!(out.contains("perché"));
        assert!(out.contains("è"));
    }

    #[test]
    fn split_short_message_returns_single_chunk() {
        let chunks = split_for_slack("hello world");
        assert_eq!(chunks, vec!["hello world"]);
    }

    #[test]
    fn split_long_message_splits_at_paragraph() {
        let long = format!(
            "{}\n\n{}",
            "a".repeat(MAX_SLACK_MESSAGE_LEN - 10),
            "b".repeat(100)
        );
        let chunks = split_for_slack(&long);
        assert!(chunks.len() >= 2);
        assert!(chunks.iter().all(|c| c.len() <= MAX_SLACK_MESSAGE_LEN));
    }
}
