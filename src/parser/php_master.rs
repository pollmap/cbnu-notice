use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

use super::{NoticeParser, RawNotice};
use crate::config::SourceConfig;

/// Parser for PHP master.php CMS used by many CBNU departments.
///
/// The board loads content via AJAX POST to `./module/board/_main.php`.
/// The main page contains hidden form fields (`bidx`, `id`) that must be
/// extracted first, then sent with the AJAX request.
///
/// The response HTML uses Bootstrap grid divs (not `<table>`), with each row
/// having class `board_rows`.
pub struct PhpMasterParser {
    source_key: String,
    display_name: String,
    base_url: String,
    pg_idx: String,
}

/// Form parameters extracted from the main page's hidden inputs.
struct FormParams {
    bidx: String,
    id: String,
}

impl PhpMasterParser {
    pub fn from_config(config: &SourceConfig) -> Self {
        Self {
            source_key: config.key.clone(),
            display_name: config.display_name.clone(),
            base_url: config.url.trim_end_matches('/').to_string(),
            pg_idx: config.params.get("pg_idx").cloned().unwrap_or_default(),
        }
    }

    fn ajax_url(&self) -> String {
        format!("{}/module/board/_main.php", self.base_url)
    }

    fn main_page_url(&self) -> String {
        format!("{}/master.php?pg_idx={}", self.base_url, self.pg_idx)
    }

    fn build_view_url(&self, pidx: &str) -> String {
        format!(
            "{}/master.php?mod=view&pg_idx={}&pidx={}",
            self.base_url, self.pg_idx, pidx
        )
    }

    /// Fetch the main page and extract hidden form fields (bidx, id).
    async fn extract_form_params(&self, client: &Client) -> anyhow::Result<FormParams> {
        let url = self.main_page_url();
        let resp = client.get(&url).send().await?;
        let html = resp.text().await?;
        let document = Html::parse_document(&html);

        let bidx_sel = Selector::parse("input#bidx").unwrap();
        let id_sel = Selector::parse("input#id").unwrap();

        let bidx = document
            .select(&bidx_sel)
            .next()
            .and_then(|el| el.value().attr("value"))
            .unwrap_or("2")
            .to_string();

        let id = document
            .select(&id_sel)
            .next()
            .and_then(|el| el.value().attr("value"))
            .unwrap_or("")
            .to_string();

        tracing::debug!(bidx = %bidx, id = %id, "Extracted form params");

        Ok(FormParams { bidx, id })
    }

    fn parse_ajax_html(&self, html: &str) -> anyhow::Result<Vec<RawNotice>> {
        let document = Html::parse_fragment(html);
        let pidx_re = Regex::new(r"pidx=(\d+)")?;

        let row_sel = Selector::parse("div.board_rows").unwrap();
        let div_sel = Selector::parse("div").unwrap();
        let a_sel = Selector::parse("a[href]").unwrap();

        let mut notices = Vec::new();

        for row in document.select(&row_sel) {
            let divs: Vec<_> = row.select(&div_sel).collect();
            if divs.len() < 4 {
                continue;
            }

            // Find link with pidx
            let link = match row.select(&a_sel).next() {
                Some(a) => a,
                None => continue,
            };

            let href = link.value().attr("href").unwrap_or("");
            let notice_id = match pidx_re.captures(href) {
                Some(caps) => caps[1].to_string(),
                None => continue,
            };

            let title = link.text().collect::<String>().trim().to_string();
            if title.is_empty() {
                continue;
            }

            let url = self.build_view_url(&notice_id);

            // First div: 순서 (번호 or "공지")
            let first_text = divs[0].text().collect::<String>().trim().to_string();
            let is_pinned = first_text.contains("공지");

            // Extract author and date from remaining divs
            // Layout: [순서, 제목, 작성자, 날짜, 조회수]
            let author = if divs.len() >= 4 {
                let t = divs[2].text().collect::<String>().trim().to_string();
                if t.is_empty() { None } else { Some(t) }
            } else {
                None
            };

            let date = if divs.len() >= 5 {
                let t = divs[3].text().collect::<String>().trim().to_string();
                if t.is_empty() { None } else { Some(t) }
            } else {
                None
            };

            notices.push(RawNotice {
                notice_id,
                title,
                url,
                author,
                date,
                category: None, // PHP CMS doesn't have categories
                is_pinned,
            });
        }

        Ok(notices)
    }
}

#[async_trait]
impl NoticeParser for PhpMasterParser {
    async fn fetch_notices(&self, client: &Client) -> anyhow::Result<Vec<RawNotice>> {
        tracing::info!(
            source = %self.source_key,
            pg_idx = %self.pg_idx,
            "Fetching PHP master notices"
        );

        // Step 1: Fetch main page to get form params (bidx, id)
        let params = self.extract_form_params(client).await?;

        // Step 2: AJAX POST for board content
        let ajax_url = self.ajax_url();
        let form_params = [
            ("pg_idx", self.pg_idx.as_str()),
            ("bidx", params.bidx.as_str()),
            ("id", params.id.as_str()),
            ("cate", ""),
            ("pidx", "0"),
            ("str", ""),
            ("page", "1"),
            ("mode", "list"),
        ];

        let resp = client
            .post(&ajax_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", self.main_page_url())
            .form(&form_params)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("HTTP {} from {}", status, ajax_url);
        }

        let html = resp.text().await?;
        if html.trim().is_empty() {
            anyhow::bail!("Empty response from {}", ajax_url);
        }

        let notices = self.parse_ajax_html(&html)?;

        tracing::info!(
            source = %self.source_key,
            count = notices.len(),
            "Parsed PHP master notices"
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
        params.insert("pg_idx".into(), "7".into());
        SourceConfig {
            key: "biz".into(),
            display_name: "경영학부".into(),
            parser: "php_master".into(),
            url: "https://biz.chungbuk.ac.kr".into(),
            params,
            enabled: true,
            channel: None,
        }
    }

    #[test]
    fn test_parse_php_master_fixture() {
        let html = std::fs::read_to_string("tests/fixtures/php_master_ajax_sample.html")
            .expect("Missing fixture: run `cargo run` first to download sample HTML");
        let parser = PhpMasterParser::from_config(&test_config());
        let notices = parser.parse_ajax_html(&html).unwrap();

        assert!(!notices.is_empty(), "Should parse at least one notice");
        println!("Parsed {} notices from PHP master fixture", notices.len());

        // Check first notice
        let first = &notices[0];
        assert!(!first.notice_id.is_empty());
        assert!(!first.title.is_empty());
        assert!(first.url.contains("pidx="));
        println!(
            "First: id={} title={} pinned={} author={:?} date={:?}",
            first.notice_id, first.title, first.is_pinned, first.author, first.date
        );

        // Check IDs are unique
        let ids: Vec<_> = notices.iter().map(|n| &n.notice_id).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "All notice_ids should be unique");
    }
}
