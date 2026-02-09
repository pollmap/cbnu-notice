use thiserror::Error;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum AppError {
    #[error("HTTP: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Parse [{source_key}]: {detail}")]
    Parse { source_key: String, detail: String },

    #[error("DB: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("Telegram: {0}")]
    Telegram(String),

    #[error("Config: {0}")]
    Config(String),

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
}

impl From<teloxide::RequestError> for AppError {
    fn from(e: teloxide::RequestError) -> Self {
        AppError::Telegram(e.to_string())
    }
}
