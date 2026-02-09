use std::sync::{Arc, Mutex};

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommands;

use crate::config::SourceConfig;
use crate::db::Database;

/// 텔레그램 봇 명령어 정의.
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "충북대 공지 봇 명령어")]
pub enum Command {
    #[command(description = "봇 시작 / 등록")]
    Start,
    #[command(description = "도움말")]
    Help,
    #[command(description = "키워드 구독 (예: /sub 장학금)")]
    Sub(String),
    #[command(description = "키워드 구독 해제 (예: /unsub 장학금)")]
    Unsub(String),
    #[command(description = "학과 구독 (예: /dept biz)")]
    Dept(String),
    #[command(description = "학과 구독 해제")]
    Undept(String),
    #[command(description = "내 구독 현황")]
    Mysubs,
    #[command(description = "사용 가능한 소스 목록")]
    Sources,
    #[command(description = "봇 상태")]
    Status,
}

/// 봇 핸들러의 공유 상태.
#[derive(Clone)]
pub struct BotState {
    pub db: Arc<Mutex<Database>>,
    pub sources: Vec<SourceConfig>,
}

/// 명령어 핸들러.
pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<BotState>,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;

    // from이 없으면 (그룹 시스템 메시지 등) 무시
    let user = match msg.from.as_ref() {
        Some(u) => u,
        None => {
            bot.send_message(chat_id, "\u{26a0}\u{fe0f} DM으로 사용해주세요.")
                .await?;
            return Ok(());
        }
    };
    let user_id = user.id.0 as i64;

    // 모든 커맨드에서 사용자 자동 등록 (users 테이블에 없으면 DM 매칭 안 됨)
    {
        let db = state.db.lock().unwrap();
        let _ = db.register_user(
            user_id,
            user.username.as_deref(),
            Some(&user.first_name),
        );
    }

    let response = match cmd {
        Command::Start => handle_start(user_id, &user.first_name),
        Command::Help => handle_help(),
        Command::Sub(kw) => handle_sub(&state, user_id, &kw),
        Command::Unsub(kw) => handle_unsub(&state, user_id, &kw),
        Command::Dept(key) => handle_dept(&state, user_id, &key),
        Command::Undept(key) => handle_undept(&state, user_id, &key),
        Command::Mysubs => handle_mysubs(&state, user_id),
        Command::Sources => handle_sources(&state),
        Command::Status => handle_status(&state),
    };

    bot.send_message(chat_id, response)
        .parse_mode(ParseMode::Html)
        .await?;
    Ok(())
}

fn handle_start(user_id: i64, first_name: &str) -> String {
    let _ = user_id; // 이미 handle_command에서 등록 완료
    format!(
        "\u{1f44b} 안녕하세요, {}님!\n\n\
         <b>충북대 공지 알림 봇</b>에 등록되었습니다.\n\n\
         \u{1f4cc} <b>사용 방법:</b>\n\
         • /sub 장학금 → '장학금' 포함 공지 DM\n\
         • /dept biz → 경영학부 공지 DM\n\
         • /mysubs → 내 구독 현황\n\
         • /sources → 학과 목록\n\
         • /help → 전체 도움말",
        first_name
    )
}

fn handle_help() -> String {
    "\u{2139}\u{fe0f} <b>충북대 공지 봇 도움말</b>\n\n\
     <b>키워드 구독</b>\n\
     /sub &lt;키워드&gt; — 키워드가 포함된 공지를 DM으로 받기\n\
     /unsub &lt;키워드&gt; — 키워드 구독 해제\n\n\
     <b>학과 구독</b>\n\
     /dept &lt;학과코드&gt; — 특정 학과 공지를 DM으로 받기\n\
     /undept &lt;학과코드&gt; — 학과 구독 해제\n\n\
     <b>조회</b>\n\
     /mysubs — 내 구독 현황 보기\n\
     /sources — 사용 가능한 학과/소스 목록\n\
     /status — 봇 상태 확인\n\n\
     \u{1f4a1} <b>예시</b>\n\
     <code>/sub 장학금</code> → '장학금' 관련 공지 알림\n\
     <code>/dept biz</code> → 경영학부 공지 알림"
        .to_string()
}

fn handle_sub(state: &BotState, user_id: i64, keyword: &str) -> String {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return "\u{26a0}\u{fe0f} 키워드를 입력하세요.\n예: /sub 장학금".to_string();
    }
    if keyword.len() > 50 {
        return "\u{26a0}\u{fe0f} 키워드가 너무 깁니다 (최대 50자).".to_string();
    }

    let db = state.db.lock().unwrap();
    match db.add_keyword_sub(user_id, keyword) {
        Ok(true) => format!("\u{2705} '{}' 키워드 구독 완료!", keyword),
        Ok(false) => format!("\u{2139}\u{fe0f} '{}' 이미 구독 중입니다.", keyword),
        Err(e) => format!("\u{274c} 구독 실패: {}", e),
    }
}

fn handle_unsub(state: &BotState, user_id: i64, keyword: &str) -> String {
    let keyword = keyword.trim();
    if keyword.is_empty() {
        return "\u{26a0}\u{fe0f} 키워드를 입력하세요.\n예: /unsub 장학금".to_string();
    }

    let db = state.db.lock().unwrap();
    match db.remove_keyword_sub(user_id, keyword) {
        Ok(true) => format!("\u{2705} '{}' 구독 해제 완료!", keyword),
        Ok(false) => format!("\u{2139}\u{fe0f} '{}' 구독 중이 아닙니다.", keyword),
        Err(e) => format!("\u{274c} 해제 실패: {}", e),
    }
}

