use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use async_imap::types::{Flag, NameAttribute};
use async_imap::Session;
use futures::TryStreamExt;
use mail_parser::{MessageParser, MimeHeaders, PartType};
use tokio::net::TcpStream;
use tokio_rustls::{
    client::TlsStream,
    rustls::{pki_types::ServerName, ClientConfig, RootCertStore},
    TlsConnector,
};

use super::message::{AttachmentMeta, Envelope, Folder, MessageBody, SpecialUse};

pub type ImapStream = TlsStream<TcpStream>;
pub type ImapSession = Session<ImapStream>;

fn flag_to_string(flag: &Flag<'_>) -> String {
    match flag {
        Flag::Seen => "\\Seen".to_string(),
        Flag::Answered => "\\Answered".to_string(),
        Flag::Flagged => "\\Flagged".to_string(),
        Flag::Deleted => "\\Deleted".to_string(),
        Flag::Draft => "\\Draft".to_string(),
        Flag::Recent => "\\Recent".to_string(),
        Flag::MayCreate => "\\*".to_string(),
        Flag::Custom(s) => s.to_string(),
    }
}

fn tls_connector() -> Result<TlsConnector> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

pub async fn connect(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
) -> Result<ImapSession> {
    crate::debug_log::push("imap", "→", format!("CONNECT {}:{}", host, port));
    let connector = tls_connector()?;
    let tcp = TcpStream::connect((host, port))
        .await
        .with_context(|| format!("TCP connect failed: {host}:{port}"))?;
    let dns = ServerName::try_from(host.to_string())
        .map_err(|e| anyhow!("invalid DNS name {host}: {e}"))?;
    let tls = connector
        .connect(dns, tcp)
        .await
        .context("TLS handshake failed")?;
    crate::debug_log::push("imap", "←", "TLS OK");

    let client = async_imap::Client::new(tls);
    crate::debug_log::push("imap", "→", format!("LOGIN {} ***", username));
    let mut session = client
        .login(username, password)
        .await
        .map_err(|(e, _)| {
            crate::debug_log::push("imap", "←", format!("LOGIN failed: {}", e));
            anyhow!("IMAP login failed: {e}")
        })?;
    crate::debug_log::push("imap", "←", "LOGIN OK");
    // RFC 2971 ID — advertise our client to the server. Best-effort; servers
    // that don't support the ID extension reject this with NO/BAD which we
    // silently ignore.
    let _ = session
        .id([
            ("name", Some("Mantybird")),
            ("version", Some(env!("CARGO_PKG_VERSION"))),
            ("vendor", Some("manty")),
            ("os", Some(std::env::consts::OS)),
        ])
        .await;
    Ok(session)
}

