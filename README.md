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
- **Search**: client-side filter (current envelopes) + server-side
  `UID SEARCH CHARSET UTF-8 TEXT ...`
- **Flag / read state**: mark read on open, toggle Flagged (`\Flagged`),
  Move-to-Trash (RFC 6851 `MOVE` with COPY+EXPUNGE fallback)
- **Secrets**: passwords stored in OS keychain
  (`apple-native` / `linux-native-sync-persistent` / `windows-native`)
- **macOS app menu**: ⌘, opens Settings, ⌘Q quits

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

## License

Dual-licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
