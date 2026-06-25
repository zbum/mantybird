use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use directories::ProjectDirs;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json;

use crate::mail::message::{AttachmentMeta, Envelope, Folder, MessageBody, SpecialUse};

#[derive(Clone)]
pub struct Cache {
    conn: Arc<Mutex<Connection>>,
}

fn db_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("dev", "manty", "manty-imap-desktop")
        .ok_or_else(|| anyhow!("no platform config directory"))?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
    Ok(dir.join("cache.sqlite"))
}

impl Cache {
    pub fn open() -> Result<Self> {
        let path = db_path()?;
        let conn = Connection::open(&path).with_context(|| format!("open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn put_folders(&self, account_key: String, folders: Vec<Folder>) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let tx = guard.transaction()?;
            tx.execute(
                "DELETE FROM folders WHERE account_key = ?1",
                params![account_key],
            )?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO folders (account_key, raw, leaf, depth, parent_path, has_children, special, ord, subscribed, unread_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )?;
                for (i, f) in folders.iter().enumerate() {
                    stmt.execute(params![
                        account_key,
                        f.raw,
                        f.leaf,
                        f.depth,
                        f.parent_path,
                        f.has_children as i64,
                        serde_json::to_string(&f.special)?,
                        i as i64,
                        f.subscribed as i64,
                        f.unread_count as i64,
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn get_folders(&self, account_key: String) -> Result<Vec<Folder>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<Folder>> {
            let guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let mut stmt = guard.prepare(
                "SELECT raw, leaf, depth, parent_path, has_children, special,
                        COALESCE(subscribed, 1) AS subscribed,
                        COALESCE(unread_count, 0) AS unread_count
                 FROM folders WHERE account_key = ?1 ORDER BY ord",
            )?;
            let rows = stmt.query_map(params![account_key], |row| {
                let special_json: String = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u16,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)? != 0,
                    special_json,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, i64>(7)? as u32,
                ))
            })?;
            let mut out = Vec::new();
            for r in rows {
                let (
                    raw,
                    leaf,
                    depth,
                    parent_path,
                    has_children,
                    special_json,
                    subscribed,
                    unread_count,
                ) = r?;
                let special: SpecialUse = serde_json::from_str(&special_json)?;
                out.push(Folder {
                    raw,
                    leaf,
                    depth,
                    parent_path,
                    has_children,
                    special,
                    subscribed,
                    unread_count,
                });
            }
            Ok(out)
        })
        .await?
    }

    pub async fn put_envelopes(
        &self,
        account_key: String,
        mailbox: String,
        envelopes: Vec<Envelope>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let tx = guard.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO messages (account_key, mailbox, uid, subject, from_addr, date, flags, seen, message_id, in_reply_to, message_references)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                     ON CONFLICT(account_key, mailbox, uid) DO UPDATE SET
                       subject = excluded.subject,
                       from_addr = excluded.from_addr,
                       date = excluded.date,
                       flags = excluded.flags,
                       seen = excluded.seen,
                       message_id = excluded.message_id,
                       in_reply_to = excluded.in_reply_to,
                       message_references = excluded.message_references",
                )?;
                for e in &envelopes {
                    stmt.execute(params![
                        account_key,
                        mailbox,
                        e.uid,
                        e.subject,
                        e.from,
                        e.date,
                        serde_json::to_string(&e.flags)?,
                        e.seen as i64,
                        e.message_id,
                        e.in_reply_to,
                        serde_json::to_string(&e.references)?,
                    ])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn get_envelopes(
        &self,
        account_key: String,
        mailbox: String,
        before_uid: Option<u32>,
        limit: u32,
    ) -> Result<Vec<Envelope>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<Envelope>> {
            let guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let mut stmt = guard.prepare(
                "SELECT uid, subject, from_addr, date, flags, seen, message_id, in_reply_to, message_references
                 FROM messages
                 WHERE account_key = ?1 AND mailbox = ?2
                   AND (?3 IS NULL OR uid < ?3)
                 ORDER BY uid DESC LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                params![account_key, mailbox, before_uid.map(|u| u as i64), limit as i64],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u32,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)? != 0,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                },
            )?;
            let mut out = Vec::new();
            for r in rows {
                let (uid, subject, from, date, flags_json, seen, message_id, in_reply_to, references_json) = r?;
                let flags: Vec<String> = serde_json::from_str(&flags_json).unwrap_or_default();
                let references: Vec<String> = references_json
                    .as_deref()
                    .map(|value| serde_json::from_str(value).unwrap_or_default())
                    .unwrap_or_default();
                out.push(Envelope {
                    uid,
                    subject,
                    from,
                    date,
                    flags,
                    seen,
                    message_id,
                    in_reply_to,
                    references,
                });
            }
            Ok(out)
        })
        .await?
    }

    pub async fn delete_messages(
        &self,
        account_key: String,
        mailbox: String,
        uids: Vec<u32>,
    ) -> Result<()> {
        if uids.is_empty() {
            return Ok(());
        }
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let tx = guard.transaction()?;
            {
                let mut stmt = tx.prepare(
                    "DELETE FROM messages
                     WHERE account_key = ?1 AND mailbox = ?2 AND uid = ?3",
                )?;
                for uid in &uids {
                    stmt.execute(params![account_key, mailbox, *uid as i64])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?
    }

    pub async fn put_body(
        &self,
        account_key: String,
        mailbox: String,
        uid: u32,
        body: MessageBody,
    ) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0) as i64;
            let attachments_json = serde_json::to_string(&body.attachments)?;
            guard.execute(
                "UPDATE messages SET
                   body_subject = ?4, body_from = ?5, body_to = ?6, body_date = ?7,
                   body_text = ?8, body_html = ?9, has_body = 1, body_fetched_at = ?10,
                   body_attachments = ?11, message_id = ?12, in_reply_to = ?13,
                   message_references = ?14
                 WHERE account_key = ?1 AND mailbox = ?2 AND uid = ?3",
                params![
                    account_key,
                    mailbox,
                    uid,
                    body.subject,
                    body.from,
                    body.to,
                    body.date,
                    body.text,
                    body.html,
                    now,
                    attachments_json,
                    body.message_id,
                    body.in_reply_to,
                    serde_json::to_string(&body.references)?,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn get_body(
        &self,
        account_key: String,
        mailbox: String,
        uid: u32,
    ) -> Result<Option<MessageBody>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<Option<MessageBody>> {
            let guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            let mut stmt = guard.prepare(
                "SELECT body_subject, body_from, body_to, body_date, body_text, body_html,
                        body_attachments, message_id, in_reply_to, message_references
                 FROM messages
                 WHERE account_key = ?1 AND mailbox = ?2 AND uid = ?3 AND has_body = 1",
            )?;
            let result = stmt
                .query_row(params![account_key, mailbox, uid], |row| {
                    let attachments_json: Option<String> = row.get(6)?;
                    let attachments: Vec<AttachmentMeta> = attachments_json
                        .as_deref()
                        .map(|s| serde_json::from_str(s).unwrap_or_default())
                        .unwrap_or_default();
                    let references_json: Option<String> = row.get(9)?;
                    let references = references_json
                        .as_deref()
                        .map(|value| serde_json::from_str(value).unwrap_or_default())
                        .unwrap_or_default();
                    Ok(MessageBody {
                        subject: row.get(0)?,
                        from: row.get(1)?,
                        to: row.get(2)?,
                        date: row.get(3)?,
                        text: row.get(4)?,
                        html: row.get(5)?,
                        attachments,
                        message_id: row.get(7)?,
                        in_reply_to: row.get(8)?,
                        references,
                    })
                })
                .optional()?;
            Ok(result)
        })
        .await?
    }

    pub async fn set_seen(
        &self,
        account_key: String,
        mailbox: String,
        uid: u32,
        seen: bool,
        flags: Vec<String>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let guard = conn.lock().map_err(|_| anyhow!("cache mutex poisoned"))?;
            guard.execute(
                "UPDATE messages SET seen = ?4, flags = ?5
                 WHERE account_key = ?1 AND mailbox = ?2 AND uid = ?3",
                params![
                    account_key,
                    mailbox,
                    uid,
                    seen as i64,
                    serde_json::to_string(&flags)?,
                ],
            )?;
            Ok(())
        })
        .await?
    }
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS folders (
            account_key TEXT NOT NULL,
            raw TEXT NOT NULL,
            leaf TEXT NOT NULL,
            depth INTEGER NOT NULL,
            parent_path TEXT,
            has_children INTEGER NOT NULL,
            special TEXT NOT NULL,
            ord INTEGER NOT NULL,
            subscribed INTEGER NOT NULL DEFAULT 1,
            unread_count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (account_key, raw)
         );

         CREATE TABLE IF NOT EXISTS messages (
            account_key TEXT NOT NULL,
            mailbox TEXT NOT NULL,
            uid INTEGER NOT NULL,
            subject TEXT NOT NULL,
            from_addr TEXT NOT NULL,
            date TEXT NOT NULL,
            flags TEXT NOT NULL,
            seen INTEGER NOT NULL,
            has_body INTEGER NOT NULL DEFAULT 0,
            body_subject TEXT,
            body_from TEXT,
            body_to TEXT,
            body_date TEXT,
            body_text TEXT,
            body_html TEXT,
            body_fetched_at INTEGER,
            body_attachments TEXT,
            message_id TEXT,
            in_reply_to TEXT,
            message_references TEXT,
            PRIMARY KEY (account_key, mailbox, uid)
         );

         CREATE INDEX IF NOT EXISTS idx_msg_lookup
            ON messages(account_key, mailbox, uid DESC);
         CREATE INDEX IF NOT EXISTS idx_msg_lru
            ON messages(has_body, body_fetched_at);
        ",
    )?;
    // Migrations for existing DBs (silently ignore "duplicate column" errors).
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN body_attachments TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE folders ADD COLUMN subscribed INTEGER NOT NULL DEFAULT 1",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE folders ADD COLUMN unread_count INTEGER NOT NULL DEFAULT 0",
        [],
    );
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN message_id TEXT", []);
    let _ = conn.execute("ALTER TABLE messages ADD COLUMN in_reply_to TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE messages ADD COLUMN message_references TEXT",
        [],
    );
    Ok(())
}
