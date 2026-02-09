use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup, ParseMode};
use tokio::time::{sleep, Duration};

use crate::category::Category;
use crate::db::{Database, Notice};

/// DM 매칭 + 발송 엔진.
/// 크롤링 후 새 공지를 구독자에게 개인 DM으로 전달한다.
pub struct DmEngine<'a> {
    bot: &'a Bot,
    db: &'a Database,
    delay_ms: u64,
}

/// DM 매칭 결과.
struct DmMatch {
    telegram_id: i64,
    match_type: String,  // "keyword" or "source"
    match_value: String,
}

impl<'a> DmEngine<'a> {
    pub fn new(bot: &'a Bot, db: &'a Database, delay_ms: u64) -> Self {
        Self { bot, db, delay_ms }
    }

    /// 최근 공지에 대해 구독 매칭 → DM 발송.
    /// 반환: 발송된 DM 수.
    pub async fn process(&self) -> anyhow::Result<u32> {
        // 최근 24시간 이내 공지 (이미 채널에 전송된 것들)
        let notices = self.db.get_recent_for_dm(100)?;
        if notices.is_empty() {
            return Ok(0);
        }

        // 전체 구독 데이터 로드
        let keyword_subs = self.db.get_all_keyword_subs()?;

        let mut total_sent = 0u32;

        for notice in &notices {
            let matches = self.find_matches(notice, &keyword_subs)?;

            for dm_match in &matches {
                // 이미 보냈으면 스킵
                if self.db.is_dm_sent(notice.id, dm_match.telegram_id)? {
                    continue;
                }

                match self
                    .send_dm(dm_match.telegram_id, notice, &dm_match.match_type, &dm_match.match_value)
                    .await
                {
                    Ok(()) => {
                        self.db.log_dm(
                            notice.id,
                            dm_match.telegram_id,
                            &dm_match.match_type,
                            Some(&dm_match.match_value),
                        )?;
                        total_sent += 1;
                        tracing::debug!(
                            telegram_id = dm_match.telegram_id,
                            notice_id = %notice.notice_id,
                            match_type = %dm_match.match_type,
                            "DM sent"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            telegram_id = dm_match.telegram_id,
                            error = %e,
                            "DM send failed (user may have blocked bot)"
                        );
                        // 403 Forbidden → 사용자가 봇을 차단한 경우
                        if e.to_string().contains("Forbidden") {
                            let _ = self.db.deactivate_user(dm_match.telegram_id);
                        }
                    }
                }

                // Rate limit 준수
                sleep(Duration::from_millis(self.delay_ms)).await;
            }
        }

        if total_sent > 0 {
            tracing::info!(count = total_sent, "DM delivery complete");
        }

        Ok(total_sent)
    }

    /// 공지에 매칭되는 구독자 목록 수집.
    fn find_matches(
        &self,
        notice: &Notice,
        keyword_subs: &[(i64, String)],
    ) -> anyhow::Result<Vec<DmMatch>> {
        let mut matches: Vec<DmMatch> = Vec::new();
        let mut seen_users = std::collections::HashSet::new();

        let title_lower = notice.title.to_lowercase();

        // 1. 키워드 매칭
        for (telegram_id, keyword) in keyword_subs {
            if title_lower.contains(&keyword.to_lowercase()) {
                if seen_users.insert(*telegram_id) {
                    matches.push(DmMatch {
                        telegram_id: *telegram_id,
                        match_type: "keyword".to_string(),
                        match_value: keyword.clone(),
                    });
                }
            }
        }

        // 2. 소스(학과) 매칭
        let source_subscribers = self.db.get_source_subscribers(&notice.source_key)?;
        for telegram_id in source_subscribers {
            if seen_users.insert(telegram_id) {
                matches.push(DmMatch {
                    telegram_id,
                    match_type: "source".to_string(),
                    match_value: notice.source_key.clone(),
                });
            }
        }

        Ok(matches)
    }

    /// 개별 DM 메시지 전송.
    async fn send_dm(
        &self,
        telegram_id: i64,
        notice: &Notice,
        match_type: &str,
        match_value: &str,
    ) -> anyhow::Result<()> {
        let category = Category::from_str_tag(&notice.category);
        let match_label = match match_type {
            "keyword" => format!("\u{1f50d} 키워드: {}", match_value),
            "source" => format!("\u{1f3eb} 학과: {}", notice.source_display_name),
            _ => String::new(),
        };

        let text = format!(
            "{emoji} <b>{source}</b>\n\n\
             {title}\n\n\
             {match_label}\n\
             \u{1f4c5} {date}",
            emoji = category.emoji(),
            source = html_escape(&notice.source_display_name),
            title = html_escape(&notice.title),
            match_label = html_escape(&match_label),
            date = html_escape(notice.published.as_deref().unwrap_or("날짜 미상")),
        );

        let keyboard = InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
            "\u{1f517} 원문 보기",
            reqwest::Url::parse(&notice.url)?,
        )]]);

        self.bot
            .send_message(ChatId(telegram_id), &text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await
            .map_err(|e| anyhow::anyhow!("DM failed: {}", e))?;

        Ok(())
    }
}

/// HTML 특수문자 이스케이프.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("hello"), "hello");
        assert_eq!(html_escape("<b>bold</b>"), "&lt;b&gt;bold&lt;/b&gt;");
        assert_eq!(html_escape("A & B"), "A &amp; B");
    }
}
