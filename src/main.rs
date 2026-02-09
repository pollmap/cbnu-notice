mod bot_commands;
mod category;
mod config;
mod deadline;
mod db;
mod dm_engine;
mod error;
mod notifier;
mod parser;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use tokio::time::sleep;

use crate::parser::{NoticeParser, RawNotice};

#[derive(Parser)]
#[command(name = "cbnu-notice-bot", about = "충북대 공지사항 자동 알림 봇")]
enum Cli {
    /// 크롤링 1회 실행 (GitHub Actions cron에서 호출)
    Crawl,
    /// 봇 서버 시작 + 자동 크롤링 (상시 실행, 이것만 돌리면 됨)
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli {
        Cli::Crawl => run_crawl().await,
        Cli::Serve => run_serve().await,
    }
}

/// DB 경로 결정 (환경변수 DATABASE_PATH > config).
fn resolve_db_path(cfg: &config::Config) -> String {
    std::env::var("DATABASE_PATH").unwrap_or_else(|_| cfg.database.path.clone())
}

/// 크롤링 1회 실행 (CLI 또는 cron용).
async fn run_crawl() -> anyhow::Result<()> {
    let config_path = Path::new("config.toml");
    let cfg = if config_path.exists() {
        config::Config::load(config_path)?
    } else {
        anyhow::bail!("config.toml is required. Please create it first.");
    };

    let client = build_http_client()?;
    let db_path = resolve_db_path(&cfg);

    let (channel_id, log_channel_id) = resolve_channels(&cfg);

    let dry_run = std::env::var("TELOXIDE_TOKEN").is_err();
    if dry_run {
        tracing::warn!("TELOXIDE_TOKEN not set. Running in dry-run mode (no Telegram messages).");
    }

    let notifier_opt = if !dry_run {
        let bot = Bot::from_env();
        Some(notifier::Notifier::new(
            bot,
            channel_id,
            log_channel_id,
            cfg.bot.message_delay_ms,
        ))
    } else {
        None
    };

    do_crawl(&cfg, &client, &db_path, notifier_opt.as_ref()).await
}

/// 봇 서버 모드: 텔레그램 커맨드 수신 + 자동 크롤링.
/// 이 모드 하나만 실행하면 모든 기능이 동작한다.
async fn run_serve() -> anyhow::Result<()> {
    let config_path = Path::new("config.toml");
    let cfg = config::Config::load(config_path)?;
    let db_path = resolve_db_path(&cfg);
    let database = db::Database::init(&db_path)?;

    let bot = Bot::from_env();
    tracing::info!("Starting serve mode (bot commands + auto crawl)...");

    let state = Arc::new(bot_commands::BotState {
        db: Arc::new(Mutex::new(database)),
        sources: cfg.sources.clone(),
    });

    // 봇 커맨드 등록
    if let Err(e) = bot
        .set_my_commands(bot_commands::Command::bot_commands())
        .await
    {
        tracing::warn!(error = %e, "Failed to set bot commands menu");
    }

    // 자동 크롤링 백그라운드 스레드 (별도 tokio 런타임).
    // rusqlite::Connection이 Sync가 아니므로 tokio::spawn 대신 별도 스레드 사용.
    let crawl_cfg = cfg.clone();
    let crawl_bot = bot.clone();
    let db_path_clone = db_path.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build crawl runtime");
        rt.block_on(crawl_loop(crawl_cfg, crawl_bot, db_path_clone));
    });

    // 텔레그램 long polling (메인 태스크)
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<bot_commands::Command>()
                .endpoint(
                    |bot: Bot, msg: Message, cmd: bot_commands::Command, state: Arc<bot_commands::BotState>| async move {
                        bot_commands::handle_command(bot, msg, cmd, state).await
                    },
                ),
        );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .default_handler(|_| async {})
        .error_handler(Arc::new(|err| {
            Box::pin(async move {
                tracing::error!(error = %err, "Dispatch error");
            })
        }))
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