fn classify_special(raw: &str, attrs: &[NameAttribute<'_>]) -> SpecialUse {
    if raw.eq_ignore_ascii_case("INBOX") {
        return SpecialUse::Inbox;
    }
    for a in attrs {
        match a {
            NameAttribute::Sent => return SpecialUse::Sent,
            NameAttribute::Drafts => return SpecialUse::Drafts,
            NameAttribute::Archive => return SpecialUse::Archive,
            NameAttribute::Junk => return SpecialUse::Junk,
            NameAttribute::Trash => return SpecialUse::Trash,
            _ => {}
        }
    }
    SpecialUse::Other
}

/// RFC 3501: `LIST "" ""` returns one untagged LIST response whose
/// delimiter is the server's hierarchy separator. Fallback to "/".
pub async fn discover_delimiter(session: &mut ImapSession) -> Result<String> {
    let stream = session.list(Some(""), Some("")).await?;
    let entries: Vec<_> = stream.try_collect().await?;
    Ok(entries
        .first()
        .and_then(|n| n.delimiter().map(str::to_string))
        .unwrap_or_else(|| "/".to_string()))
}

pub async fn list_folders(session: &mut ImapSession) -> Result<Vec<Folder>> {
    crate::debug_log::push("imap", "→", "LIST \"\" \"*\"");
    let stream = session.list(Some(""), Some("*")).await?;
    let entries: Vec<_> = stream.try_collect().await?;
    crate::debug_log::push(
        "imap",
        "←",
        format!("LIST OK — {} entries", entries.len()),
    );
    // Emit specials on their own line so the trash / junk wire-format name
    // is easy to spot when debugging MOVE / SELECT failures.
    for n in &entries {
        let attrs = n.attributes();
        let name_is_inbox = n.name().eq_ignore_ascii_case("INBOX");
        let attr_is_special = attrs.iter().any(|a| {
            matches!(
                a,
                NameAttribute::Sent
                    | NameAttribute::Drafts
                    | NameAttribute::Archive
                    | NameAttribute::Junk
                    | NameAttribute::Trash
            )
        });
        if name_is_inbox || attr_is_special {
            crate::debug_log::push(
                "imap",
                "←",
                format!("  · {:?} attrs={:?}", n.name(), attrs),
            );
        }
    }

    // LSUB to discover which mailboxes the user has subscribed to.
    // Some servers (Gmail) treat all mailboxes as subscribed; failures are
    // not fatal — we just fall back to "everything subscribed".
    let subscribed_set: std::collections::HashSet<String> = match session
        .lsub(Some(""), Some("*"))
        .await
    {
        Ok(s) => match s.try_collect::<Vec<_>>().await {
            Ok(rows) => rows.into_iter().map(|n| n.name().to_string()).collect(),
            Err(_) => std::collections::HashSet::new(),
        },
        Err(_) => std::collections::HashSet::new(),
    };
    let assume_all = subscribed_set.is_empty();

    let folders: Vec<(Folder, bool)> = entries
        .into_iter()
        .map(|n| {
            let raw = n.name().to_string();
            let delimiter = n.delimiter().unwrap_or("/").to_string();
            let display = utf7_imap::decode_utf7_imap(raw.clone());
            let (leaf, depth, parent_path) = if delimiter.is_empty() {
                (display.clone(), 0u16, None)
            } else {
                let segments: Vec<&str> = display.split(&delimiter).collect();
                let depth = segments.len().saturating_sub(1) as u16;
                let leaf = segments.last().copied().unwrap_or("").to_string();
                let parent_path = if depth == 0 {
                    None
                } else {
                    let raw_segs: Vec<&str> = raw.split(&delimiter).collect();
                    Some(raw_segs[..raw_segs.len() - 1].join(&delimiter))
                };
                (leaf, depth, parent_path)
            };
            let special = classify_special(&raw, n.attributes());
            let subscribed = assume_all || subscribed_set.contains(&raw);
            let no_select = n
                .attributes()
                .iter()
                .any(|a| matches!(a, NameAttribute::NoSelect));
            let no_inferiors = n
                .attributes()
                .iter()
                .any(|a| matches!(a, NameAttribute::NoInferiors));
            (
                Folder {
                    raw,
                    leaf,
                    depth,
                    parent_path,
                    has_children: false,
                    no_select,
                    no_inferiors,
                    special,
                    subscribed,
                    unread_count: 0,
                },
                no_select,
            )
        })
        .collect::<Vec<_>>();

    let no_select_set: std::collections::HashSet<String> = folders
        .iter()
        .filter(|(_, ns)| *ns)
        .map(|(f, _)| f.raw.clone())
        .collect();
    let folders: Vec<Folder> = folders.into_iter().map(|(f, _)| f).collect();
    let mut sorted = sort_folder_tree(folders);
    // Populate unread counts via STATUS for selectable folders.
    for folder in sorted.iter_mut() {
        if no_select_set.contains(&folder.raw) {
            continue;
        }
        if let Ok(mbox) = session.status(&folder.raw, "(UNSEEN)").await {
            folder.unread_count = mbox.unseen.unwrap_or(0);
        }
    }
    Ok(sorted)
}

pub async fn unread_count(session: &mut ImapSession, mailbox: &str) -> Result<u32> {
    crate::debug_log::push("imap", "→", format!("STATUS {:?} (UNSEEN)", mailbox));
    let mbox = session
        .status(mailbox, "(UNSEEN)")
        .await
        .context("STATUS UNSEEN failed")?;
    let count = mbox.unseen.unwrap_or(0);
    crate::debug_log::push("imap", "←", format!("STATUS OK — unseen={}", count));
    Ok(count)
}

fn sort_folder_tree(mut folders: Vec<Folder>) -> Vec<Folder> {
    let n = folders.len();
    let path_to_idx: HashMap<String, usize> = folders
        .iter()
        .enumerate()
        .map(|(i, f)| (f.raw.clone(), i))
        .collect();

    let mut children_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    for (i, f) in folders.iter().enumerate() {
        match f.parent_path.as_ref().and_then(|p| path_to_idx.get(p)) {
            Some(&pi) => children_of[pi].push(i),
            None => roots.push(i),
        }
    }

    for (i, cs) in children_of.iter().enumerate() {
        folders[i].has_children = !cs.is_empty();
    }

    let mut min_pri: Vec<u8> = vec![u8::MAX; n];
    fn compute_min(
        i: usize,
        children_of: &[Vec<usize>],
        folders: &[Folder],
        min_pri: &mut [u8],
    ) -> u8 {
        let mut p = folders[i].special.priority();
        let cs = children_of[i].clone();
        for c in cs {
            p = p.min(compute_min(c, children_of, folders, min_pri));
        }
        min_pri[i] = p;
        p
    }
    for &r in &roots {
        compute_min(r, &children_of, &folders, &mut min_pri);
    }

    let sort_key = |i: usize, folders: &[Folder], min_pri: &[u8]| -> (u8, String) {
        (min_pri[i], folders[i].leaf.to_lowercase())
    };
    for cs in children_of.iter_mut() {
        cs.sort_by_key(|&i| sort_key(i, &folders, &min_pri));
    }
    roots.sort_by_key(|&i| sort_key(i, &folders, &min_pri));

    let mut out: Vec<Folder> = Vec::with_capacity(n);
    fn dfs(
        i: usize,
        children_of: &[Vec<usize>],
        folders: &[Folder],
        out: &mut Vec<Folder>,
    ) {
        out.push(folders[i].clone());
        for &c in &children_of[i] {
            dfs(c, children_of, folders, out);
        }
    }
    for &r in &roots {
        dfs(r, &children_of, &folders, &mut out);
    }
    out
}

fn parse_envelopes(fetched: Vec<async_imap::types::Fetch>) -> Vec<Envelope> {
    let mut envelopes = Vec::with_capacity(fetched.len());
    for f in fetched {
        let Some(uid) = f.uid else { continue };
        let flags: Vec<String> = f.flags().map(|fl| flag_to_string(&fl)).collect();
        let seen = flags.iter().any(|s| s.eq_ignore_ascii_case("\\Seen"));
        let header_bytes = f.header().unwrap_or(&[]);
        let internal_date_str = f.internal_date().map(|d| d.to_rfc3339());
        let summary = parse_header_summary(header_bytes, internal_date_str);
        envelopes.push(Envelope {
            uid,
            flags,
            seen,
            subject: summary.subject,
            from: summary.from,
            date: summary.date,
            message_id: summary.message_id,
            in_reply_to: summary.in_reply_to,
            references: summary.references,
        });
    }
    envelopes
}

/// Returns the subset of `uids` that the server still has in `mailbox`.
/// Uses a single `UID FETCH <set> (UID)` so the wire payload stays bounded
/// (one tiny line per surviving UID) regardless of mailbox size.
pub async fn check_uids_alive(
    session: &mut ImapSession,
    mailbox: &str,
    uids: &[u32],
) -> Result<Vec<u32>> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    let mb = session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push(
        "imap",
        "←",
        format!("SELECT OK exists={}", mb.exists),
    );
    if uids.is_empty() || mb.exists == 0 {
        return Ok(Vec::new());
    }
    let uid_set = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    crate::debug_log::push(
        "imap",
        "→",
        format!("UID FETCH <{} uids> (UID)", uids.len()),
    );
    let stream = session
        .uid_fetch(uid_set, "UID")
        .await
        .context("UID FETCH (UID) failed")?;
    let entries: Vec<_> = stream
        .try_collect::<Vec<_>>()
        .await
        .context("UID FETCH (UID) collect failed")?;
    let alive: Vec<u32> = entries.iter().filter_map(|f| f.uid).collect();
    crate::debug_log::push(
        "imap",
        "←",
        format!("UID FETCH (UID) → {} alive", alive.len()),
    );
    Ok(alive)
}

