use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::account::Account;

pub fn default_folder_labels() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("Inbox".into(), "받은 메일함".into());
    m.insert("Sent".into(), "보낸 메일함".into());
    m.insert("Drafts".into(), "임시 보관함".into());
    m.insert("Archive".into(), "보관 메일함".into());
    m.insert("Junk".into(), "스팸 메일함".into());
    m.insert("Trash".into(), "휴지통".into());
    m
}

const KEYRING_SERVICE: &str = "manty-imap-desktop";

pub fn account_key(account: &Account) -> String {
    format!("{}@{}:{}", account.username, account.host, account.port)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredConfig {
    #[serde(default)]
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub current_key: Option<String>,
    #[serde(default)]
    pub download_dir: Option<String>,
    #[serde(default)]
    pub folder_labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub mark_seen_delay_seconds: u32,
    #[serde(default)]
    pub last_mailbox: Option<String>,
    #[serde(default)]
    pub expanded_folders: Vec<String>,
    // Back-compat with the old single-account schema.
    #[serde(default, skip_serializing)]
    pub account: Option<Account>,
}

pub fn set_last_mailbox(mailbox: Option<String>) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.last_mailbox = mailbox;
    save(&cfg)?;
    Ok(cfg)
}

pub fn set_expanded_folders(paths: Vec<String>) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.expanded_folders = paths;
    save(&cfg)?;
    Ok(cfg)
}

pub fn set_folder_labels(labels: HashMap<String, String>) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.folder_labels = Some(labels);
    save(&cfg)?;
    Ok(cfg)
}

pub fn set_download_dir(dir: Option<String>) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.download_dir = dir.filter(|s| !s.trim().is_empty());
    save(&cfg)?;
    Ok(cfg)
}

pub fn set_mark_seen_delay_seconds(seconds: u32) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.mark_seen_delay_seconds = seconds;
    save(&cfg)?;
    Ok(cfg)
}

impl StoredConfig {
    fn migrate(mut self) -> Self {
        if let Some(legacy) = self.account.take() {
            if !self.accounts.iter().any(|a| account_key(a) == account_key(&legacy)) {
                self.accounts.push(legacy.clone());
            }
            if self.current_key.is_none() {
                self.current_key = Some(account_key(&legacy));
            }
        }
        self
    }
}

fn config_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "manty", "manty-imap-desktop")
        .ok_or_else(|| anyhow!("no platform config directory"))?;
    let dir = dirs.config_dir().to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir.join("config.json"))
}

pub fn load() -> StoredConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(_) => return StoredConfig::default(),
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(_) => return StoredConfig::default(),
    };
    serde_json::from_str::<StoredConfig>(&raw)
        .unwrap_or_default()
        .migrate()
}

pub fn save(cfg: &StoredConfig) -> Result<()> {
    let path = config_path()?;
    let raw = serde_json::to_string_pretty(cfg)?;
    std::fs::write(&path, raw)?;
    Ok(())
}

pub fn upsert_account(account: Account) -> Result<StoredConfig> {
    let mut cfg = load();
    let key = account_key(&account);
    if let Some(existing) = cfg.accounts.iter_mut().find(|a| account_key(a) == key) {
        *existing = account;
    } else {
        cfg.accounts.push(account);
    }
    if cfg.current_key.is_none() {
        cfg.current_key = Some(key);
    }
    save(&cfg)?;
    Ok(cfg)
}

pub fn delete_account(account_key_arg: String) -> Result<StoredConfig> {
    let mut cfg = load();
    cfg.accounts.retain(|a| account_key(a) != account_key_arg);
    if cfg.current_key.as_deref() == Some(account_key_arg.as_str()) {
        cfg.current_key = cfg.accounts.first().map(account_key);
    }
    save(&cfg)?;
    Ok(cfg)
}

pub fn set_current(key: String) -> Result<StoredConfig> {
    let mut cfg = load();
    if cfg.accounts.iter().any(|a| account_key(a) == key) {
        cfg.current_key = Some(key);
        save(&cfg)?;
    }
    Ok(cfg)
}

fn keyring_key(account: &Account) -> String {
    format!("{}@{}", account.username, account.host)
}

pub fn save_password(account: &Account, password: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &keyring_key(account))?;
    entry.set_password(password)?;
    Ok(())
}

pub fn load_password(account: &Account) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_key(account))
        .ok()?
        .get_password()
        .ok()
}

pub fn delete_password(account: &Account) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, &keyring_key(account))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}
