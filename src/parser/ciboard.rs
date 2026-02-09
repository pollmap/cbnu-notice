use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

use super::{NoticeParser, RawNotice};
use crate::config::SourceConfig;

/// Parser for CIBoard (CodeIgniter Board) CMS.
///
/// Used by social science departments (sociology, public admin, politics,
/// psychology, economics). The board listing is at `/board/{board_name}`.
/// Each notice links to `/post/{id}`.
///
/// HTML structure:
/// ```html
/// <table class="gitav_table_skin1">
///   <tbody>
///     <tr>
///       <td><span class="label ...">공지</span></td>  <!-- or number -->
///       <td class="text-left text_over">
///         <a href="https://site/post/123" title="...">Title</a>
///       </td>
///       <td>-</td>            <!-- file -->
///       <td>01-27</td>        <!-- date -->
///       <td>391</td>          <!-- views -->
///     </tr>
///   </tbody>
/// </table>
/// ```
pub struct CiBoardParser {
    source_key: String,
    display_name: String,
    base_url: String,
    board_name: String,
}

impl CiBoardParser {
    pub fn from_config(config: &SourceConfig) -> Self {
        Self {
            source_key: config.key.clone(),
            display_name: config.display_name.clone(),
            base_url: config.url.trim_end_matches('/').to_string(),
            board_name: config
                .params
                .get("board_name")
                .cloned()
                .unwrap_or_else(|| "department_notice".to_string()),
        }
    }

    fn board_url(&self) -> String {
        format!("{}/board/{}", self.base_url, self.board_name)
    }

    fn parse_html(&self, html: &str) -> anyhow::Result<Vec<RawNotice>> {
        let document = Html::parse_document(html);
        let post_re = Regex::new(r"/post/(\d+)")?;

        // Table selectors - CIBoard uses gitav_table_skin1 or standard Bootstrap
        let table_selectors = [
            "table.gitav_table_skin1 tbody tr",
            "table.board tbody tr",
            "table tbody tr",
        ];

        let td_sel = Selector::parse("td").unwrap();
        let a_sel = Selector::parse("a[href]").unwrap();
        let pinned_sel = Selector::parse("span.label").unwrap();

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

                // Find link with /post/{id}
                let link = match row.select(&a_sel).next() {
                    Some(a) => a,
                    None => continue,
                };

                let href = link.value().attr("href").unwrap_or("");
                let notice_id = match post_re.captures(href) {
                    Some(caps) => caps[1].to_string(),
                    None => continue,
                };

                let title = link
                    .value()
                    .attr("title")
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| link.text().collect::<String>().trim().to_string());

                if title.is_empty() {
                    continue;
                }

                let url = format!("{}/post/{}", self.base_url, notice_id);

                // Pinned: first cell contains <span class="label">
                let is_pinned = cells[0].select(&pinned_sel).next().is_some();

                // Date is in the 4th cell (index 3)
                let date = if cells.len() >= 4 {
                    let t = cells[3].text().collect::<String>().trim().to_string();
                    if t.is_empty() { None } else { Some(t) }
                } else {
                    None
                };

                notices.push(RawNotice {
                    notice_id,
                    title,
                    url,
                    author: None,
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
impl NoticeParser for CiBoardParser {
    async fn fetch_notices(&self, client: &Client) -> anyhow::Result<Vec<RawNotice>> {
        let url = self.board_url();
        tracing::info!(source = %self.source_key, url = %url, "Fetching CIBoard notices");

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
            "Parsed CIBoard notices"
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
        params.insert("board_name".into(), "department_notice".into());
        SourceConfig {
            key: "sociology".into(),
            display_name: "사회학과".into(),
            parser: "ciboard".into(),
            url: "https://sociology.chungbuk.ac.kr".into(),
            params,
            enabled: true,
            channel: None,
        }
    }

    #[test]
    fn test_parse_ciboard_fixture() {
        let html = std::fs::read_to_string("tests/fixtures/ciboard_sample.html")
            .expect("Missing fixture: tests/fixtures/ciboard_sample.html");
        let parser = CiBoardParser::from_config(&test_config());
        let notices = parser.parse_html(&html).unwrap();

        assert!(!notices.is_empty(), "Should parse at least one notice");
        println!("Parsed {} notices from CIBoard fixture", notices.len());

        let first = &notices[0];
        assert!(!first.notice_id.is_empty());
        assert!(!first.title.is_empty());
        assert!(first.url.contains("/post/"));
        println!(
            "First: id={} title={} pinned={} date={:?}",
            first.notice_id, first.title, first.is_pinned, first.date
        );

        // Check IDs are unique
        let ids: Vec<_> = notices.iter().map(|n| &n.notice_id).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "All notice_ids should be unique");
    }
}
