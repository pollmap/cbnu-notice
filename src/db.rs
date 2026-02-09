use chrono::Utc;
use rusqlite::{params, Connection};

use crate::category::Category;
use crate::parser::RawNotice;

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
        let now = Utc::now().to_rfc3339();

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

    /// Update crawl state after successful crawl.
    pub fn update_crawl_state(&self, source_key: &str, last_id: Option<&str>) -> anyhow::Result<()> {
        let now = Utc::now().to_rfc3339();
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
        let now = Utc::now().to_rfc3339();
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
}
