use anyhow::{anyhow, Context, Result};
use lettre::message::{header::ContentType, Attachment, Mailbox, Mailboxes, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

use crate::account::Account;

pub struct OutgoingAttachment {
    pub filename: String,
    pub mime: String,
    pub data: Vec<u8>,
}

pub struct OutgoingMail {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    pub body: String,
    pub html: Option<String>,
    pub attachments: Vec<OutgoingAttachment>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}

pub fn parse_addresses(raw: &str) -> Result<Vec<Mailbox>> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mailboxes: Mailboxes = raw
        .parse()
        .with_context(|| format!("invalid address list: {raw}"))?;
    Ok(mailboxes.into_iter().collect())
}

pub fn build_from(account: &Account) -> Result<Mailbox> {
    let address = if account.username.contains('@') {
        account.username.clone()
    } else {
        format!("{}@{}", account.username, account.host)
    };
    let display = if account.name.is_empty() {
        None
    } else {
        Some(account.name.clone())
    };
    let mailbox = match display {
        Some(d) => format!("{d} <{address}>"),
        None => address,
    };
    mailbox
        .parse::<Mailbox>()
        .with_context(|| format!("invalid From address: {mailbox}"))
}

fn format_message_id(value: &str) -> Option<String> {
    let value = value.trim().trim_start_matches('<').trim_end_matches('>');
    (!value.is_empty()).then(|| format!("<{value}>"))
}

fn build_message(mail: OutgoingMail) -> Result<Message> {
    let mut builder = Message::builder()
        .from(mail.from)
        .subject(mail.subject);
    for t in mail.to {
        builder = builder.to(t);
    }
    for c in mail.cc {
        builder = builder.cc(c);
    }
    for b in mail.bcc {
        builder = builder.bcc(b);
    }
    if let Some(in_reply_to) = mail
        .in_reply_to
        .as_deref()
        .and_then(format_message_id)
    {
        builder = builder.in_reply_to(in_reply_to);
    }
    let references = mail
        .references
        .iter()
        .filter_map(|value| format_message_id(value))
        .collect::<Vec<_>>();
    if !references.is_empty() {
        builder = builder.references(references.join(" "));
    }

    let plain = SinglePart::builder()
        .header(ContentType::TEXT_PLAIN)
        .body(mail.body.clone());

    let body_part: MultiPart = if let Some(html) = mail.html {
        let html_part = SinglePart::builder()
            .header(ContentType::TEXT_HTML)
            .body(html);
        MultiPart::alternative().singlepart(plain).singlepart(html_part)
    } else {
        MultiPart::alternative().singlepart(plain)
    };

    let msg = if mail.attachments.is_empty() {
        builder.multipart(body_part)?
    } else {
        let mut mixed = MultiPart::mixed().multipart(body_part);
        for a in mail.attachments {
            let mime: ContentType = a.mime.parse().unwrap_or(ContentType::TEXT_PLAIN);
            let part = Attachment::new(a.filename).body(a.data, mime);
            mixed = mixed.singlepart(part);
        }
        builder.multipart(mixed)?
    };
    Ok(msg)
}

pub async fn send_mail(
    account: &Account,
    password: &str,
    mail: OutgoingMail,
) -> Result<Vec<u8>> {
    if account.smtp_host.is_empty() {
        return Err(anyhow!(
            "smtp_host not set; configure SMTP in account settings"
        ));
    }

    let msg = build_message(mail)?;
    let raw = msg.formatted();

    let creds = Credentials::new(account.username.clone(), password.to_string());
    let tls = TlsParameters::new(account.smtp_host.clone())?;

    let mut transport_builder =
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(account.smtp_host.clone())
            .port(account.smtp_port)
            .credentials(creds);
    transport_builder = if account.smtp_port == 465 {
        transport_builder.tls(Tls::Wrapper(tls))
    } else {
        transport_builder.tls(Tls::Required(tls))
    };
    let transport = transport_builder.build();

    crate::debug_log::push(
        "smtp",
        "→",
        format!(
            "SEND host={}:{} from={} ({} bytes)",
            account.smtp_host,
            account.smtp_port,
            account.username,
            raw.len()
        ),
    );
    match transport.send(msg).await {
        Ok(_) => {
            crate::debug_log::push("smtp", "←", "SEND OK");
            Ok(raw)
        }
        Err(e) => {
            crate::debug_log::push("smtp", "←", format!("SEND failed: {}", e));
            Err(anyhow::Error::from(e).context("SMTP send failed"))
        }
    }
}

pub fn auto_save_to_sent(account: &Account) -> bool {
    // Gmail / Googlemail save sent messages server-side automatically.
    let host = account.host.to_ascii_lowercase();
    !(host.ends_with("gmail.com") || host.ends_with("googlemail.com"))
}

#[cfg(test)]
mod tests {
    use super::format_message_id;

    #[test]
    fn formats_message_ids_for_rfc_headers() {
        assert_eq!(
            format_message_id("parent@example.com").as_deref(),
            Some("<parent@example.com>")
        );
        assert_eq!(
            format_message_id("<parent@example.com>").as_deref(),
            Some("<parent@example.com>")
        );
        assert_eq!(format_message_id(""), None);
    }
}