pub async fn search_uids(
    session: &mut ImapSession,
    mailbox: &str,
    query: &str,
) -> Result<Vec<u32>> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    let escaped = query.replace('\\', "\\\\").replace('"', "\\\"");
    let cmd = format!("CHARSET UTF-8 TEXT \"{}\"", escaped);
    crate::debug_log::push("imap", "→", format!("UID SEARCH {}", cmd));
    let uids = session
        .uid_search(cmd)
        .await
        .context("UID SEARCH failed")?;
    let mut v: Vec<u32> = uids.into_iter().collect();
    v.sort_by(|a, b| b.cmp(a));
    crate::debug_log::push(
        "imap",
        "←",
        format!("UID SEARCH OK — {} matches", v.len()),
    );
    Ok(v)
}

pub async fn fetch_envelopes_by_uids(
    session: &mut ImapSession,
    mailbox: &str,
    uids: &[u32],
    limit: usize,
) -> Result<Vec<Envelope>> {
    session.select(mailbox).await.context("SELECT failed")?;
    if uids.is_empty() {
        return Ok(vec![]);
    }
    let take = uids.iter().take(limit).copied().collect::<Vec<_>>();
    let uid_set = take
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let stream = session
        .uid_fetch(
            uid_set,
            "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE MESSAGE-ID IN-REPLY-TO REFERENCES)])",
        )
        .await
        .context("UID FETCH (search) failed")?;
    let fetched: Vec<_> = stream.try_collect().await?;
    let mut envelopes = parse_envelopes(fetched);
    envelopes.sort_by_key(|e| std::cmp::Reverse(e.uid));
    Ok(envelopes)
}

