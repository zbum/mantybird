# Mantybird

Cross-platform IMAP desktop client built with **Tauri 2** (Rust backend) +
**React 18 + Vite** (frontend).

> Manty + Thunderbird — a small, fast, native-feeling email client that talks
> plain IMAP/SMTP and stores your data locally.

---

## Features

- **Multi-account IMAP**: Gmail, Naver, iCloud, Outlook.com, self-hosted
  Dovecot/Cyrus, ...
- **Local SQLite cache**: folders, envelope headers, message bodies persist
  across launches
- **IDLE push** for new mail on the selected mailbox (RFC 2177), 5-minute
  folder-list polling otherwise
- **Folder management**: create / rename / delete / subscribe / unsubscribe
  via right-click menu (RFC 6154 special-use folders are locked)
- **HTML mail rendering** inside the app (sandboxed iframe; `cid:` inline
  images embedded as `data:` URLs; external links open in the system browser)
- **Attachments**: list + download to a configurable folder
- **Compose**: WYSIWYG editor (TipTap), Reply, file attachments, HTML body,
  Sent-folder APPEND for non-Gmail servers (Gmail saves server-side)
- **Mail threads**: group conversations using `Message-ID`, `In-Reply-To`, and
  `References`; replies preserve RFC threading headers
- **Search**: client-side filter (current envelopes) + server-side
  `UID SEARCH CHARSET UTF-8 TEXT ...`
- **Flag / read state**: mark read on open, toggle Flagged (`\Flagged`),
  Move-to-Trash (RFC 6851 `MOVE` with COPY+EXPUNGE fallback)
- **Keyboard delete**: Backspace/Delete trashes the selected message;
  inside Junk/Trash it prompts for permanent delete (STORE `+\Deleted`
  + EXPUNGE)
- **Optimistic UI**: deletes apply to the list instantly and roll back
  on server failure (works in search results too)
- **Server-side deletion sync**: after a folder opens and on IDLE push,
  the top 1000 visible UIDs are reconciled against the server via
  `UID FETCH (UID)` so messages expunged from another client disappear
- **Background body prefetch**: after the envelope list loads, message
  bodies are slowly fetched in the background (1.5s spacing) so opening
  a mail later is instant. STORE / fetchBody initiated by the user
  always preempts the prefetch.
- **Window state persistence**: last window size & position restored on
  relaunch (`tauri-plugin-window-state`)
- **Developer Console**: Mantybird → Developer Console… (⌘⇧D) opens a
  separate window that streams every IMAP / SMTP / IDLE command and
  response in real time, with per-channel toggles
- **Secrets**: passwords stored in OS keychain
  (`apple-native` / `linux-native-sync-persistent` / `windows-native`)
- **macOS app menu**: ⌘, opens Settings, ⌘⇧D opens Developer Console,
  ⌘Q quits

---

## Requirements

