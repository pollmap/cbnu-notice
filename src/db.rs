use chrono::Utc;
use rusqlite::{params, Connection};

use crate::category::Category;
use crate::parser::RawNotice;

/// SQLite datetime() 호환 포맷으로 현재 시간 반환.
/// RFC3339 대신 "YYYY-MM-DD HH:MM:SS" 형식을 사용해야
/// SQLite의 datetime('now', '-1 day') 등과 올바르게 비교된다.
fn now_sqlite() -> String {
    Utc::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 사용자 구독 정보.
#[derive(Debug, Clone)]
pub struct UserSubs {
    pub keywords: Vec<String>,
    pub sources: Vec<String>,
}

/// 크롤 상태 통계.
#[derive(Debug, Clone)]
pub struct CrawlStat {
    pub source_key: String,
    pub last_crawled: Option<String>,
    pub error_count: u32,
}

/// A stored notice from the database.
#[derive(Debug, Clone)]
pub struct Notice {
    pub id: i64,
    #[allow(dead_code)]
    pub source_key: String,
    pub notice_id: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub category: String,
    pub published: Option<String>,
    pub source_display_name: String,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn init(path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS notices (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                source_key  TEXT NOT NULL,
                notice_id   TEXT NOT NULL,
                title       TEXT NOT NULL,
                url         TEXT NOT NULL,
                author      TEXT,
                category    TEXT DEFAULT 'general',
                published   TEXT,
                deadline    TEXT,
                crawled_at  TEXT NOT NULL DEFAULT (datetime('now')),
                notified    INTEGER DEFAULT 0,
                UNIQUE(source_key, notice_id)
            );
            CREATE INDEX IF NOT EXISTS idx_pending ON notices(notified) WHERE notified = 0;

            CREATE TABLE IF NOT EXISTS crawl_state (
                source_key     TEXT PRIMARY KEY,
                last_crawled   TEXT,
                last_notice_id TEXT,
                error_count    INTEGER DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS users (
                telegram_id  INTEGER PRIMARY KEY,
                username     TEXT,
                first_name   TEXT,
                registered   TEXT NOT NULL DEFAULT (datetime('now')),
                is_active    INTEGER DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS keyword_subs (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                telegram_id  INTEGER NOT NULL REFERENCES users(telegram_id),
                keyword      TEXT NOT NULL,
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(telegram_id, keyword)
            );

            CREATE TABLE IF NOT EXISTS source_subs (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                telegram_id  INTEGER NOT NULL REFERENCES users(telegram_id),
                source_key   TEXT NOT NULL,
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(telegram_id, source_key)
            );

            CREATE TABLE IF NOT EXISTS dm_log (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                notice_id    INTEGER NOT NULL,
                telegram_id  INTEGER NOT NULL,
                match_type   TEXT NOT NULL,
                match_value  TEXT,
                sent_at      TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(notice_id, telegram_id)
            );
            CREATE INDEX IF NOT EXISTS idx_dm_log ON dm_log(notice_id);
            ",
        )?;

        Ok(Self { conn })
    }

    /// Insert a new notice. Returns true if it was actually new (not a duplicate).
    pub fn insert_if_new(
        &self,
        source_key: &str,
        notice: &RawNotice,
        display_name: &str,
    ) -> anyhow::Result<bool> {
        let category = Category::classify(&notice.title);
        let now = now_sqlite();

        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO notices (source_key, notice_id, title, url, author, category, published, crawled_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                source_key,
                notice.notice_id,
                notice.title,
                notice.url,
                notice.author,
                category.as_str(),
                notice.date,
                now,
            ],
        )?;

        // Store display_name mapping in crawl_state for later use
        let _ = self.conn.execute(
            "INSERT INTO crawl_state (source_key, last_crawled) VALUES (?1, ?2)
             ON CONFLICT(source_key) DO NOTHING",
            params![source_key, now],
        );

        // We don't actually use display_name in the DB, but we pass it through via Notice
        let _ = display_name;

        Ok(affected > 0)
    }

    /// Get pending notifications (notified=0), most recent first.
    pub fn get_pending(&self, limit: usize, source_display_names: &std::collections::HashMap<String, String>) -> anyhow::Result<Vec<Notice>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_key, notice_id, title, url, author, category, published
             FROM notices WHERE notified = 0 ORDER BY crawled_at DESC LIMIT ?1",
        )?;

        let notices = stmt.query_map(params![limit as i64], |row| {
            let source_key: String = row.get(1)?;
            let display_name = source_display_names
                .get(&source_key)
                .cloned()
                .unwrap_or_else(|| source_key.clone());
            Ok(Notice {
                id: row.get(0)?,
                source_key,
                notice_id: row.get(2)?,
                title: row.get(3)?,
                url: row.get(4)?,
                author: row.get(5)?,
                category: row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "general".into()),
                published: row.get(7)?,
                source_display_name: display_name,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(notices)
    }

    /// Mark a notice as notified.
    pub fn mark_notified(&self, id: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE notices SET notified = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// Mark all notices from a source as notified (seed 모드용).
    pub fn mark_all_notified(&self, source_key: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE notices SET notified = 1 WHERE source_key = ?1 AND notified = 0",
            params![source_key],
        )?;
        Ok(())
    }

    /// Update crawl state after successful crawl.
    pub fn update_crawl_state(&self, source_key: &str, last_id: Option<&str>) -> anyhow::Result<()> {
        let now = now_sqlite();
        self.conn.execute(
            "INSERT INTO crawl_state (source_key, last_crawled, last_notice_id, error_count)
             VALUES (?1, ?2, ?3, 0)
             ON CONFLICT(source_key) DO UPDATE SET
               last_crawled = ?2,
               last_notice_id = COALESCE(?3, last_notice_id),
               error_count = 0",
            params![source_key, now, last_id],
        )?;
        Ok(())
    }

    /// Increment error count and return the new count.
    pub fn increment_error(&self, source_key: &str) -> anyhow::Result<u32> {
        let now = now_sqlite();
        self.conn.execute(
            "INSERT INTO crawl_state (source_key, last_crawled, error_count)
             VALUES (?1, ?2, 1)
             ON CONFLICT(source_key) DO UPDATE SET
               last_crawled = ?2,
               error_count = error_count + 1",
            params![source_key, now],
        )?;

        let count: u32 = self.conn.query_row(
            "SELECT error_count FROM crawl_state WHERE source_key = ?1",
            params![source_key],
            |row| row.get(0),
        )?;

        Ok(count)
    }

    /// Reset error count for a source (used in tests and Phase 2).
    #[allow(dead_code)]
    pub fn reset_error(&self, source_key: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE crawl_state SET error_count = 0 WHERE source_key = ?1",
            params![source_key],
        )?;
        Ok(())
    }

    // ── Phase 2: 구독 / DM 관련 메서드 ─────────────────────────────

    /// 사용자 등록 (첫 /start 시 호출). 이미 있으면 활성화만 갱신.
    pub fn register_user(
        &self,
        telegram_id: i64,
        username: Option<&str>,
        first_name: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO users (telegram_id, username, first_name)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(telegram_id) DO UPDATE SET
               username = COALESCE(?2, username),
               first_name = COALESCE(?3, first_name),
               is_active = 1",
            params![telegram_id, username, first_name],
        )?;
        Ok(())
    }

    /// 키워드 구독 추가. 이미 있으면 무시.
    pub fn add_keyword_sub(&self, telegram_id: i64, keyword: &str) -> anyhow::Result<bool> {
        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO keyword_subs (telegram_id, keyword) VALUES (?1, ?2)",
            params![telegram_id, keyword],
        )?;
        Ok(affected > 0)
    }

    /// 키워드 구독 제거.
    pub fn remove_keyword_sub(&self, telegram_id: i64, keyword: &str) -> anyhow::Result<bool> {
        let affected = self.conn.execute(
            "DELETE FROM keyword_subs WHERE telegram_id = ?1 AND keyword = ?2",
            params![telegram_id, keyword],
        )?;
        Ok(affected > 0)
    }

    /// 소스(학과) 구독 추가.
    pub fn add_source_sub(&self, telegram_id: i64, source_key: &str) -> anyhow::Result<bool> {
        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO source_subs (telegram_id, source_key) VALUES (?1, ?2)",
            params![telegram_id, source_key],
        )?;
        Ok(affected > 0)
    }

    /// 소스(학과) 구독 제거.
    pub fn remove_source_sub(&self, telegram_id: i64, source_key: &str) -> anyhow::Result<bool> {
        let affected = self.conn.execute(
            "DELETE FROM source_subs WHERE telegram_id = ?1 AND source_key = ?2",
            params![telegram_id, source_key],
        )?;
        Ok(affected > 0)
    }

    /// 특정 사용자의 전체 구독 정보 조회.
    pub fn get_user_subs(&self, telegram_id: i64) -> anyhow::Result<UserSubs> {
        let mut kw_stmt = self.conn.prepare(
            "SELECT keyword FROM keyword_subs WHERE telegram_id = ?1 ORDER BY keyword",
        )?;
        let keywords: Vec<String> = kw_stmt
            .query_map(params![telegram_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut src_stmt = self.conn.prepare(
            "SELECT source_key FROM source_subs WHERE telegram_id = ?1 ORDER BY source_key",
        )?;
        let sources: Vec<String> = src_stmt
            .query_map(params![telegram_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(UserSubs { keywords, sources })
    }

    /// 특정 소스를 구독 중인 활성 사용자 목록.
    pub fn get_source_subscribers(&self, source_key: &str) -> anyhow::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.telegram_id FROM source_subs s
             JOIN users u ON u.telegram_id = s.telegram_id
             WHERE s.source_key = ?1 AND u.is_active = 1",
        )?;
        let ids: Vec<i64> = stmt
            .query_map(params![source_key], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// 전체 키워드 구독 목록 (DM 매칭 엔진용).
    /// 반환: Vec<(telegram_id, keyword)>
    pub fn get_all_keyword_subs(&self) -> anyhow::Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT k.telegram_id, k.keyword FROM keyword_subs k
             JOIN users u ON u.telegram_id = k.telegram_id
             WHERE u.is_active = 1",
        )?;
        let subs: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(subs)
    }

    /// 이미 DM을 보냈는지 확인.
    pub fn is_dm_sent(&self, notice_db_id: i64, telegram_id: i64) -> anyhow::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM dm_log WHERE notice_id = ?1 AND telegram_id = ?2",
            params![notice_db_id, telegram_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// DM 발송 기록.
    pub fn log_dm(
        &self,
        notice_db_id: i64,
        telegram_id: i64,
        match_type: &str,
        match_value: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO dm_log (notice_id, telegram_id, match_type, match_value)
             VALUES (?1, ?2, ?3, ?4)",
            params![notice_db_id, telegram_id, match_type, match_value],
        )?;
        Ok(())
    }

    /// 사용자 비활성화 (봇 차단 등).
    #[allow(dead_code)]
    pub fn deactivate_user(&self, telegram_id: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE users SET is_active = 0 WHERE telegram_id = ?1",
            params![telegram_id],
        )?;
        Ok(())
    }

    /// 마감일이 있는 최근 공지 조회 (Phase 3 알림용).
    #[allow(dead_code)]
    pub fn get_deadline_notices(&self, limit: usize) -> anyhow::Result<Vec<Notice>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_key, notice_id, title, url, author, category, published
             FROM notices
             WHERE deadline IS NOT NULL AND deadline >= date('now')
             ORDER BY deadline ASC
             LIMIT ?1",
        )?;
        let notices = stmt
            .query_map(params![limit as i64], |row| {
                let source_key: String = row.get(1)?;
                Ok(Notice {
                    id: row.get(0)?,
                    source_key: source_key.clone(),
                    notice_id: row.get(2)?,
                    title: row.get(3)?,
                    url: row.get(4)?,
                    author: row.get(5)?,
                    category: row.get::<_, Option<String>>(6)?
                        .unwrap_or_else(|| "general".into()),
                    published: row.get(7)?,
                    source_display_name: source_key,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(notices)
    }

    /// 공지에 마감일 설정.
    pub fn set_deadline(&self, notice_db_id: i64, deadline: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE notices SET deadline = ?1 WHERE id = ?2",
            params![deadline, notice_db_id],
        )?;
        Ok(())
    }

    /// 크롤 상태 통계 조회.
    pub fn get_crawl_stats(&self) -> anyhow::Result<Vec<CrawlStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_key, last_crawled, error_count FROM crawl_state ORDER BY source_key",
        )?;
        let stats = stmt
            .query_map([], |row| {
                Ok(CrawlStat {
                    source_key: row.get(0)?,
                    last_crawled: row.get(1)?,
                    error_count: row.get(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(stats)
    }

    /// DM 대상 공지 조회 (notified=1이면서 아직 DM 처리 안 된 최근 공지).
    pub fn get_recent_for_dm(&self, limit: usize) -> anyhow::Result<Vec<Notice>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source_key, notice_id, title, url, author, category, published
             FROM notices
             WHERE notified = 1 AND crawled_at >= datetime('now', '-1 day')
             ORDER BY crawled_at DESC
             LIMIT ?1",
        )?;
        let notices = stmt
            .query_map(params![limit as i64], |row| {
                let source_key: String = row.get(1)?;
                Ok(Notice {
                    id: row.get(0)?,
                    source_key: source_key.clone(),
                    notice_id: row.get(2)?,
                    title: row.get(3)?,
                    url: row.get(4)?,
                    author: row.get(5)?,
                    category: row.get::<_, Option<String>>(6)?
                        .unwrap_or_else(|| "general".into()),
                    published: row.get(7)?,
                    source_display_name: source_key,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(notices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::RawNotice;

    fn make_notice(id: &str, title: &str) -> RawNotice {
        RawNotice {
            notice_id: id.to_string(),
            title: title.to_string(),
            url: format!("https://example.com/{}", id),
            author: Some("테스트".into()),
            date: Some("2026-02-01".into()),
            category: None,
            is_pinned: false,
        }
    }

    #[test]
    fn test_insert_and_dedup() {
        let db = Database::init(":memory:").unwrap();
        let n = make_notice("123", "테스트 공지");

        let first = db.insert_if_new("test", &n, "테스트 소스").unwrap();
        assert!(first, "First insert should be new");

        let second = db.insert_if_new("test", &n, "테스트 소스").unwrap();
        assert!(!second, "Duplicate insert should be ignored");
    }

    #[test]
    fn test_pending_and_mark_notified() {
        let db = Database::init(":memory:").unwrap();
        let display = std::collections::HashMap::from([
            ("test".to_string(), "테스트 소스".to_string()),
        ]);

        db.insert_if_new("test", &make_notice("1", "공지1"), "테스트 소스").unwrap();
        db.insert_if_new("test", &make_notice("2", "공지2"), "테스트 소스").unwrap();

        let pending = db.get_pending(10, &display).unwrap();
        assert_eq!(pending.len(), 2);

        db.mark_notified(pending[0].id).unwrap();

        let pending = db.get_pending(10, &display).unwrap();
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn test_error_count() {
        let db = Database::init(":memory:").unwrap();
        let c1 = db.increment_error("test").unwrap();
        assert_eq!(c1, 1);
        let c2 = db.increment_error("test").unwrap();
        assert_eq!(c2, 2);
        db.reset_error("test").unwrap();
        let c3 = db.increment_error("test").unwrap();
        assert_eq!(c3, 1);
    }

    #[test]
    fn test_user_registration_and_subs() {
        let db = Database::init(":memory:").unwrap();

        // 사용자 등록
        db.register_user(12345, Some("testuser"), Some("Test")).unwrap();

        // 키워드 구독
        assert!(db.add_keyword_sub(12345, "장학금").unwrap());
        assert!(db.add_keyword_sub(12345, "채용").unwrap());
        // 중복 무시
        assert!(!db.add_keyword_sub(12345, "장학금").unwrap());

        // 소스 구독
        assert!(db.add_source_sub(12345, "cbnu_main").unwrap());

        // 구독 조회
        let subs = db.get_user_subs(12345).unwrap();
        assert_eq!(subs.keywords, vec!["장학금", "채용"]);
        assert_eq!(subs.sources, vec!["cbnu_main"]);

        // 키워드 삭제
        assert!(db.remove_keyword_sub(12345, "채용").unwrap());
        let subs = db.get_user_subs(12345).unwrap();
        assert_eq!(subs.keywords, vec!["장학금"]);
    }

    #[test]
    fn test_source_subscribers() {
        let db = Database::init(":memory:").unwrap();
        db.register_user(100, None, None).unwrap();
        db.register_user(200, None, None).unwrap();
        db.add_source_sub(100, "biz").unwrap();
        db.add_source_sub(200, "biz").unwrap();

        let subs = db.get_source_subscribers("biz").unwrap();
        assert_eq!(subs.len(), 2);
        assert!(subs.contains(&100));
        assert!(subs.contains(&200));

        // 비활성 유저는 제외
        db.deactivate_user(200).unwrap();
        let subs = db.get_source_subscribers("biz").unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0], 100);
    }

    #[test]
    fn test_dm_log() {
        let db = Database::init(":memory:").unwrap();
        db.register_user(100, None, None).unwrap();
        db.insert_if_new("test", &make_notice("1", "장학금 공지"), "테스트").unwrap();

        // 아직 DM 안 보냄
        assert!(!db.is_dm_sent(1, 100).unwrap());

        // DM 기록
        db.log_dm(1, 100, "keyword", Some("장학금")).unwrap();
        assert!(db.is_dm_sent(1, 100).unwrap());

        // 중복 기록은 무시
        db.log_dm(1, 100, "keyword", Some("장학금")).unwrap();
    }
}
