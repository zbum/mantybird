use std::sync::Arc;

use base64::Engine;
use futures::future::BoxFuture;
use serde::Deserialize;
use tauri::State;
use tokio::sync::Mutex;
use tracing::warn;

mod account;
mod cache;
mod config;
mod mail;

use account::Account;
use cache::Cache;
use config::StoredConfig;
use mail::idle as mail_idle;
use mail::imap as imap_client;
use mail::imap::ImapSession;
use mail::message::{Envelope, Folder, MessageBody, SpecialUse};
use mail::smtp;

#[derive(Debug, Deserialize)]
pub struct AttachmentPayload {
    pub filename: String,
    pub mime: String,
    pub data_base64: String,
}

struct AppState {
    session: Arc<Mutex<Option<ImapSession>>>,
    current_account: Arc<Mutex<Option<Account>>>,
    current_password: Arc<Mutex<Option<String>>>,
    current_mailbox: Arc<Mutex<Option<String>>>,
    cache: Cache,
    idle_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    idle_mailbox: Arc<Mutex<Option<String>>>,
    delimiter: Arc<Mutex<String>>,
    folder_poll_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

async fn ensure_idle_for(
    state: &AppState,
    app: &tauri::AppHandle,
    mailbox: &str,
) {
    {
        let current = state.idle_mailbox.lock().await;
        if current.as_deref() == Some(mailbox) {
            return; // already idling on this mailbox
        }
    }
    let account = match state.current_account.lock().await.clone() {
        Some(a) => a,
        None => return,
    };
    let password = match state.current_password.lock().await.clone() {
        Some(p) => p,
        None => return,
    };

    // Abort existing worker first.
    if let Some(prev) = state.idle_handle.lock().await.take() {
        prev.abort();
    }
    let mailbox_owned = mailbox.to_string();
    let app_clone = app.clone();
    let handle = tokio::spawn(async move {
        mail_idle::run(account, password, mailbox_owned, app_clone).await;
    });
    *state.idle_handle.lock().await = Some(handle);
    *state.idle_mailbox.lock().await = Some(mailbox.to_string());
}

async fn stop_idle(state: &AppState) {
    if let Some(prev) = state.idle_handle.lock().await.take() {
        prev.abort();
    }
    *state.idle_mailbox.lock().await = None;
}

fn spawn_folder_poll(
    session: Arc<Mutex<Option<ImapSession>>>,
    current_account: Arc<Mutex<Option<Account>>>,
    cache: Cache,
    app: tauri::AppHandle,
) -> tokio::task::JoinHandle<()> {
    use tauri::Emitter;
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(5 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let mut guard = session.lock().await;
            let Some(sess) = guard.as_mut() else { continue };
            match imap_client::list_folders(sess).await {
                Ok(folders) => {
                    drop(guard);
                    if let Some(acc) = current_account.lock().await.as_ref() {
                        let key = config::account_key(acc);
                        let _ = cache.put_folders(key, folders.clone()).await;
                    }
                    let _ = app.emit("folders:changed", folders);
                }
                Err(e) => {
                    drop(guard);
                    warn!(error = %e, "folder poll list failed; will retry next tick");
                }
            }
        }
    })
}

async fn start_folder_poll(state: &AppState, app: &tauri::AppHandle) {
    if let Some(prev) = state.folder_poll_handle.lock().await.take() {
        prev.abort();
    }
    let handle = spawn_folder_poll(
        state.session.clone(),
        state.current_account.clone(),
        state.cache.clone(),
        app.clone(),
    );
    *state.folder_poll_handle.lock().await = Some(handle);
}

async fn stop_folder_poll(state: &AppState) {
    if let Some(prev) = state.folder_poll_handle.lock().await.take() {
        prev.abort();
    }
}

/// Make sure `current_account` (and password) are populated.
/// Falls back to disk config + keychain if the in-memory state was
/// cleared (e.g. after a hot-reload of the Rust binary in dev mode).
async fn ensure_account_loaded(state: &AppState) -> Result<Account, String> {
    {
        let guard = state.current_account.lock().await;
        if let Some(a) = guard.as_ref() {
            return Ok(a.clone());
        }
    }
    let cfg = config::load();
    let key = cfg
        .current_key
        .clone()
        .ok_or_else(|| "no active session".to_string())?;
    let account = cfg
        .accounts
        .into_iter()
        .find(|a| config::account_key(a) == key)
        .ok_or_else(|| "no active session".to_string())?;
    let password = config::load_password(&account);
    *state.current_account.lock().await = Some(account.clone());
    *state.current_password.lock().await = password;
    Ok(account)
}

