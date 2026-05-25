use std::time::Duration;

use async_imap::extensions::idle::IdleResponse;
use tauri::{AppHandle, Emitter};
use tracing::{info, warn};

use crate::account::Account;
use crate::mail::imap;

#[derive(serde::Serialize, Clone, Debug)]
pub struct NewMailEvent {
    pub mailbox: String,
}

/// Run an IDLE loop for `mailbox` until the task is cancelled.
///
/// One cycle:
///   * connect a *dedicated* IMAP session
///   * verify the server advertises IDLE
///   * SELECT mailbox
///   * IDLE → wait → DONE → loop
///
/// On any new server data (EXISTS, EXPUNGE, …) we emit a `mail:new`
/// Tauri event so the frontend can refetch envelopes. Reconnects with
/// backoff if anything fails.
pub async fn run(
    account: Account,
    password: String,
    mailbox: String,
    app: AppHandle,
) {
    let mut backoff = Duration::from_secs(5);
    loop {
        match one_session(&account, &password, &mailbox, &app).await {
            Ok(()) => {
                backoff = Duration::from_secs(5);
            }
            Err(e) => {
                warn!(error = %e, mailbox = %mailbox, "IDLE session ended; reconnecting after backoff");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(5 * 60));
            }
        }
    }
}

async fn one_session(
    account: &Account,
    password: &str,
    mailbox: &str,
    app: &AppHandle,
) -> anyhow::Result<()> {
    let mut session =
        imap::connect(&account.host, account.port, &account.username, password).await?;

    let caps = session.capabilities().await?;
    if !caps.has_str("IDLE") {
        info!(
            host = %account.host,
            "server does not advertise IDLE; idle worker idling indefinitely"
        );
        // Park here forever; the task will be aborted by the supervisor
        // when the user switches mailbox / disconnects.
        std::future::pending::<()>().await;
        return Ok(());
    }

    crate::debug_log::push("idle", "→", format!("SELECT {:?}", mailbox));
    session.select(mailbox).await?;
    crate::debug_log::push("idle", "←", "SELECT OK");
    let mut session = session;

    loop {
        let mut handle = session.idle();
        handle.init().await?;
        crate::debug_log::push("idle", "→", "IDLE");
        let (fut, _stop_source) =
            handle.wait_with_timeout(Duration::from_secs(25 * 60));
        let response = fut.await?;
        let session_back = handle.done().await?;
        session = session_back;

        match response {
            IdleResponse::NewData(_) => {
                crate::debug_log::push(
                    "idle",
                    "←",
                    format!("NewData ({:?})", mailbox),
                );
                let _ = app.emit(
                    "mail:new",
                    NewMailEvent {
                        mailbox: mailbox.to_string(),
                    },
                );
            }
            IdleResponse::Timeout | IdleResponse::ManualInterrupt => {
                crate::debug_log::push("idle", "←", "Timeout/Interrupt");
            }
        }
    }
}
