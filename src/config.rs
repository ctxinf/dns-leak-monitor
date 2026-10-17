use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RuntimeConfig {
    pub interface: String,
    pub exclude: Vec<String>,
}

pub struct Runtime {
    pub settings: RuntimeConfig,
    pub status: String,
    pub path: std::path::PathBuf,
}
pub type Shared = Arc<Mutex<Runtime>>;

impl Runtime {
    pub fn load(cfg: &Config, path: std::path::PathBuf) -> Self {
        let mut settings = RuntimeConfig {
            interface: cfg.interface.clone(),
            exclude: cfg.normalized_exclude(),
        };
        let mut status = "Starting capture".to_string();
        if path.exists() {
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|s| toml::from_str::<RuntimeConfig>(&s).map_err(|e| e.to_string()))
            {
                Ok(c) => settings = c,
                Err(e) => {
                    eprintln!("runtime config: {e}");
                    status = format!("Runtime config ignored: {e}");
                }
            }
        }
        settings.exclude = settings
            .exclude
            .iter()
            .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Self {
            settings,
            status,
            path,
        }
    }
    pub fn save(&mut self, mut settings: RuntimeConfig) -> Result<(), String> {
        settings.interface = settings.interface.trim().to_string();
        if settings.interface.is_empty() {
            return Err("Interface must not be empty".into());
        }
        settings.exclude = settings
            .exclude
            .iter()
            .map(|s| s.trim().trim_end_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        let raw = toml::to_string(&settings).map_err(|e| e.to_string())?;
        let temp = self.path.with_extension("toml.tmp");
        std::fs::write(&temp, raw)
            .and_then(|_| std::fs::rename(&temp, &self.path))
            .map_err(|e| format!("Cannot save {}: {e}", self.path.display()))?;
        self.settings = settings;
        Ok(())
    }
}

#[cfg(test)]
mod runtime_tests {
    use super::*;
    #[test]
    fn persist_override_and_failed_save() {
        let dir = std::env::temp_dir().join(format!("dns-runtime-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = Config {
            interface: "default0".into(),
            db_path: "unused".into(),
            bind: "unused".into(),
            retention_days: 1,
            exclude: vec![],
        };
        let path = dir.join("config-runtime.toml");
        let mut state = Runtime::load(&cfg, path.clone());
        let next = RuntimeConfig {
            interface: "missing0".into(),
            exclude: vec![" EXAMPLE.COM. ".into()],
        };
        state.save(next).unwrap();
        let loaded = Runtime::load(&cfg, path);
        assert_eq!(loaded.settings.interface, "missing0");
        assert_eq!(loaded.settings.exclude, vec!["example.com"]);
        let before = state.settings.clone();
        state.path = dir.join("absent/config-runtime.toml");
        assert!(state
            .save(RuntimeConfig {
                interface: "other0".into(),
                exclude: vec![]
            })
            .is_err());
        assert_eq!(state.settings, before);
        assert!(state
            .save(RuntimeConfig {
                interface: " ".into(),
                exclude: vec![]
            })
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