fn handle_dept(state: &BotState, user_id: i64, source_key: &str) -> String {
    let source_key = source_key.trim();
    if source_key.is_empty() {
        return "\u{26a0}\u{fe0f} 학과 코드를 입력하세요.\n/sources 로 목록을 확인하세요."
            .to_string();
    }

    // 유효한 소스인지 확인
    let valid = state.sources.iter().any(|s| s.key == source_key);
    if !valid {
        return format!(
            "\u{274c} '{}' 는 유효한 소스가 아닙니다.\n/sources 로 목록을 확인하세요.",
            source_key
        );
    }

    let db = state.db.lock().unwrap();
    match db.add_source_sub(user_id, source_key) {
        Ok(true) => {
            let display = state
                .sources
                .iter()
                .find(|s| s.key == source_key)
                .map(|s| s.display_name.as_str())
                .unwrap_or(source_key);
            format!("\u{2705} {} 구독 완료!", display)
        }
        Ok(false) => format!("\u{2139}\u{fe0f} '{}' 이미 구독 중입니다.", source_key),
        Err(e) => format!("\u{274c} 구독 실패: {}", e),
    }
}

fn handle_undept(state: &BotState, user_id: i64, source_key: &str) -> String {
    let source_key = source_key.trim();
    if source_key.is_empty() {
        return "\u{26a0}\u{fe0f} 학과 코드를 입력하세요.".to_string();
    }

    let db = state.db.lock().unwrap();
    match db.remove_source_sub(user_id, source_key) {
        Ok(true) => format!("\u{2705} '{}' 구독 해제 완료!", source_key),
        Ok(false) => format!("\u{2139}\u{fe0f} '{}' 구독 중이 아닙니다.", source_key),
        Err(e) => format!("\u{274c} 해제 실패: {}", e),
    }
}

fn handle_mysubs(state: &BotState, user_id: i64) -> String {
    let db = state.db.lock().unwrap();
    match db.get_user_subs(user_id) {
        Ok(subs) => {
            if subs.keywords.is_empty() && subs.sources.is_empty() {
                return "\u{1f4ed} 구독 중인 항목이 없습니다.\n\n\
                        /sub 키워드 또는 /dept 학과코드 로 구독하세요!"
                    .to_string();
            }

            let mut text = "\u{1f4cb} <b>내 구독 현황</b>\n\n".to_string();

            if !subs.keywords.is_empty() {
                text.push_str("\u{1f50d} <b>키워드 구독:</b>\n");
                for kw in &subs.keywords {
                    text.push_str(&format!("  • {}\n", kw));
                }
                text.push('\n');
            }

            if !subs.sources.is_empty() {
                text.push_str("\u{1f3eb} <b>학과 구독:</b>\n");
                for src in &subs.sources {
                    let display = state
                        .sources
                        .iter()
                        .find(|s| s.key == *src)
                        .map(|s| s.display_name.as_str())
                        .unwrap_or(src.as_str());
                    text.push_str(&format!("  • {} ({})\n", display, src));
                }
            }

            text
        }
        Err(e) => format!("\u{274c} 조회 실패: {}", e),
    }
}

fn handle_sources(state: &BotState) -> String {
    let mut text = "\u{1f4da} <b>사용 가능한 소스 목록</b>\n\n".to_string();
    for src in &state.sources {
        let status = if src.enabled { "\u{2705}" } else { "\u{23f8}\u{fe0f}" };
        text.push_str(&format!(
            "{} <code>{}</code> — {}\n",
            status, src.key, src.display_name
        ));
    }
    text.push_str("\n\u{1f4a1} /dept &lt;코드&gt; 로 구독하세요!");
    text
}

fn handle_status(state: &BotState) -> String {
    let db = state.db.lock().unwrap();
    match db.get_crawl_stats() {
        Ok(stats) => {
            if stats.is_empty() {
                return "\u{2139}\u{fe0f} 아직 크롤링 기록이 없습니다.".to_string();
            }

            let mut text = "\u{1f4ca} <b>봇 상태</b>\n\n".to_string();
            for stat in &stats {
                let display = state
                    .sources
                    .iter()
                    .find(|s| s.key == stat.source_key)
                    .map(|s| s.display_name.as_str())
                    .unwrap_or(&stat.source_key);
                let last = stat
                    .last_crawled
                    .as_deref()
                    .unwrap_or("없음");
                let err_icon = if stat.error_count > 0 {
                    format!(" \u{26a0}\u{fe0f}({})", stat.error_count)
                } else {
                    String::new()
                };
                text.push_str(&format!(
                    "• {} — 최근: {}{}\n",
                    display, last, err_icon
                ));
            }
            text
        }
        Err(e) => format!("\u{274c} 상태 조회 실패: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commands_parse() {
        // Verify BotCommands derive works
        let descriptions = Command::descriptions();
        let text = descriptions.to_string();
        assert!(text.contains("도움말"));
        assert!(text.contains("키워드 구독"));
    }
}