- Node.js 20+ and npm
- Rust stable (`rustup`)
- macOS: Xcode Command Line Tools
- Linux: standard webkit2gtk dev packages
  (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`, ...)
- Windows: WebView2 Runtime (preinstalled on Windows 11)

---

## Quickstart

```bash
make install      # npm install
make dev          # npm run tauri dev — hot-reload dev mode
make package      # production .app + .dmg for current host
```

`make` is just a thin wrapper over `npm` / `cargo` / `tauri` commands —
see the `Makefile` for the full list of targets.

### Cross-platform packaging

```bash
make targets-install      # one-time: rustup add target triples
make package-darwin       # both aarch64 and x86_64 .app + .dmg
make package-linux        # x86_64 .deb / .AppImage / .rpm
make package-windows      # x86_64 .msi / .exe
```

Tauri bundling generally needs to run **on** (or cross-compile to) the
target OS for code signing, framework deps, etc.

---

## Where your data lives

| What | Where |
|---|---|
| Accounts metadata (host/port/user/SMTP) | `<config_dir>/dev.manty.manty-imap-desktop/config.json` |
| Mail cache (folders, headers, bodies) | `<data_dir>/dev.manty.manty-imap-desktop/cache.sqlite` |
| Passwords | OS keychain — service `manty-imap-desktop`, account `<user>@<host>` |
| Attachment downloads | configurable in Settings → 환경설정 → 다운로드 폴더 (default: browser default) |

On macOS, `<config_dir>` and `<data_dir>` are both
`~/Library/Application Support/`.

Nothing user-specific is ever written to the repo or shipped with the binary.

---

## Project layout

```
mantybird/
├── src/                        React frontend
│   ├── App.tsx
│   ├── api.ts                  Tauri command bindings
│   ├── types.ts
│   ├── main.tsx
│   └── App.css
├── src-tauri/                  Rust backend
│   └── src/
│       ├── main.rs / lib.rs    Tauri builder + commands
│       ├── account.rs          Account model
│       ├── config.rs           Multi-account config + keychain
│       ├── cache.rs            SQLite cache (folders / messages)
│       └── mail/
│           ├── imap.rs         IMAP client (async-imap + tokio-rustls)
│           ├── smtp.rs         SMTP client (lettre)
│           ├── idle.rs         IDLE worker (per selected mailbox)
│           └── message.rs      Shared mail types
├── package.json
├── Makefile
└── src-tauri/tauri.conf.json
```

Calendar (CalDAV) and contacts (CardDAV) are intentionally not in tree yet;
the module layout reserves a parallel `calendar/` and `contacts/` slot for
when they're added.

---

## Changelog

### 2026-06-30 — `feature/multi-delete-messages`

- Add desktop-style multi-select for messages with Cmd/Ctrl-click toggles,
  Shift-click range selection, a selected-message action bar, and a message
  context menu for deletion.
- Support bulk move-to-trash and bulk permanent delete with a single IMAP
  UID set operation, keeping lists and cache state in sync.
- Keep the message list layout stable while selecting or opening context
  menus.
- Verified with `cargo test` and `npm run build`.

### 2026-06-29 — `feature/folder-bulk-actions`

- Add folder context-menu actions for marking every message in a folder as
  read and permanently deleting every message in a folder.
- Keep envelope lists, unread badges, and the local message cache in sync
  after bulk folder actions.
- Verified with `cargo test` and `npm run build`.

### 2026-06-29 — `feature/folder-list-attributes`

- Preserve IMAP `\NoSelect` and `\NoInferiors` LIST attributes in the folder
  model and local cache.
- Prevent selecting `\NoSelect` hierarchy nodes as mailboxes, and prevent
  creating child folders under `\NoInferiors` or special-use folders.
- Verified with `cargo test` and `npm run build`.

### 2026-06-29 — `feature/refresh-mailbox-counts`

- Refresh the selected mailbox unread badge from server `STATUS (UNSEEN)`
  when opening a mailbox.
- Refresh source and Trash unread badges after moving or deleting mail, and
  persist the updated count in the local folder cache.
- Verified with `cargo test` and `npm run build`.

### 2026-06-25 — `bugfix/mark-seen-delay`

- Start the configured read-state delay when a message is selected, so a
  zero-second delay marks it read immediately even during rapid navigation.
- Apply saved delay settings to the active mailbox view immediately.
- Reuse cached message bodies without a redundant server fetch, except for
  Drafts where server refresh remains enabled.
- Verified with `cargo test`, `npm run build`, and `git diff --check`.

### 2026-06-25 — `feature/thread-mail`

- Group mailbox entries into expandable conversations using RFC
  `Message-ID`, `In-Reply-To`, and `References` headers.
- Persist thread metadata in SQLite with automatic migration for existing
  caches, and preserve thread headers when sending replies.
- Verified with `cargo test`, `npm run build`, and `git diff --check`.

---

## License

Dual-licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

---

## Changelog

### 2026-05-25 — feature/background-body-prefetch

- **Background body prefetch** — message bodies for the current folder
  are prefetched in the background (1.5s spacing) so opening a mail
  later is instant; the loop cancels on folder/account switch.
- **User actions preempt prefetch** — clicking a mail / mark-seen
  STORE / move-to-trash / permanent delete each push prefetch into a
  short pause window so the IMAP session is free for the user.
- **Server-side deletion sync** — new `prune_deleted(mailbox, uids)`
  Tauri command verifies the top 1000 visible UIDs against the server
  via `UID FETCH (UID)` and removes anything the server no longer has
  from both cache and UI. Triggered on folder open and on IDLE push.
- **Keyboard delete** — Backspace / Delete in the mailbox view trashes
  the selected message; in Junk / Trash it prompts and then `STORE
  +\Deleted` + `EXPUNGE` for permanent delete. Optimistic UI removal,
  rollback on server failure, also clears search results.
- **Race-condition fixes** — `listGenRef` invalidates stale envelope
  fetches when folder or account changes so late results can't
  overwrite the new view.
- **Window state persistence** — added `tauri-plugin-window-state`
  (capability `window-state:default`) for restored size & position.
- **Developer Console** — Mantybird → Developer Console… (⌘⇧D) opens
  a separate webview window that streams IMAP / SMTP / IDLE traffic
  via a new `debug_log` ring buffer + `debug:log` Tauri events. New
  Tauri command `get_debug_log()` returns a snapshot.
- **Lighter prune wire payload** — switched server-side reconciliation
  from `UID SEARCH ALL` (slow on large mailboxes) to
  `UID FETCH <set> (UID)` over a bounded UID set.
- **New module**: `src-tauri/src/debug_log.rs` (ring buffer + global
  handle bound to AppHandle during `setup`).
- **Schema**: `StoredConfig` unchanged. Cache schema unchanged.