pub async fn fetch_envelopes(
    session: &mut ImapSession,
    mailbox: &str,
    offset: u32,
    limit: u32,
) -> Result<Vec<Envelope>> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    let info = session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push(
        "imap",
        "←",
        format!("SELECT OK exists={}", info.exists),
    );
    let total = info.exists;
    if total == 0 || offset >= total {
        return Ok(vec![]);
    }
    let end = total - offset;
    let start = end.saturating_sub(limit.saturating_sub(1)).max(1);
    let range = format!("{start}:{end}");
    crate::debug_log::push(
        "imap",
        "→",
        format!("FETCH {} (UID FLAGS INTERNALDATE BODY.PEEK[HEADER...])", range),
    );
    // ENVELOPE intentionally NOT requested: async-imap/imap-proto fails on
    // 8-bit chars inside quoted strings (e.g. unencoded Korean names).
    // HEADER.FIELDS minimizes payload (~10x smaller than full HEADER) while
    // imap-proto still maps the response to MessageSection::Header, so
    // f.header() works.
    let stream = session
        .fetch(
            range,
            "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE MESSAGE-ID IN-REPLY-TO REFERENCES)])",
        )
        .await
        .context("FETCH headers failed")?;
    let fetched: Vec<_> = stream.try_collect().await?;

    let mut envelopes = Vec::with_capacity(fetched.len());
    for f in fetched {
        let Some(uid) = f.uid else { continue };
        let flags: Vec<String> = f.flags().map(|fl| flag_to_string(&fl)).collect();
        let seen = flags.iter().any(|s| s.eq_ignore_ascii_case("\\Seen"));
        let header_bytes = f.header().unwrap_or(&[]);
        let internal_date_str = f.internal_date().map(|d| d.to_rfc3339());
        let summary = parse_header_summary(header_bytes, internal_date_str);
        envelopes.push(Envelope {
            uid,
            flags,
            seen,
            subject: summary.subject,
            from: summary.from,
            date: summary.date,
            message_id: summary.message_id,
            in_reply_to: summary.in_reply_to,
            references: summary.references,
        });
    }
    envelopes.sort_by_key(|e| std::cmp::Reverse(e.uid));
    crate::debug_log::push(
        "imap",
        "←",
        format!("FETCH OK — {} envelopes", envelopes.len()),
    );
    Ok(envelopes)
}