async fn ensure_connected(state: &AppState) -> Result<(), String> {
    if state.session.lock().await.is_some() {
        return Ok(());
    }
    let account = state
        .current_account
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no saved account".to_string())?;
    let password = state
        .current_password
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no saved password".to_string())?;
    let mut session = imap_client::connect(
        &account.host,
        account.port,
        &account.username,
        &password,
    )
    .await
    .map_err(|e| format!("reconnect failed: {e}"))?;

    if let Some(mailbox) = state.current_mailbox.lock().await.clone() {
        if let Err(e) = session.select(&mailbox).await {
            warn!(error = %e, "re-select mailbox after reconnect failed");
        }
    }
    *state.session.lock().await = Some(session);
    Ok(())
}

async fn with_imap<T, F>(state: &AppState, op: F) -> Result<T, String>
where
    F: for<'a> Fn(&'a mut ImapSession) -> BoxFuture<'a, anyhow::Result<T>>,
{
    // First attempt with whatever session we have (or none).
    let first_err: Option<String> = {
        let mut guard = state.session.lock().await;
        match guard.as_mut() {
            Some(session) => match op(session).await {
                Ok(v) => return Ok(v),
                Err(e) => Some(e.to_string()),
            },
            None => Some("no session".to_string()),
        }
    };

    warn!(error = ?first_err, "imap op failed; reconnecting and retrying once");
    *state.session.lock().await = None;
    ensure_connected(state).await?;

    let mut guard = state.session.lock().await;
    let session = guard
        .as_mut()
        .ok_or_else(|| "session unavailable after reconnect".to_string())?;
    op(session).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn connect_imap(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    host: String,
    port: u16,
    username: String,
    password: String,
) -> Result<Vec<Folder>, String> {
    let mut session = imap_client::connect(&host, port, &username, &password)
        .await
        .map_err(|e| e.to_string())?;
    let delim = imap_client::discover_delimiter(&mut session)
        .await
        .map_err(|e| e.to_string())?;
    *state.delimiter.lock().await = delim;
    let folders = imap_client::list_folders(&mut session)
        .await
        .map_err(|e| e.to_string())?;
    let account = Account {
        name: String::new(),
        host,
        port,
        username,
        smtp_host: String::new(),
        smtp_port: 465,
    };
    let key = config::account_key(&account);
    if let Err(e) = state.cache.put_folders(key, folders.clone()).await {
        warn!(error = %e, "cache put_folders failed");
    }
    *state.current_account.lock().await = Some(account);
    *state.current_password.lock().await = Some(password);
    *state.session.lock().await = Some(session);
    *state.current_mailbox.lock().await = None;
    start_folder_poll(&state, &app).await;
    Ok(folders)
}

#[tauri::command]
async fn disconnect_imap(state: State<'_, AppState>) -> Result<(), String> {
    stop_idle(&state).await;
    stop_folder_poll(&state).await;
    if let Some(mut session) = state.session.lock().await.take() {
        let _ = session.logout().await;
    }
    *state.current_account.lock().await = None;
    *state.current_password.lock().await = None;
    *state.current_mailbox.lock().await = None;
    Ok(())
}

#[tauri::command]
async fn fetch_envelopes(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    mailbox: String,
    offset: u32,
    limit: u32,
) -> Result<Vec<Envelope>, String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    // Record the mailbox up-front so subsequent commands (fetch_body,
    // download_attachment, mark_seen) know which folder to operate on
    // even if this fetch fails.
    *state.current_mailbox.lock().await = Some(mailbox.clone());
    let mailbox_owned = mailbox.clone();
    let envs = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        Box::pin(async move { imap_client::fetch_envelopes(sess, &mb, offset, limit).await })
    })
    .await?;
    let mailbox_for_cache = mailbox.clone();
    if let Err(e) = state
        .cache
        .put_envelopes(key, mailbox_for_cache, envs.clone())
        .await
    {
        warn!(error = %e, "cache put_envelopes failed");
    }
    // Start (or rebind) IDLE worker to push new-mail notifications.
    ensure_idle_for(&state, &app, &mailbox).await;
    Ok(envs)
}

