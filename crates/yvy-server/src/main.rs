use std::{sync::Arc, time::Duration};

use clap::{Parser, Subcommand};
use tokio::sync::watch;
use tracing_subscriber::EnvFilter;
use yvy_server::{
    api, chesscom,
    config::Config,
    db,
    import::{self, ImportOptions, JobStatus, Progress},
    model::{self, Filters},
    state::AppState,
};

#[derive(Parser)]
#[command(name = "yvy-server", about = "You v You Chess: play against a model of your own chess.com history")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the HTTP server (the default).
    Serve,
    /// Import a chess.com account into the database.
    Import {
        username: String,
        /// Only import the most recent N monthly archives.
        #[arg(long)]
        months: Option<u32>,
    },
    /// Rebuild a player's position index from the stored games (no download).
    Reindex { username: String },
    /// Exit 0 if the server at YVY_BIND answers its health check (for Docker HEALTHCHECK).
    Health,
    /// Show what the player did from a position.
    Lookup {
        username: String,
        /// Full FEN of the position.
        fen: String,
        /// Comma-separated time classes, e.g. blitz,rapid.
        #[arg(long)]
        time_classes: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cli = Cli::parse();
    let config = Config::from_env()?;
    match cli.cmd.unwrap_or(Cmd::Serve) {
        Cmd::Serve => serve(config).await,
        Cmd::Import { username, months } => import_cmd(config, username, months).await,
        Cmd::Reindex { username } => reindex_cmd(config, username).await,
        Cmd::Health => health_cmd(config).await,
        Cmd::Lookup { username, fen, time_classes } => lookup_cmd(config, username, fen, time_classes).await,
    }
}

async fn serve(config: Config) -> anyhow::Result<()> {
    let pool = db::connect(&config.database_url).await?;
    import::spawn_backfill(pool.clone());
    let bind = config.bind;
    let state = AppState::new(pool, config);
    yvy_server::session::spawn_memory_backfill(state.clone());
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("listening on http://{bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn import_cmd(config: Config, username: String, months: Option<u32>) -> anyhow::Result<()> {
    let username = db::normalize_username(&username);
    let pool = db::connect(&config.database_url).await?;
    let client = Arc::new(chesscom::Client::new(&config.contact));
    let (tx, mut rx) = watch::channel(Progress::new(&username));
    let task = tokio::spawn(import::run_import(pool, client, username, ImportOptions { months }, tx));
    loop {
        let p = rx.borrow_and_update().clone();
        eprintln!(
            "[{}] {:<9} archives {}/{}  games seen {} new {} skipped {}  positions {}",
            p.status, p.phase, p.archives_done, p.archives_total, p.games_seen, p.games_new, p.games_skipped, p.positions_new
        );
        if p.status != JobStatus::Running {
            for e in &p.errors {
                eprintln!("  problem: {e}");
            }
            if let Some(m) = &p.message {
                eprintln!("  {m}");
            }
            break;
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
    task.await??;
    Ok(())
}

async fn health_cmd(config: Config) -> anyhow::Result<()> {
    let host = if config.bind.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        config.bind.ip().to_string()
    };
    let url = format!("http://{host}:{}/api/health", config.bind.port());
    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?
        .get(&url)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.is_success() && body.contains("\"ok\":true") {
        println!("{body}");
        Ok(())
    } else {
        anyhow::bail!("unhealthy: HTTP {status} {body}")
    }
}

async fn reindex_cmd(config: Config, username: String) -> anyhow::Result<()> {
    let username = db::normalize_username(&username);
    let pool = db::connect(&config.database_url).await?;
    let rows = import::reindex(&pool, &username).await?;
    println!("rebuilt {rows} position rows for {username}");
    Ok(())
}

async fn lookup_cmd(config: Config, username: String, fen: String, time_classes: Option<String>) -> anyhow::Result<()> {
    let username = db::normalize_username(&username);
    let pool = db::connect(&config.database_url).await?;
    let epd = model::key_from_fen(&fen).map_err(|e| anyhow::anyhow!(e))?;
    let filters = Filters {
        time_classes: time_classes.map(|s| s.split(',').map(|x| x.to_string()).collect()),
        ..Default::default()
    }
    .normalized();
    let candidates = model::candidates(&pool, &username, &epd, &filters).await?;
    let total = model::total_games(&candidates);
    println!("{username} faced {epd} in {total} game(s)");
    for c in &candidates {
        let pct = if total > 0 { 100.0 * c.count as f64 / total as f64 } else { 0.0 };
        println!("  {:<8} {:<6} {:>5}  {:>5.1}%  e.g. {}", c.san, c.uci, c.count, pct, c.game_ids.first().map(String::as_str).unwrap_or("-"));
    }
    Ok(())
}