struct HeaderSummary {
    subject: String,
    from: String,
    date: String,
    message_id: Option<String>,
    in_reply_to: Option<String>,
    references: Vec<String>,
}

fn header_ids(value: &mail_parser::HeaderValue<'_>) -> Vec<String> {
    value
        .as_text_list()
        .unwrap_or_default()
        .into_iter()
        .flat_map(|value| {
            value
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_header_summary(
    raw_header: &[u8],
    internal_date: Option<String>,
) -> HeaderSummary {
    // Append the CRLF CRLF that ends the header section if not present,
    // so mail-parser treats input as a complete (header-only) message.
    let mut blob = raw_header.to_vec();
    if !blob.ends_with(b"\r\n\r\n") {
        if blob.ends_with(b"\r\n") {
            blob.extend_from_slice(b"\r\n");
        } else {
            blob.extend_from_slice(b"\r\n\r\n");
        }
    }
    let parsed = MessageParser::default().parse(&blob);
    let subject = parsed
        .as_ref()
        .and_then(|m| m.subject())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(no subject)".to_string());
    let from = parsed
        .as_ref()
        .and_then(|m| m.from())
        .and_then(|a| a.first())
        .map(|a| match (a.name(), a.address()) {
            (Some(n), Some(e)) if !n.is_empty() && !e.is_empty() => format!("{n} <{e}>"),
            (Some(n), _) if !n.is_empty() => n.to_string(),
            (_, Some(e)) => e.to_string(),
            _ => String::new(),
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(unknown)".to_string());
    let date = parsed
        .as_ref()
        .and_then(|m| m.date())
        .map(|d| d.to_rfc3339())
        .or(internal_date)
        .unwrap_or_default();
    let message_id = parsed
        .as_ref()
        .and_then(|m| m.message_id())
        .map(str::to_string);
    let in_reply_to = parsed
        .as_ref()
        .and_then(|m| header_ids(m.in_reply_to()).into_iter().last());
    let references = parsed
        .as_ref()
        .map(|m| header_ids(m.references()))
        .unwrap_or_default();
    HeaderSummary {
        subject,
        from,
        date,
        message_id,
        in_reply_to,
        references,
    }
}

#[cfg(test)]
mod tests {
    use super::parse_header_summary;

    #[test]
    fn parses_thread_headers() {
        let raw = concat!(
            "Subject: Re: status\r\n",
            "From: Alice <alice@example.com>\r\n",
            "Message-ID: <child@example.com>\r\n",
            "In-Reply-To: <parent@example.com>\r\n",
            "References: <root@example.com> <parent@example.com>\r\n",
            "\r\n",
        );

        let summary = parse_header_summary(raw.as_bytes(), None);

        assert_eq!(summary.message_id.as_deref(), Some("child@example.com"));
        assert_eq!(summary.in_reply_to.as_deref(), Some("parent@example.com"));
        assert_eq!(
            summary.references,
            vec!["root@example.com", "parent@example.com"]
        );
    }
}

pub async fn fetch_body(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
) -> Result<MessageBody> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    crate::debug_log::push("imap", "→", format!("UID FETCH {} RFC822", uid));
    let stream = session
        .uid_fetch(uid.to_string(), "RFC822")
        .await
        .context("UID FETCH failed")?;
    let fetched: Vec<_> = stream.try_collect().await?;
    let raw = fetched
        .first()
        .and_then(|f| f.body())
        .ok_or_else(|| {
            crate::debug_log::push(
                "imap",
                "←",
                format!("UID FETCH {} → empty body", uid),
            );
            anyhow!("empty body for uid {uid}")
        })?;
    crate::debug_log::push(
        "imap",
        "←",
        format!("UID FETCH OK — {} bytes", raw.len()),
    );

    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| anyhow!("failed to parse message"))?;

    let subject = parsed.subject().unwrap_or("").to_string();
    let from = parsed
        .from()
        .and_then(|a| a.first())
        .and_then(|a| a.name().or(a.address()))
        .unwrap_or("")
        .to_string();
    let to = parsed
        .to()
        .map(|list| {
            list.iter()
                .filter_map(|a| a.address().or(a.name()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let date = parsed.date().map(|d| d.to_rfc3339()).unwrap_or_default();
    let message_id = parsed.message_id().map(str::to_string);
    let in_reply_to = header_ids(parsed.in_reply_to()).into_iter().last();
    let references = header_ids(parsed.references());

    let raw_html = parsed.body_html(0).map(|h| h.into_owned());
    let html = raw_html.map(|h| inline_cid_images(&h, &parsed));
    let text = parsed
        .body_text(0)
        .map(|t| t.into_owned())
        .unwrap_or_default();

    let attachments: Vec<AttachmentMeta> = parsed
        .attachments()
        .enumerate()
        .filter_map(|(index, part)| {
            let content_id = part.content_id().map(str::to_string);
            let is_inline = part
                .content_disposition()
                .map(|d| d.ctype().eq_ignore_ascii_case("inline"))
                .unwrap_or(false);
            // Hide parts that are referenced inline from the HTML body
            // (typically images with Content-ID embedded as data URLs).
            if content_id.is_some() || is_inline {
                return None;
            }
            let (filename, size, mime) = describe_part(part);
            Some(AttachmentMeta {
                index,
                filename,
                mime,
                size,
                content_id,
                is_inline,
            })
        })
        .collect();

    Ok(MessageBody {
        subject,
        from,
        to,
        date,
        text,
        html,
        attachments,
        message_id,
        in_reply_to,
        references,
    })
}

fn describe_part(part: &mail_parser::MessagePart<'_>) -> (String, u64, String) {
    let filename = part
        .attachment_name()
        .map(str::to_string)
        .or_else(|| part.content_id().map(str::to_string))
        .unwrap_or_else(|| "attachment".to_string());
    let mime = part
        .content_type()
        .map(|ct| {
            let t = ct.ctype().to_string();
            match ct.subtype() {
                Some(s) => format!("{t}/{s}"),
                None => t,
            }
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());
    let size = match &part.body {
        PartType::Binary(b) | PartType::InlineBinary(b) => b.len() as u64,
        PartType::Text(s) | PartType::Html(s) => s.len() as u64,
        PartType::Message(m) => m.raw_message().len() as u64,
        PartType::Multipart(_) => 0,
    };
    (filename, size, mime)
}

/// Replace `cid:` references in `html` with embedded `data:` URLs built
/// from the message's inline parts (typically images shown inside the
/// HTML body). Falls back to leaving the reference untouched if the
/// corresponding part isn't present.
fn inline_cid_images(html: &str, message: &mail_parser::Message<'_>) -> String {
    use base64::Engine;
    use std::collections::HashMap;

    let mut cid_to_data_url: HashMap<String, String> = HashMap::new();
    for part in message.attachments() {
        let cid = match part.content_id() {
            Some(c) => c.to_string(),
            None => continue,
        };
        let bytes = match &part.body {
            PartType::Binary(b) | PartType::InlineBinary(b) => b.as_ref(),
            _ => continue,
        };
        let mime = part
            .content_type()
            .map(|ct| {
                let t = ct.ctype().to_string();
                match ct.subtype() {
                    Some(s) => format!("{t}/{s}"),
                    None => t,
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        cid_to_data_url.insert(cid, format!("data:{mime};base64,{encoded}"));
    }

    if cid_to_data_url.is_empty() {
        return html.to_string();
    }

    let mut out = html.to_string();
    for (cid, data_url) in &cid_to_data_url {
        // Common patterns: cid:value, "cid:value", cid:<value>
        let patterns = [
            format!("cid:{cid}"),
            format!("cid:<{cid}>"),
        ];
        for pat in &patterns {
            out = out.replace(pat, data_url);
        }
    }
    out
}

pub async fn fetch_attachment_bytes(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
    index: usize,
) -> Result<(String, String, Vec<u8>)> {
    session.select(mailbox).await.context("SELECT failed")?;
    let stream = session
        .uid_fetch(uid.to_string(), "RFC822")
        .await
        .context("UID FETCH (attachment) failed")?;
    let fetched: Vec<_> = stream.try_collect().await?;
    let raw = fetched
        .first()
        .and_then(|f| f.body())
        .ok_or_else(|| anyhow!("empty body for uid {uid}"))?;

    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| anyhow!("failed to parse message"))?;
    let part = parsed
        .attachment(index)
        .ok_or_else(|| anyhow!("attachment {index} not found"))?;
    let (filename, _size, mime) = describe_part(part);
    let bytes = match &part.body {
        PartType::Binary(b) | PartType::InlineBinary(b) => b.as_ref().to_vec(),
        PartType::Text(s) | PartType::Html(s) => s.as_bytes().to_vec(),
        PartType::Message(m) => m.raw_message().to_vec(),
        PartType::Multipart(_) => return Err(anyhow!("multipart cannot be downloaded directly")),
    };
    Ok((filename, mime, bytes))
}

pub async fn create_mailbox(session: &mut ImapSession, name: &str) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("CREATE {:?}", name));
    let r = session.create(name).await.context("CREATE failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "CREATE OK".to_string(),
            Err(e) => format!("CREATE failed: {}", e),
        },
    );
    r
}

pub async fn delete_mailbox(session: &mut ImapSession, name: &str) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("DELETE {:?}", name));
    let r = session.delete(name).await.context("DELETE failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "DELETE OK".to_string(),
            Err(e) => format!("DELETE failed: {}", e),
        },
    );
    r
}

pub async fn rename_mailbox(
    session: &mut ImapSession,
    from: &str,
    to: &str,
) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("RENAME {:?} {:?}", from, to));
    let r = session.rename(from, to).await.context("RENAME failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "RENAME OK".to_string(),
            Err(e) => format!("RENAME failed: {}", e),
        },
    );
    r
}

pub async fn subscribe_mailbox(session: &mut ImapSession, name: &str) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("SUBSCRIBE {:?}", name));
    let r = session.subscribe(name).await.context("SUBSCRIBE failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "SUBSCRIBE OK".to_string(),
            Err(e) => format!("SUBSCRIBE failed: {}", e),
        },
    );
    r
}