/// 백그라운드 자동 크롤링 루프.
/// 시작 즉시 1회 실행 후, 설정된 간격으로 반복.
async fn crawl_loop(cfg: config::Config, bot: Bot, db_path: String) {
    let interval = Duration::from_secs(cfg.bot.crawl_interval_secs);
    tracing::info!(
        interval_secs = cfg.bot.crawl_interval_secs,
        "Auto-crawl loop started"
    );

    let client = match build_http_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build HTTP client for crawl loop");
            return;
        }
    };

    let (channel_id, log_channel_id) = resolve_channels(&cfg);
    let notifier = notifier::Notifier::new(
        bot,
        channel_id,
        log_channel_id,
        cfg.bot.message_delay_ms,
    );

    loop {
        if let Err(e) = do_crawl(&cfg, &client, &db_path, Some(&notifier)).await {
            tracing::error!(error = %e, "Crawl cycle failed");
        }

        tracing::info!(next_in_secs = interval.as_secs(), "Sleeping until next crawl");
        sleep(interval).await;
    }
}

/// 크롤링 핵심 로직 (crawl + notify + DM).
/// `run_crawl()`과 `crawl_loop()` 모두 이 함수를 호출한다.
/// 매 호출마다 자체 DB 연결을 열어 Send 안전성을 보장한다.
async fn do_crawl(
    cfg: &config::Config,
    client: &reqwest::Client,
    db_path: &str,
    notifier_opt: Option<&notifier::Notifier>,
) -> anyhow::Result<()> {
    let database = db::Database::init(db_path)?;
    // Build source display name map + channel routing map
    let display_names: HashMap<String, String> = cfg
        .sources
        .iter()
        .map(|s| (s.key.clone(), s.display_name.clone()))
        .collect();

    let channel_map: HashMap<String, String> = cfg
        .sources
        .iter()
        .filter_map(|s| s.channel.as_ref().map(|ch| (s.key.clone(), ch.clone())))
        .collect();

    // Crawl each enabled source
    let enabled_sources = cfg.enabled_sources();
    tracing::info!(count = enabled_sources.len(), "Starting crawl");

    let mut total_new = 0u32;
    let mut source_stats: Vec<String> = Vec::new();

    for source_cfg in &enabled_sources {
        let parser = parser::create_parser(source_cfg);
        let source_key = parser.source_key().to_string();
        let display_name = parser.display_name().to_string();

        match fetch_with_retry(parser.as_ref(), client).await {
            Ok(notices) => {
                let mut new_count = 0u32;
                let last_id = notices.first().map(|n| n.notice_id.clone());

                for notice in &notices {
                    match database.insert_if_new(&source_key, notice, &display_name) {
                        Ok(true) => new_count += 1,
                        Ok(false) => {} // duplicate
                        Err(e) => {
                            tracing::error!(
                                source = %source_key,
                                notice_id = %notice.notice_id,
                                error = %e,
                                "DB insert failed"
                            );
                        }
                    }
                }

                database.update_crawl_state(&source_key, last_id.as_deref())?;
                tracing::info!(
                    source = %source_key,
                    total = notices.len(),
                    new = new_count,
                    "Crawl complete"
                );

                total_new += new_count;
                source_stats.push(format!("{}:{}", source_key, new_count));
            }
            Err(e) => {
                let err_count = database.increment_error(&source_key)?;
                tracing::error!(
                    source = %source_key,
                    error = %e,
                    consecutive_errors = err_count,
                    "Crawl failed"
                );

                if err_count >= 5 {
                    let alert = format!(
                        "\u{26a0}\u{fe0f} 크롤링 경고\n\n소스: {}\n상태: 연속 {}회 실패\n에러: {}",
                        source_key, err_count, e
                    );
                    if let Some(notifier) = notifier_opt {
                        let _ = notifier.send_error_alert(&alert).await;
                    }
                }

                source_stats.push(format!("{}:ERR", source_key));
            }
        }
    }

    // Send pending notifications
    let pending = database.get_pending(cfg.bot.max_notices_per_run, &display_names)?;
    let sent = if let Some(notifier) = notifier_opt {
        let sent_ids = notifier.send_batch(&pending, cfg.bot.max_notices_per_run, &channel_map).await?;

        for id in &sent_ids {
            database.mark_notified(*id)?;
        }

        sent_ids.len()
    } else {
        // Dry-run: print and mark as notified to avoid re-showing
        for notice in &pending {
            println!(
                "[DRY-RUN] Would send: {} {} - {}",
                category::Category::from_str_tag(&notice.category).emoji(),
                notice.source_display_name,
                notice.title
            );
            database.mark_notified(notice.id)?;
        }
        pending.len()
    };

    // 마감일 추출 + 저장
    {
        use crate::deadline::extract_deadline;
        let recent = database.get_recent_for_dm(100).unwrap_or_default();
        for notice in &recent {
            if let Some(dl) = extract_deadline(&notice.title) {
                let _ = database.set_deadline(notice.id, &dl.format("%Y-%m-%d").to_string());
            }
        }
    }

    // DM 발송 (구독자에게 개인 메시지)
    let dm_sent = if let Some(notifier) = notifier_opt {
        let engine = dm_engine::DmEngine::new(notifier.bot(), &database, cfg.bot.message_delay_ms);
        match engine.process().await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!(error = %e, "DM engine failed");
                0
            }
        }
    } else {
        0
    };

    // Summary
    let summary = format!(
        "\u{2705} Crawl done: {} new / {} ch-sent / {} dm | {}",
        total_new,
        sent,
        dm_sent,
        source_stats.join(" ")
    );
    tracing::info!("{}", summary);

    if let Some(notifier) = notifier_opt {
        if total_new > 0 || sent > 0 || dm_sent > 0 {
            let _ = notifier.send_summary(&summary).await;
        }
    }

    Ok(())
}

