use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub interface: String,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(default)]
    pub exclude: Vec<String>,
}

fn default_db_path() -> String {
    "/var/lib/dns-leak-monitor/dns.db".into()
}

fn default_bind() -> String {
    "127.0.0.1:28107".into()
}

fn default_retention_days() -> u32 {
    90
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let cfg: Config =
            toml::from_str(&raw).map_err(|e| format!("bad TOML in {}: {e}", path.display()))?;
        // Never fall back to a default NIC — capturing the wrong interface
        // silently defeats the whole tool.
        if cfg.interface.trim().is_empty() {
            return Err("config: `interface` is required and must be non-empty".into());
        }
        if cfg.retention_days == 0 {
            return Err("config: `retention_days` must be >= 1".into());
        }
        Ok(cfg)
    }

    /// Exclude entries normalized for matching: lowercase, no trailing dot.
    pub fn normalized_exclude(&self) -> Vec<String> {
        self.exclude
            .iter()
            .map(|e| e.trim_end_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect()
    }
}