#[tauri::command]
async fn search_mailbox(
    state: State<'_, AppState>,
    mailbox: String,
    query: String,
    limit: u32,
) -> Result<Vec<Envelope>, String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    *state.current_mailbox.lock().await = Some(mailbox.clone());

    let mailbox_owned = mailbox.clone();
    let query_owned = query.clone();
    let uids = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        let q = query_owned.clone();
        Box::pin(async move { imap_client::search_uids(sess, &mb, &q).await })
    })
    .await?;

    let mailbox_owned = mailbox.clone();
    let uids_clone = uids.clone();
    let envs = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        let u = uids_clone.clone();
        let lim = limit as usize;
        Box::pin(async move {
            imap_client::fetch_envelopes_by_uids(sess, &mb, &u, lim).await
        })
    })
    .await?;

    // Persist into the cache so subsequent local-search calls and offline
    // viewing can find them.
    if let Err(e) = state
        .cache
        .put_envelopes(key, mailbox, envs.clone())
        .await
    {
        warn!(error = %e, "cache put_envelopes (search) failed");
    }
    Ok(envs)
}

#[tauri::command]
async fn fetch_body(
    state: State<'_, AppState>,
    mailbox: String,
    uid: u32,
) -> Result<MessageBody, String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    *state.current_mailbox.lock().await = Some(mailbox.clone());

    // Always fetch live so the body reflects the latest server state
    // (e.g., new attachments metadata, schema changes). Frontend can read
    // from `cached_body` separately for instant display.
    let mailbox_owned = mailbox.clone();
    let body = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        Box::pin(async move { imap_client::fetch_body(sess, &mb, uid).await })
    })
    .await?;

    if let Err(e) = state
        .cache
        .put_body(key, mailbox, uid, body.clone())
        .await
    {
        warn!(error = %e, "cache put_body failed");
    }
    Ok(body)
}

#[derive(Debug, serde::Serialize)]
pub struct DownloadResult {
    pub filename: String,
    pub mime: String,
    pub data_base64: String,
    pub saved_path: Option<String>,
}

#[tauri::command]
async fn download_attachment(
    state: State<'_, AppState>,
    mailbox: String,
    uid: u32,
    index: usize,
) -> Result<DownloadResult, String> {
    *state.current_mailbox.lock().await = Some(mailbox.clone());
    let mailbox_owned = mailbox.clone();
    let (filename, mime, bytes) = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        Box::pin(async move {
            imap_client::fetch_attachment_bytes(sess, &mb, uid, index).await
        })
    })
    .await?;

    let saved_path = match config::load().download_dir {
        Some(dir) => Some(
            write_to_dir(&dir, &filename, &bytes).map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let data_base64 = if saved_path.is_some() {
        String::new()
    } else {
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    };
    Ok(DownloadResult {
        filename,
        mime,
        data_base64,
        saved_path,
    })
}

fn write_to_dir(dir: &str, filename: &str, bytes: &[u8]) -> std::io::Result<String> {
    let dir_path = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir_path)?;
    let safe_name = sanitize_filename(filename);
    let path = unique_path(&dir_path, &safe_name);
    std::fs::write(&path, bytes)?;
    Ok(path.display().to_string())
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            _ => c,
        })
        .collect();
    if cleaned.trim().is_empty() {
        "attachment".to_string()
    } else {
        cleaned
    }
}

fn unique_path(dir: &std::path::Path, filename: &str) -> std::path::PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }
    let path = std::path::Path::new(filename);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| filename.to_string());
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for i in 1..1000 {
        let cand = dir.join(format!("{stem} ({i}){ext}"));
        if !cand.exists() {
            return cand;
        }
    }
    candidate
}