/// HTTP 클라이언트 생성 (SSL 인증서 문제 우회).
fn build_http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .user_agent("CBNU-Notice-Bot/1.0 (student project)")
        .timeout(Duration::from_secs(15))
        .build()?)
}

/// 채널 ID 결정 (환경변수 > config).
fn resolve_channels(cfg: &config::Config) -> (String, Option<String>) {
    let channel_id = std::env::var("CHANNEL_ID")
        .or_else(|_| std::env::var("TELEGRAM_CHANNEL_ID"))
        .unwrap_or_else(|_| cfg.bot.telegram_channel.clone());

    let log_channel_id = std::env::var("LOG_CHANNEL_ID")
        .or_else(|_| std::env::var("TELEGRAM_LOG_CHANNEL"))
        .ok()
        .or_else(|| cfg.bot.log_channel.clone())
        .filter(|s| !s.is_empty());

    (channel_id, log_channel_id)
}

/// 최대 3회 재시도 (2초 → 4초 → 8초 backoff)
async fn fetch_with_retry(
    parser: &dyn NoticeParser,
    client: &reqwest::Client,
) -> anyhow::Result<Vec<RawNotice>> {
    let max_retries = 3;
    let mut last_err = None;

    for attempt in 0..=max_retries {
        match parser.fetch_notices(client).await {
            Ok(notices) => return Ok(notices),
            Err(e) => {
                if attempt < max_retries {
                    let delay = Duration::from_secs(2u64.pow(attempt as u32 + 1));
                    tracing::warn!(
                        source = %parser.source_key(),
                        attempt = attempt + 1,
                        delay_secs = delay.as_secs(),
                        error = %e,
                        "Fetch failed, retrying"
                    );
                    sleep(delay).await;
                }
                last_err = Some(e);
            }
        }
    }

    Err(last_err.unwrap())
}
