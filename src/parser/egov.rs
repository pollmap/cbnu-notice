use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

use super::{NoticeParser, RawNotice};
use crate::config::SourceConfig;

pub struct EgovParser {
    source_key: String,
    display_name: String,
    base_url: String,
    bbs_no: String,
    key: String,
    page_unit: String,
}

impl EgovParser {
    pub fn from_config(config: &SourceConfig) -> Self {
        Self {
            source_key: config.key.clone(),
            display_name: config.display_name.clone(),
            base_url: config.url.clone(),
            bbs_no: config.params.get("bbsNo").cloned().unwrap_or_default(),
            key: config.params.get("key").cloned().unwrap_or_default(),
            page_unit: config
                .params
                .get("pageUnit")
                .cloned()
                .unwrap_or_else(|| "10".to_string()),
        }
    }

    fn build_list_url(&self) -> String {
        format!(
            "{}?bbsNo={}&key={}&pageUnit={}&pageIndex=1",
            self.base_url, self.bbs_no, self.key, self.page_unit
        )
    }

    fn build_view_url(&self, ntt_no: &str) -> String {
        let base = self.base_url.replace("selectBbsNttList.do", "selectBbsNttView.do");
        format!("{}?bbsNo={}&key={}&nttNo={}", base, self.bbs_no, self.key, ntt_no)
    }

    fn parse_html(&self, html: &str) -> anyhow::Result<Vec<RawNotice>> {
        let document = Html::parse_document(html);
        let ntt_re = Regex::new(r"nttNo=(\d+)")?;

        // Try multiple selectors for resilience
        let table_selectors = [
            "table.board-list tbody tr",
            "table.bbs-list tbody tr",
            ".boardList tbody tr",
            "table tbody tr",
        ];

        let td_sel = Selector::parse("td").unwrap();
        let a_sel = Selector::parse("a[href]").unwrap();

        let mut notices = Vec::new();

        for sel_str in &table_selectors {
            let row_sel = match Selector::parse(sel_str) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let rows: Vec<_> = document.select(&row_sel).collect();
            if rows.is_empty() {
                continue;
            }

            for row in rows {
                let cells: Vec<_> = row.select(&td_sel).collect();
                if cells.len() < 4 {
                    continue;
                }

                // Find the link with nttNo
                let link = match row.select(&a_sel).next() {
                    Some(a) => a,
                    None => continue,
                };

                let href = link.value().attr("href").unwrap_or("");
                let notice_id = match ntt_re.captures(href) {
                    Some(caps) => caps[1].to_string(),
                    None => continue,
                };

                let title = link.text().collect::<String>().trim().to_string();
                if title.is_empty() {
                    continue;
                }

                let url = self.build_view_url(&notice_id);

                // Determine if pinned: first cell contains "공지"
                let first_cell_text = cells[0].text().collect::<String>();
                let is_pinned = first_cell_text.contains("공지");

                // Extract category, author, date based on column count
                let (category, author, date) = if cells.len() >= 6 {
                    // [번호, 카테고리, 제목, 작성자, 날짜, 조회수]
                    let cat = cells[1].text().collect::<String>().trim().to_string();
                    let aut = cells[3].text().collect::<String>().trim().to_string();
                    let dat = cells[4].text().collect::<String>().trim().to_string();
                    (
                        if cat.is_empty() { None } else { Some(cat) },
                        if aut.is_empty() { None } else { Some(aut) },
                        if dat.is_empty() { None } else { Some(dat) },
                    )
                } else if cells.len() >= 5 {
                    // [번호, 제목, 작성자, 날짜, 조회수]
                    let aut = cells[2].text().collect::<String>().trim().to_string();
                    let dat = cells[3].text().collect::<String>().trim().to_string();
                    (
                        None,
                        if aut.is_empty() { None } else { Some(aut) },
                        if dat.is_empty() { None } else { Some(dat) },
                    )
                } else {
                    (None, None, None)
                };

                notices.push(RawNotice {
                    notice_id,
                    title,
                    url,
                    author,
                    date,
                    category,
                    is_pinned,
                });
            }

            // If we found notices with this selector, don't try others
            if !notices.is_empty() {
                break;
            }
        }

        Ok(notices)
    }
}

#[async_trait]
impl NoticeParser for EgovParser {
    async fn fetch_notices(&self, client: &Client) -> anyhow::Result<Vec<RawNotice>> {
        let url = self.build_list_url();
        tracing::info!(source = %self.source_key, url = %url, "Fetching eGov notices");

        let resp = client.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {} from {}", status, url);
        }

        let html = resp.text().await?;
        let notices = self.parse_html(&html)?;

        tracing::info!(
            source = %self.source_key,
            count = notices.len(),
            "Parsed eGov notices"
        );

        Ok(notices)
    }

    fn source_key(&self) -> &str {
        &self.source_key
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SourceConfig;
    use std::collections::HashMap;

    fn test_config() -> SourceConfig {
        let mut params = HashMap::new();
        params.insert("bbsNo".into(), "8".into());
        params.insert("key".into(), "813".into());
        params.insert("pageUnit".into(), "10".into());
        SourceConfig {
            key: "cbnu_main".into(),
            display_name: "충북대 공지".into(),
            parser: "egov".into(),
            url: "https://www.chungbuk.ac.kr/www/selectBbsNttList.do".into(),
            params,
            enabled: true,
            channel: None,
        }
    }

    #[test]
    fn test_parse_egov_fixture() {
        let html = std::fs::read_to_string("tests/fixtures/egov_sample.html")
            .expect("Missing fixture file");
        let parser = EgovParser::from_config(&test_config());
        let notices = parser.parse_html(&html).unwrap();

        assert!(!notices.is_empty(), "Should parse at least one notice");
        assert_eq!(notices.len(), 10, "Fixture has 10 entries");

        // Check first notice (pinned)
        let first = &notices[0];
        assert_eq!(first.notice_id, "182452");
        assert!(first.title.contains("수강신청"));
        assert!(first.is_pinned);
        assert_eq!(first.author.as_deref(), Some("학사과"));
        assert_eq!(first.date.as_deref(), Some("2026-02-01"));
        assert_eq!(first.category.as_deref(), Some("학사"));

        // Check a non-pinned notice
        let third = &notices[2];
        assert_eq!(third.notice_id, "182451");
        assert!(!third.is_pinned);

        // Check all notice_ids are unique
        let ids: Vec<_> = notices.iter().map(|n| &n.notice_id).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "All notice_ids should be unique");
    }
}
