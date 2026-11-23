use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// Parsed command-line arguments for the BigPicture launcher.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliArgs {
    monitor: Option<usize>,
    covers_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,
    exit_after_ms: Option<u64>,
    benchmark_scroll: bool,
}

impl CliArgs {
    pub fn from_env() -> Result<Self> {
        Self::parse(std::env::args().skip(1))
    }

    pub fn parse<I, S>(args: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut parsed = Self::default();
        let mut iter = args.into_iter();

        while let Some(raw) = iter.next() {
            let arg = raw.as_ref();
            if arg == "--monitor" || arg == "-m" {
                let value = iter
                    .next()
                    .context("missing monitor index after --monitor")?;
                parsed.monitor = Some(parse_usize(value.as_ref(), "--monitor")?);
            } else if let Some(value) = arg.strip_prefix("--monitor=") {
                parsed.monitor = Some(parse_usize(value, "--monitor")?);
            } else if arg == "--covers-dir" {
                let value = iter
                    .next()
                    .context("missing directory path after --covers-dir")?;
                parsed.covers_dir = Some(PathBuf::from(value.as_ref()));
            } else if let Some(value) = arg.strip_prefix("--covers-dir=") {
                parsed.covers_dir = Some(PathBuf::from(value));
            } else if arg == "--cache-dir" {
                let value = iter
                    .next()
                    .context("missing directory path after --cache-dir")?;
                parsed.cache_dir = Some(PathBuf::from(value.as_ref()));
            } else if let Some(value) = arg.strip_prefix("--cache-dir=") {
                parsed.cache_dir = Some(PathBuf::from(value));
            } else if arg == "--exit-after-ms" {
                let value = iter
                    .next()
                    .context("missing milliseconds value after --exit-after-ms")?;
                parsed.exit_after_ms = Some(parse_u64(value.as_ref(), "--exit-after-ms")?);
            } else if let Some(value) = arg.strip_prefix("--exit-after-ms=") {
                parsed.exit_after_ms = Some(parse_u64(value, "--exit-after-ms")?);
            } else if arg == "--benchmark-scroll" {
                parsed.benchmark_scroll = true;
            } else if arg == "--help" || arg == "-h" {
                println!(
                    "BigPicture Launcher\n\nUsage:\n  bigpicture-app [--monitor <N>] [--covers-dir <DIR>] [--cache-dir <DIR>] [--benchmark-scroll] [--exit-after-ms <MS>]"
                );
                std::process::exit(0);
            } else {
                bail!("unrecognized argument: {arg}");
            }
        }

        Ok(parsed)
    }

    pub fn monitor(&self) -> Option<usize> {
        self.monitor
    }

    pub fn covers_dir(&self) -> Option<&Path> {
        self.covers_dir.as_deref()
    }

    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    pub fn exit_after_ms(&self) -> Option<u64> {
        self.exit_after_ms
    }

    pub fn benchmark_scroll(&self) -> bool {
        self.benchmark_scroll
    }
}

fn parse_usize(raw: &str, flag: &str) -> Result<usize> {
    raw.parse::<usize>()
        .with_context(|| format!("invalid integer '{raw}' for {flag}"))
}

fn parse_u64(raw: &str, flag: &str) -> Result<u64> {
    raw.parse::<u64>()
        .with_context(|| format!("invalid integer '{raw}' for {flag}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_empty_args() {
        let args = CliArgs::parse(std::iter::empty::<&str>()).unwrap();
        assert_eq!(args.monitor(), None);
        assert_eq!(args.covers_dir(), None);
        assert_eq!(args.cache_dir(), None);
        assert_eq!(args.exit_after_ms(), None);
        assert!(!args.benchmark_scroll());
    }

    #[test]
    fn parses_monitor_covers_cache_and_benchmark_flags() {
        let parsed = CliArgs::parse([
            "--monitor",
            "1",
            "--covers-dir=assets/covers",
            "--cache-dir",
            "assets/cache",
            "--benchmark-scroll",
            "--exit-after-ms=500",
        ])
        .unwrap();
        assert_eq!(parsed.monitor(), Some(1));
        assert_eq!(parsed.covers_dir(), Some(Path::new("assets/covers")));
        assert_eq!(parsed.cache_dir(), Some(Path::new("assets/cache")));
        assert!(parsed.benchmark_scroll());
        assert_eq!(parsed.exit_after_ms(), Some(500));
    }

    #[test]
    fn rejects_invalid_or_missing_monitor_value() {
        assert!(CliArgs::parse(["--monitor"]).is_err());
        assert!(CliArgs::parse(["--monitor", "abc"]).is_err());
        assert!(CliArgs::parse(["--unknown"]).is_err());
    }
}
