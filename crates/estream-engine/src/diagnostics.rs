//! Logging setup: a timestamped log file per run (keeping the 10 most recent)
//! plus stderr output, filtered by `RUST_LOG` (default `info`).

use std::path::{Path, PathBuf};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

pub const KEEP_LOG_FILES: usize = 10;

/// Keep this alive for the life of the process so buffered log lines are flushed.
pub struct LogGuard {
    _worker: WorkerGuard,
    pub path: PathBuf,
}

/// Initializes logging into `dir`. Safe to call once per process.
pub fn init_logging(dir: &Path) -> std::io::Result<LogGuard> {
    std::fs::create_dir_all(dir)?;
    let name = format!(
        "estream-{}.log",
        chrono::Local::now().format("%Y%m%d-%H%M%S%.3f")
    );
    let path = dir.join(name);
    let file = std::fs::File::create(&path)?;
    prune_logs(dir, KEEP_LOG_FILES)?;

    let (writer, worker) = tracing_appender::non_blocking(file);
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_writer(writer).with_ansi(false))
        .with(fmt::layer().with_writer(std::io::stderr))
        .try_init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "eStream logging started"
    );
    Ok(LogGuard {
        _worker: worker,
        path,
    })
}

/// Deletes all but the `keep` most recent `estream-*.log` files in `dir`.
pub fn prune_logs(dir: &Path, keep: usize) -> std::io::Result<()> {
    let mut logs: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("estream-") && n.ends_with(".log"))
        })
        .collect();
    // Names embed a sortable timestamp.
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        std::fs::remove_file(old)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_most_recent() {
        let dir = std::env::temp_dir().join(format!("estream-logs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..15 {
            std::fs::write(dir.join(format!("estream-20260101-0000{i:02}.000.log")), "").unwrap();
        }
        std::fs::write(dir.join("other.txt"), "").unwrap();
        prune_logs(&dir, 10).unwrap();
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left.len(), 11);
        assert_eq!(left[0], "estream-20260101-000005.000.log");
        assert!(left.contains(&"other.txt".to_owned()));
    }
}