pub async fn unsubscribe_mailbox(session: &mut ImapSession, name: &str) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("UNSUBSCRIBE {:?}", name));
    let r = session
        .unsubscribe(name)
        .await
        .context("UNSUBSCRIBE failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "UNSUBSCRIBE OK".to_string(),
            Err(e) => format!("UNSUBSCRIBE failed: {}", e),
        },
    );
    r
}

pub async fn append_message(
    session: &mut ImapSession,
    mailbox: &str,
    raw: &[u8],
    flags: Option<&str>,
) -> Result<()> {
    crate::debug_log::push(
        "imap",
        "→",
        format!("APPEND {:?} ({} bytes) flags={:?}", mailbox, raw.len(), flags),
    );
    let r = session
        .append(mailbox, flags, None, raw)
        .await
        .context("APPEND failed");
    crate::debug_log::push(
        "imap",
        "←",
        match &r {
            Ok(_) => "APPEND OK".to_string(),
            Err(e) => format!("APPEND failed: {}", e),
        },
    );
    r
}

pub async fn set_flag(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
    flag: &str,
    on: bool,
) -> Result<Vec<String>> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    let op = if on { "+FLAGS" } else { "-FLAGS" };
    let store_arg = format!("{op} ({flag})");
    crate::debug_log::push(
        "imap",
        "→",
        format!("UID STORE {} {}", uid, store_arg),
    );
    let stream = session
        .uid_store(uid.to_string(), &store_arg)
        .await
        .context("STORE failed")?;
    let results: Vec<_> = stream.try_collect().await?;
    let mut flags = Vec::new();
    for f in results {
        for fl in f.flags() {
            flags.push(flag_to_string(&fl));
        }
    }
    crate::debug_log::push(
        "imap",
        "←",
        format!("UID STORE OK — flags=[{}]", flags.join(", ")),
    );
    Ok(flags)
}

