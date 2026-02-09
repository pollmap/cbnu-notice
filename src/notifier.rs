use std::collections::HashMap;

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::time::{sleep, Duration};

use crate::category::Category;
use crate::db::Notice;

pub struct Notifier {
    bot: Bot,
    channel_id: String,
    log_channel_id: Option<String>,
    delay_ms: u64,
}

impl Notifier {
    pub fn new(bot: Bot, channel_id: String, log_channel_id: Option<String>, delay_ms: u64) -> Self {
        Self {
            bot,
            channel_id,
            log_channel_id,
            delay_ms,
        }
    }

    /// Bot 인스턴스 참조 (DM 엔진용).
    pub fn bot(&self) -> &Bot {
        &self.bot
    }

    /// Send a single notice to the specified channel (or default).
    pub async fn send_notice(&self, notice: &Notice, channel_override: Option<&str>) -> anyhow::Result<()> {
        let target_channel = channel_override.unwrap_or(&self.channel_id);
        let category = Category::from_str_tag(&notice.category);
        let cat_tag = if notice.category != "general" {
            format!("[{}] ", category.label())
        } else {
            String::new()
        };

        let date_str = notice
            .published
            .as_deref()
            .unwrap_or("날짜 미상");
        let author_str = notice
            .author
            .as_deref()
            .unwrap_or("작성자 미상");

        // Build message text (MarkdownV2 escaped)
        let text = format!(
            "{emoji} *{source}*\n\n{cat}{title}\n\n\u{1f4c5} {date} \\| \u{270d}\u{fe0f} {author}",
            emoji = category.emoji(),
            source = escape_markdown(&notice.source_display_name),
            cat = escape_markdown(&cat_tag),
            title = escape_markdown(&notice.title),
            date = escape_markdown(date_str),
            author = escape_markdown(author_str),
        );

        let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "\u{1f517} 원문 보기",
            reqwest::Url::parse(&notice.url)?,
        )]]);

        self.bot
            .send_message(ChatId(0), &text)
            .chat_id(target_channel.to_string())
            .parse_mode(ParseMode::MarkdownV2)
            .reply_markup(keyboard)
            .await
            .map_err(|e| anyhow::anyhow!("Telegram send failed: {}", e))?;

        Ok(())
    }

    /// Send a batch of notices, respecting rate limits and max count.
    /// `channel_map`: source_key → channel override.
    pub async fn send_batch(
        &self,
        notices: &[Notice],
        max: usize,
        channel_map: &HashMap<String, String>,
    ) -> anyhow::Result<usize> {
        let mut sent = 0;
        for notice in notices.iter().take(max) {
            let ch = channel_map.get(&notice.source_key).map(|s| s.as_str());
            match self.send_notice(notice, ch).await {
                Ok(()) => {
                    sent += 1;
                    tracing::info!(
                        notice_id = %notice.notice_id,
                        title = %notice.title,
                        "Sent notification"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        notice_id = %notice.notice_id,
                        error = %e,
                        "Failed to send notification"
                    );
                    // Don't break on individual failures; try the rest
                }
            }
            if sent < max {
                sleep(Duration::from_millis(self.delay_ms)).await;
            }
        }
        Ok(sent)
    }

    /// Send an error/status alert to the log channel.
    pub async fn send_error_alert(&self, message: &str) -> anyhow::Result<()> {
        let channel = match &self.log_channel_id {
            Some(ch) if !ch.is_empty() => ch.clone(),
            _ => {
                tracing::warn!("No log channel configured, skipping alert: {}", message);
                return Ok(());
            }
        };

        self.bot
            .send_message(ChatId(0), message)
            .chat_id(channel)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send alert: {}", e))?;

        Ok(())
    }

    /// Send a crawl summary to the log channel.
    pub async fn send_summary(&self, summary: &str) -> anyhow::Result<()> {
        self.send_error_alert(summary).await
    }
}

/// Escape special characters for Telegram MarkdownV2 format.
fn escape_markdown(text: &str) -> String {
    let special_chars = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut escaped = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if special_chars.contains(&ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_markdown() {
        assert_eq!(escape_markdown("hello"), "hello");
        assert_eq!(escape_markdown("test_var"), "test\\_var");
        assert_eq!(escape_markdown("[학사]"), "\\[학사\\]");
        assert_eq!(
            escape_markdown("2026.02.01 | author"),
            "2026\\.02\\.01 \\| author"
        );
    }
}
