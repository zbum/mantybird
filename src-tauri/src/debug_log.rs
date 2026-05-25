use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub ts: i64,
    pub channel: String,
    pub direction: String,
    pub text: String,
}

pub struct DebugLog {
    entries: Mutex<VecDeque<LogEntry>>,
    next_id: AtomicU64,
    app: OnceLock<AppHandle>,
}

impl DebugLog {
    fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::with_capacity(MAX_ENTRIES)),
            next_id: AtomicU64::new(1),
            app: OnceLock::new(),
        }
    }

    pub fn bind(&self, app: AppHandle) {
        let _ = self.app.set(app);
    }

    fn record(&self, channel: &str, direction: &str, text: String) {
        let entry = LogEntry {
            id: self.next_id.fetch_add(1, Ordering::Relaxed),
            ts: now_ms(),
            channel: channel.to_string(),
            direction: direction.to_string(),
            text,
        };
        if let Ok(mut guard) = self.entries.lock() {
            if guard.len() >= MAX_ENTRIES {
                guard.pop_front();
            }
            guard.push_back(entry.clone());
        }
        if let Some(app) = self.app.get() {
            let _ = app.emit("debug:log", &entry);
        }
    }

    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries
            .lock()
            .map(|g| g.iter().cloned().collect())
            .unwrap_or_default()
    }
}

static DEBUG_LOG: OnceLock<Arc<DebugLog>> = OnceLock::new();

pub fn init() -> Arc<DebugLog> {
    let log = Arc::new(DebugLog::new());
    let _ = DEBUG_LOG.set(log.clone());
    log
}

pub fn handle() -> Option<Arc<DebugLog>> {
    DEBUG_LOG.get().cloned()
}

pub fn push(channel: &str, direction: &str, text: impl Into<String>) {
    if let Some(log) = DEBUG_LOG.get() {
        log.record(channel, direction, text.into());
    }
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