pub async fn delete_permanent(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
) -> Result<()> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    crate::debug_log::push(
        "imap",
        "→",
        format!("UID STORE {} +FLAGS (\\Deleted)", uid),
    );
    let stream = session
        .uid_store(uid.to_string(), "+FLAGS (\\Deleted)")
        .await
        .context("STORE \\Deleted failed")?;
    let _ = stream.try_collect::<Vec<_>>().await?;
    crate::debug_log::push("imap", "←", "UID STORE OK");
    crate::debug_log::push("imap", "→", "EXPUNGE");
    let exp = session.expunge().await.context("EXPUNGE failed")?;
    let _ = exp.try_collect::<Vec<_>>().await?;
    crate::debug_log::push("imap", "←", "EXPUNGE OK");
    Ok(())
}

pub async fn move_message(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
    dest: &str,
) -> Result<()> {
    crate::debug_log::push(
        "imap",
        "→",
        format!("SELECT {:?}", mailbox),
    );
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    // Try RFC 6851 MOVE first; fall back to COPY + \Deleted + EXPUNGE.
    crate::debug_log::push(
        "imap",
        "→",
        format!("UID MOVE {} {:?}", uid, dest),
    );
    match session.uid_mv(uid.to_string(), dest).await {
        Ok(()) => {
            crate::debug_log::push("imap", "←", "UID MOVE OK");
            Ok(())
        }
        Err(e) => {
            crate::debug_log::push(
                "imap",
                "←",
                format!("UID MOVE failed: {} — falling back to COPY+STORE+EXPUNGE", e),
            );
            crate::debug_log::push(
                "imap",
                "→",
                format!("UID COPY {} {:?}", uid, dest),
            );
            session
                .uid_copy(uid.to_string(), dest)
                .await
                .context("COPY (fallback for MOVE) failed")?;
            crate::debug_log::push("imap", "←", "UID COPY OK");
            let store_arg = "+FLAGS (\\Deleted)";
            crate::debug_log::push(
                "imap",
                "→",
                format!("UID STORE {} {}", uid, store_arg),
            );
            let stream = session
                .uid_store(uid.to_string(), store_arg)
                .await
                .context("STORE \\Deleted failed")?;
            let _ = stream.try_collect::<Vec<_>>().await?;
            crate::debug_log::push("imap", "←", "UID STORE OK");
            crate::debug_log::push("imap", "→", "EXPUNGE");
            let exp = session.expunge().await.context("EXPUNGE failed")?;
            let _ = exp.try_collect::<Vec<_>>().await?;
            crate::debug_log::push("imap", "←", "EXPUNGE OK");
            Ok(())
        }
    }
}

pub async fn mark_seen(
    session: &mut ImapSession,
    mailbox: &str,
    uid: u32,
    seen: bool,
) -> Result<Vec<String>> {
    crate::debug_log::push("imap", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await.context("SELECT failed")?;
    crate::debug_log::push("imap", "←", "SELECT OK");
    let store_arg = if seen { "+FLAGS (\\Seen)" } else { "-FLAGS (\\Seen)" };
    crate::debug_log::push(
        "imap",
        "→",
        format!("UID STORE {} {}", uid, store_arg),
    );
    let stream = session
        .uid_store(uid.to_string(), store_arg)
        .await
        .context("STORE failed")?;
    let results: Vec<_> = stream.try_collect().await?;
    let mut flags = Vec::new();
    for f in results {
        for fl in f.flags() {
            flags.push(flag_to_string(&fl));
        }
    }
    crate::debug_log::push(
        "imap",
        "←",
        format!("UID STORE OK — flags=[{}]", flags.join(", ")),
    );
    Ok(flags)
}
