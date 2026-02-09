use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    pub bot: BotConfig,
    pub database: DbConfig,
    #[serde(rename = "source")]
    pub sources: Vec<SourceConfig>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BotConfig {
    pub telegram_channel: String,
    pub log_channel: Option<String>,
    #[serde(default = "default_max_notices")]
    pub max_notices_per_run: usize,
    #[serde(default = "default_delay")]
    pub message_delay_ms: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DbConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SourceConfig {
    pub key: String,
    pub display_name: String,
    pub parser: String,
    pub url: String,
    #[serde(default)]
    pub params: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 이 소스의 공지를 보낼 채널. 미지정 시 bot.telegram_channel 사용.
    pub channel: Option<String>,
}

fn default_max_notices() -> usize {
    20
}
fn default_delay() -> u64 {
    150
}
fn default_db_path() -> String {
    "notices.db".to_string()
}
fn default_true() -> bool {
    true
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read config file {:?}: {}", path, e))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;
        Ok(config)
    }

    pub fn enabled_sources(&self) -> Vec<&SourceConfig> {
        self.sources.iter().filter(|s| s.enabled).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml_str = r#"
[bot]
telegram_channel = "@cbnu_notice"
max_notices_per_run = 10
message_delay_ms = 200

[database]
path = "test.db"

[[source]]
key = "cbnu_main"
display_name = "충북대 공지"
parser = "egov"
url = "https://www.chungbuk.ac.kr/www/selectBbsNttList.do"
enabled = true
[source.params]
bbsNo = "8"
key = "813"

[[source]]
key = "biz"
display_name = "경영학부"
parser = "php_master"
url = "https://biz.chungbuk.ac.kr"
enabled = false
channel = "@cbnu_dept"
[source.params]
pg_idx = "7"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.bot.telegram_channel, "@cbnu_notice");
        assert_eq!(config.bot.max_notices_per_run, 10);
        assert_eq!(config.sources.len(), 2);
        assert_eq!(config.enabled_sources().len(), 1);
        assert_eq!(config.sources[0].params.get("bbsNo").unwrap(), "8");
    }
}
