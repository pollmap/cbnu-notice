use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

use super::{NoticeParser, RawNotice};
use crate::config::SourceConfig;

/// Parser for XpressEngine (XE) board modules.
///
/// Used by engineering departments (civil, material, safety, cheme, me, ee,
/// env, software). The board listing is at `/{mid}` where mid is the module
/// ID (e.g., `board_jIDW98`, `material5_1_1`, `safety4_1`).
///
/// HTML structure:
/// ```html
/// <table class="bd_lst bd_tb_lst bd_tb">
///   <tbody>
///     <tr>
///       <td class="no"><strong>공지</strong></td>  <!-- or number -->
///       <td class="title">
///         <a href="https://site/{mid}/{document_srl}" class="hx">Title</a>
///       </td>
///       <td class="author"><span><a>Author</a></span></td>
///       <td class="time">2026.02.06</td>
///       <td class="m_no">22</td>
///     </tr>
///   </tbody>
/// </table>
/// ```
pub struct XeBoardParser {
    source_key: String,
    display_name: String,
    base_url: String,
    mid: String,
}

impl XeBoardParser {
    pub fn from_config(config: &SourceConfig) -> Self {
        Self {
            source_key: config.key.clone(),
            display_name: config.display_name.clone(),
            base_url: config.url.trim_end_matches('/').to_string(),
            mid: config.params.get("mid").cloned().unwrap_or_default(),
        }
    }

    fn board_url(&self) -> String {
        format!("{}/{}", self.base_url, self.mid)
    }

    fn build_view_url(&self, document_srl: &str) -> String {
        format!("{}/{}/{}", self.base_url, self.mid, document_srl)
    }

    fn parse_html(&self, html: &str) -> anyhow::Result<Vec<RawNotice>> {
        let document = Html::parse_document(html);
        let srl_re = Regex::new(r"/(\d+)(?:\?|#|$)")?;

        let table_selectors = [
            "table.bd_lst tbody tr",
            "table.bd_tb_lst tbody tr",
            "table.bd_tb tbody tr",
        ];

        let td_sel = Selector::parse("td").unwrap();
        let a_sel = Selector::parse("a[href]").unwrap();
        let no_sel = Selector::parse("td.no").unwrap();
        let title_sel = Selector::parse("td.title").unwrap();
        let author_sel = Selector::parse("td.author").unwrap();
        let time_sel = Selector::parse("td.time").unwrap();

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
                if cells.len() < 3 {
                    continue;
                }

                // Find link in title cell
                let title_cell = match row.select(&title_sel).next() {
                    Some(td) => td,
                    None => continue,
                };

                let link = match title_cell.select(&a_sel).next() {
                    Some(a) => a,
                    None => continue,
                };

                let href = link.value().attr("href").unwrap_or("");
                let notice_id = if let Some(caps) = srl_re.captures(href) {
                    caps[1].to_string()
                } else {
                    // Try document_srl parameter
                    let dsrl_re = Regex::new(r"document_srl=(\d+)").unwrap();
                    match dsrl_re.captures(href) {
                        Some(caps) => caps[1].to_string(),
                        None => continue,
                    }
                };

                let title = link.text().collect::<String>().trim().to_string();
                if title.is_empty() {
                    continue;
                }

                let url = self.build_view_url(&notice_id);

                // Pinned: "no" cell contains "공지" (in <strong> tag)
                let is_pinned = row
                    .select(&no_sel)
                    .next()
                    .map(|td| td.text().collect::<String>().contains("공지"))
                    .unwrap_or(false);

                // Author
                let author = row
                    .select(&author_sel)
                    .next()
                    .map(|td| td.text().collect::<String>().trim().to_string())
                    .filter(|t| !t.is_empty());

                // Date
                let date = row
                    .select(&time_sel)
                    .next()
                    .map(|td| td.text().collect::<String>().trim().to_string())
                    .filter(|t| !t.is_empty());

                notices.push(RawNotice {
                    notice_id,
                    title,
                    url,
                    author,
                    date,
                    category: None,
                    is_pinned,
                });
            }

            if !notices.is_empty() {
                break;
            }
        }

        Ok(notices)
    }
}

#[async_trait]
impl NoticeParser for XeBoardParser {
    async fn fetch_notices(&self, client: &Client) -> anyhow::Result<Vec<RawNotice>> {
        let url = self.board_url();
        tracing::info!(source = %self.source_key, url = %url, "Fetching XE board notices");

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
            "Parsed XE board notices"
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
        params.insert("mid".into(), "board_jIDW98".into());
        SourceConfig {
            key: "civil".into(),
            display_name: "토목공학부".into(),
            parser: "xe_board".into(),
            url: "https://civil.chungbuk.ac.kr".into(),
            params,
            enabled: true,
            channel: None,
        }
    }

    #[test]
    fn test_parse_xe_board_fixture() {
        let html = std::fs::read_to_string("tests/fixtures/xe_board_sample.html")
            .expect("Missing fixture: tests/fixtures/xe_board_sample.html");
        let parser = XeBoardParser::from_config(&test_config());
        let notices = parser.parse_html(&html).unwrap();

        assert!(!notices.is_empty(), "Should parse at least one notice");
        println!("Parsed {} notices from XE board fixture", notices.len());

        let first = &notices[0];
        assert!(!first.notice_id.is_empty());
        assert!(!first.title.is_empty());
        assert!(first.url.contains("board_jIDW98"));
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
