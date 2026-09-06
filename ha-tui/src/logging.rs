use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Initializes file logging at `~/.local/share/ha-tui/app.log`.
/// Must never write to stdout, since stdout is the TUI surface.
/// The returned guard must be held for the lifetime of the app -
/// dropping it flushes and stops the background writer.
pub fn init() -> Result<WorkerGuard> {
    let dirs = directories::ProjectDirs::from("", "", "ha-tui")
        .context("could not determine home directory for log path")?;
    let log_dir = dirs.data_dir();
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create log directory at {}", log_dir.display()))?;

    let file_appender = tracing_appender::rolling::never(log_dir, "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    Ok(guard)
}