#[tauri::command]
fn reveal_in_file_manager(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(format!("file does not exist: {path}"));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| format!("open failed: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn()
            .map_err(|e| format!("explorer failed: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let dir = p.parent().unwrap_or(std::path::Path::new("."));
        std::process::Command::new("xdg-open")
            .arg(dir)
            .spawn()
            .map_err(|e| format!("xdg-open failed: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // Allow only http(s) / mailto, to avoid arbitrary file:// or shell escapes.
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:"))
    {
        return Err(format!("blocked unsupported url scheme: {url}"));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut cmd = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    };
    cmd.arg(&url)
        .spawn()
        .map_err(|e| format!("open failed: {e}"))?;
    Ok(())
}

#[tauri::command]
fn get_last_mailbox() -> Option<String> {
    config::load().last_mailbox
}

#[tauri::command]
fn set_last_mailbox(mailbox: Option<String>) -> Result<StoredConfig, String> {
    config::set_last_mailbox(mailbox).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_expanded_folders() -> Vec<String> {
    config::load().expanded_folders
}

#[tauri::command]
fn set_expanded_folders(paths: Vec<String>) -> Result<StoredConfig, String> {
    config::set_expanded_folders(paths).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_folder_labels() -> std::collections::HashMap<String, String> {
    config::load()
        .folder_labels
        .unwrap_or_else(config::default_folder_labels)
}

#[tauri::command]
fn set_folder_labels(
    labels: std::collections::HashMap<String, String>,
) -> Result<StoredConfig, String> {
    config::set_folder_labels(labels).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_download_dir() -> Option<String> {
    config::load().download_dir
}

#[tauri::command]
fn set_download_dir(dir: Option<String>) -> Result<StoredConfig, String> {
    config::set_download_dir(dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_mark_seen_delay() -> u32 {
    config::load().mark_seen_delay_seconds
}

#[tauri::command]
fn set_mark_seen_delay(seconds: u32) -> Result<StoredConfig, String> {
    config::set_mark_seen_delay_seconds(seconds).map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_flag(
    state: State<'_, AppState>,
    mailbox: String,
    uid: u32,
    flag: String,
    on: bool,
) -> Result<Vec<String>, String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    *state.current_mailbox.lock().await = Some(mailbox.clone());
    let mailbox_owned = mailbox.clone();
    let flag_owned = flag.clone();
    let flags = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        let f = flag_owned.clone();
        Box::pin(async move { imap_client::set_flag(sess, &mb, uid, &f, on).await })
    })
    .await?;
    let seen = flags.iter().any(|s| s.eq_ignore_ascii_case("\\Seen"));
    if let Err(e) = state
        .cache
        .set_seen(key, mailbox, uid, seen, flags.clone())
        .await
    {
        warn!(error = %e, "cache set_flag persistence failed");
    }
    Ok(flags)
}

#[tauri::command]
async fn move_to_trash(
    state: State<'_, AppState>,
    mailbox: String,
    uid: u32,
) -> Result<(), String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    let folders = state
        .cache
        .get_folders(key.clone())
        .await
        .map_err(|e| e.to_string())?;
    let trash = folders
        .iter()
        .find(|f| matches!(f.special, SpecialUse::Trash))
        .map(|f| f.raw.clone())
        .ok_or_else(|| "Trash folder not found".to_string())?;
    if trash == mailbox {
        return Err("이미 휴지통의 메일입니다. 영구 삭제는 별도로 구현 필요.".to_string());
    }
    *state.current_mailbox.lock().await = Some(mailbox.clone());
    let mailbox_owned = mailbox.clone();
    let trash_owned = trash.clone();
    with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        let t = trash_owned.clone();
        Box::pin(async move { imap_client::move_message(sess, &mb, uid, &t).await })
    })
    .await?;
    Ok(())
}

#[tauri::command]
async fn mark_seen(
    state: State<'_, AppState>,
    mailbox: String,
    uid: u32,
    seen: bool,
) -> Result<Vec<String>, String> {
    let account = ensure_account_loaded(&state).await?;
    let key = config::account_key(&account);
    *state.current_mailbox.lock().await = Some(mailbox.clone());
    let mailbox_owned = mailbox.clone();
    let flags = with_imap(&state, move |sess| {
        let mb = mailbox_owned.clone();
        Box::pin(async move { imap_client::mark_seen(sess, &mb, uid, seen).await })
    })
    .await?;

    if let Err(e) = state
        .cache
        .set_seen(key, mailbox, uid, seen, flags.clone())
        .await
    {
        warn!(error = %e, "cache set_seen failed");
    }
    Ok(flags)
}

#[tauri::command]
async fn create_mailbox(
    state: State<'_, AppState>,
    parent_raw: Option<String>,
    name: String,
) -> Result<Vec<Folder>, String> {
    let delim = state.delimiter.lock().await.clone();
    let leaf_raw = utf7_imap::encode_utf7_imap(name);
    let raw = match parent_raw.as_deref() {
        Some(p) if !p.is_empty() => format!("{p}{delim}{leaf_raw}"),
        _ => leaf_raw,
    };
    with_imap(&state, move |sess| {
        let r = raw.clone();
        Box::pin(async move { imap_client::create_mailbox(sess, &r).await })
    })
    .await?;
    refresh_folders(&state).await
}

#[tauri::command]
async fn rename_mailbox(
    state: State<'_, AppState>,
    from_raw: String,
    new_leaf: String,
) -> Result<Vec<Folder>, String> {
    let delim = state.delimiter.lock().await.clone();
    let parent = from_raw
        .rsplit_once(delim.as_str())
        .map(|(p, _)| p.to_string())
        .unwrap_or_default();
    let new_leaf_raw = utf7_imap::encode_utf7_imap(new_leaf);
    let new_raw = if parent.is_empty() {
        new_leaf_raw
    } else {
        format!("{parent}{delim}{new_leaf_raw}")
    };
    let from = from_raw.clone();
    with_imap(&state, move |sess| {
        let f = from.clone();
        let t = new_raw.clone();
        Box::pin(async move { imap_client::rename_mailbox(sess, &f, &t).await })
    })
    .await?;
    refresh_folders(&state).await
}

#[tauri::command]
async fn subscribe_mailbox(
    state: State<'_, AppState>,
    raw: String,
) -> Result<Vec<Folder>, String> {
    let r = raw.clone();
    with_imap(&state, move |sess| {
        let r = r.clone();
        Box::pin(async move { imap_client::subscribe_mailbox(sess, &r).await })
    })
    .await?;
    refresh_folders(&state).await
}

#[tauri::command]
async fn unsubscribe_mailbox(
    state: State<'_, AppState>,
    raw: String,
) -> Result<Vec<Folder>, String> {
    let r = raw.clone();
    with_imap(&state, move |sess| {
        let r = r.clone();
        Box::pin(async move { imap_client::unsubscribe_mailbox(sess, &r).await })
    })
    .await?;
    refresh_folders(&state).await
}

#[tauri::command]
async fn delete_mailbox(
    state: State<'_, AppState>,
    raw: String,
) -> Result<Vec<Folder>, String> {
    with_imap(&state, move |sess| {
        let r = raw.clone();
        Box::pin(async move { imap_client::delete_mailbox(sess, &r).await })
    })
    .await?;
    refresh_folders(&state).await
}

async fn refresh_folders(state: &AppState) -> Result<Vec<Folder>, String> {
    let folders = with_imap(state, move |sess| {
        Box::pin(async move { imap_client::list_folders(sess).await })
    })
    .await?;
    if let Some(account) = state.current_account.lock().await.as_ref() {
        let key = config::account_key(account);
        if let Err(e) = state.cache.put_folders(key, folders.clone()).await {
            warn!(error = %e, "cache refresh failed");
        }
    }
    Ok(folders)
}

#[tauri::command]
async fn cached_folders(
    state: State<'_, AppState>,
    account: Account,
) -> Result<Vec<Folder>, String> {
    state
        .cache
        .get_folders(config::account_key(&account))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cached_envelopes(
    state: State<'_, AppState>,
    account: Account,
    mailbox: String,
    before_uid: Option<u32>,
    limit: u32,
) -> Result<Vec<Envelope>, String> {
    state
        .cache
        .get_envelopes(config::account_key(&account), mailbox, before_uid, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cached_body(
    state: State<'_, AppState>,
    account: Account,
    mailbox: String,
    uid: u32,
) -> Result<Option<MessageBody>, String> {
    state
        .cache
        .get_body(config::account_key(&account), mailbox, uid)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_mail(
    state: State<'_, AppState>,
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body: String,
    html: Option<String>,
    attachments: Vec<AttachmentPayload>,
) -> Result<(), String> {
    let in_memory = state
        .current_account
        .lock()
        .await
        .clone()
        .ok_or_else(|| "no active account".to_string())?;
    let key = config::account_key(&in_memory);
    // Always read the latest from disk so SMTP settings edited via the
    // Settings page take effect without requiring a reconnect.
    let account = config::load()
        .accounts
        .into_iter()
        .find(|a| config::account_key(a) == key)
        .unwrap_or(in_memory);
    let password = state
        .current_password
        .lock()
        .await
        .clone()
        .or_else(|| config::load_password(&account))
        .ok_or_else(|| "no saved password".to_string())?;

    let from = smtp::build_from(&account).map_err(|e| e.to_string())?;
    let to = smtp::parse_addresses(&to).map_err(|e| e.to_string())?;
    let cc = smtp::parse_addresses(&cc).map_err(|e| e.to_string())?;
    let bcc = smtp::parse_addresses(&bcc).map_err(|e| e.to_string())?;
    if to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err("at least one recipient required".to_string());
    }

    let decoded_attachments: Vec<smtp::OutgoingAttachment> = attachments
        .into_iter()
        .map(|a| {
            let data = base64::engine::general_purpose::STANDARD
                .decode(a.data_base64.as_bytes())
                .map_err(|e| format!("base64 decode {}: {e}", a.filename))?;
            Ok::<_, String>(smtp::OutgoingAttachment {
                filename: a.filename,
                mime: a.mime,
                data,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let raw = smtp::send_mail(
        &account,
        &password,
        smtp::OutgoingMail {
            from,
            to,
            cc,
            bcc,
            subject,
            body,
            html,
            attachments: decoded_attachments,
        },
    )
    .await
    .map_err(|e| e.to_string())?;

    // Save to Sent folder if the server doesn't do it automatically.
    if smtp::auto_save_to_sent(&account) {
        let folders = state
            .cache
            .get_folders(key)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(sent_path) = folders
            .iter()
            .find(|f| matches!(f.special, SpecialUse::Sent))
            .map(|f| f.raw.clone())
        {
            let raw_bytes = raw.clone();
            let path_owned = sent_path.clone();
            let result = with_imap(&state, move |sess| {
                let path = path_owned.clone();
                let bytes = raw_bytes.clone();
                Box::pin(async move {
                    imap_client::append_message(sess, &path, &bytes, Some("(\\Seen)")).await
                })
            })
            .await;
            if let Err(e) = result {
                warn!(error = %e, "Sent APPEND failed (mail was sent OK)");
            }
        }
    }

    Ok(())
}

#[tauri::command]
fn list_accounts() -> StoredConfig {
    config::load()
}

#[tauri::command]
fn current_account() -> Option<Account> {
    let cfg = config::load();
    let key = cfg.current_key.clone()?;
    cfg.accounts
        .into_iter()
        .find(|a| config::account_key(a) == key)
}

#[tauri::command]
fn upsert_account(account: Account) -> Result<StoredConfig, String> {
    config::upsert_account(account).map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_account(account: Account) -> Result<StoredConfig, String> {
    let _ = config::delete_password(&account);
    config::delete_account(config::account_key(&account)).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_current_account(account: Account) -> Result<StoredConfig, String> {
    config::set_current(config::account_key(&account)).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_password(account: Account, password: String) -> Result<(), String> {
    config::save_password(&account, &password).map_err(|e| e.to_string())
}

#[tauri::command]
fn load_password(account: Account) -> Option<String> {
    config::load_password(&account)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let cache = Cache::open().expect("failed to open cache");

    tauri::Builder::default()
        .manage(AppState {
            session: Arc::new(Mutex::new(None)),
            current_account: Arc::new(Mutex::new(None)),
            current_password: Arc::new(Mutex::new(None)),
            current_mailbox: Arc::new(Mutex::new(None)),
            cache,
            idle_handle: Arc::new(Mutex::new(None)),
            idle_mailbox: Arc::new(Mutex::new(None)),
            delimiter: Arc::new(Mutex::new("/".to_string())),
            folder_poll_handle: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            connect_imap,
            disconnect_imap,
            fetch_envelopes,
            search_mailbox,
            fetch_body,
            download_attachment,
            get_download_dir,
            set_download_dir,
            get_mark_seen_delay,
            set_mark_seen_delay,
            get_folder_labels,
            set_folder_labels,
            get_last_mailbox,
            set_last_mailbox,
            get_expanded_folders,
            set_expanded_folders,
            open_url,
            reveal_in_file_manager,
            mark_seen,
            set_flag,
            move_to_trash,
            create_mailbox,
            rename_mailbox,
            delete_mailbox,
            subscribe_mailbox,
            unsubscribe_mailbox,
            cached_folders,
            cached_envelopes,
            cached_body,
            send_mail,
            list_accounts,
            current_account,
            upsert_account,
            delete_account,
            set_current_account,
            save_password,
            load_password,
        ])
        .setup(|app| {
            use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
            use tauri::Emitter;

            let preferences = MenuItemBuilder::new("환경설정...")
                .id("open-settings")
                .accelerator("Cmd+,")
                .build(app)?;
            let quit = MenuItemBuilder::new("Mantybird 종료")
                .id("quit")
                .accelerator("Cmd+Q")
                .build(app)?;
            let app_menu = SubmenuBuilder::new(app, "Mantybird")
                .item(&preferences)
                .separator()
                .item(&quit)
                .build()?;
            let menu = MenuBuilder::new(app).item(&app_menu).build()?;
            app.set_menu(menu)?;

            let handle = app.handle().clone();
            app.on_menu_event(move |_app, event| match event.id().0.as_str() {
                "open-settings" => {
                    let _ = handle.emit("settings:open", ());
                }
                "quit" => std::process::exit(0),
                _ => {}
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
