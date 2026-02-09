pub mod ciboard;
pub mod egov;
pub mod php_master;
pub mod xe_board;

use async_trait::async_trait;
use reqwest::Client;

use crate::config::SourceConfig;

#[derive(Debug, Clone)]
pub struct RawNotice {
    pub notice_id: String,
    pub title: String,
    pub url: String,
    pub author: Option<String>,
    pub date: Option<String>,
    #[allow(dead_code)]
    pub category: Option<String>,
    #[allow(dead_code)]
    pub is_pinned: bool,
}

#[async_trait]
pub trait NoticeParser: Send + Sync {
    async fn fetch_notices(&self, client: &Client) -> anyhow::Result<Vec<RawNotice>>;
    fn source_key(&self) -> &str;
    fn display_name(&self) -> &str;
}

pub fn create_parser(source: &SourceConfig) -> Box<dyn NoticeParser> {
    match source.parser.as_str() {
        "egov" => Box::new(egov::EgovParser::from_config(source)),
        "php_master" => Box::new(php_master::PhpMasterParser::from_config(source)),
        "ciboard" => Box::new(ciboard::CiBoardParser::from_config(source)),
        "xe_board" => Box::new(xe_board::XeBoardParser::from_config(source)),
        other => panic!("Unknown parser type: {other}"),
    }
}
