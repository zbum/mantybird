import { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { EditorContent, useEditor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Underline from "@tiptap/extension-underline";
import Link from "@tiptap/extension-link";
import * as api from "./api";
import type {
  Account,
  Envelope,
  Folder,
  MessageBody,
  StoredConfig,
} from "./types";
import { accountKey, DEFAULT_FOLDER_LABELS } from "./types";
import type { SpecialUse } from "./types";

interface ComposeAttachment {
  filename: string;
  mime: string;
  data_base64: string;
  size: number;
}

interface ComposeDraft {
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  isHtml: boolean;
  attachments: ComposeAttachment[];
  inReplyTo: string | null;
  references: string[];
}

const defaultAccount = (): Account => ({
  name: "",
  host: "",
  port: 993,
  username: "",
  smtp_host: "",
  smtp_port: 465,
});

const isDebugWindow =
  typeof window !== "undefined" && window.location.hash === "#debug";

export default function App() {
  // The Tauri "debug" window loads the same HTML bundle with #debug; render
  // only the console there so the main IMAP app doesn't boot a second time.
  if (isDebugWindow) {
    return <DebugWindowFrame />;
  }
  const [screen, setScreen] = useState<"login" | "mailbox">("login");

  const [account, setAccount] = useState<Account>(defaultAccount);
  const [password, setPassword] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [status, setStatus] = useState("");
  const [error, setError] = useState(false);

  const [folders, setFolders] = useState<Folder[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [envelopes, setEnvelopes] = useState<Envelope[]>([]);
  const [selectedUid, setSelectedUid] = useState<number | null>(null);
  const [body, setBody] = useState<MessageBody | null>(null);
  const [loadingBody, setLoadingBody] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [noMore, setNoMore] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchingServer, setSearchingServer] = useState(false);
  const [serverResults, setServerResults] = useState<Envelope[] | null>(null);
  const [expandedThreads, setExpandedThreads] = useState<Set<string>>(
    new Set(),
  );

  async function handleServerSearch() {
    if (!searchQuery.trim() || !selectedFolder) return;
    setSearchingServer(true);
    setStatus(`서버 검색 중: ${searchQuery}…`);
    try {
      const results = await api.searchMailbox(selectedFolder, searchQuery, 200);
      setServerResults(results);
      setStatus(`서버 검색 결과 ${results.length}건`);
    } catch (err) {
      setStatus(`서버 검색 실패: ${err}`);
    } finally {
      setSearchingServer(false);
    }
  }

  function clearSearch() {
    setSearchQuery("");
    setServerResults(null);
  }
  const [rootExpanded, setRootExpanded] = useState(true);
  const [allAccounts, setAllAccounts] = useState<Account[]>([]);
  const [foldersByAccount, setFoldersByAccount] = useState<
    Record<string, Folder[]>
  >({});
  const [expandedRoots, setExpandedRoots] = useState<Set<string>>(new Set());
  const envelopesRef = useRef<Envelope[]>([]);
  envelopesRef.current = envelopes;
  const selectedFolderRef = useRef<string | null>(null);
  selectedFolderRef.current = selectedFolder;
  const sentinelRef = useRef<HTMLDivElement | null>(null);
  const selectionGenRef = useRef(0);
  // Bumped on every action that should invalidate any in-flight envelope
  // list fetch (folder switch, account switch). Async paths capture the gen
  // at start and discard their results when it no longer matches.
  const listGenRef = useRef(0);
  const markSeenTimerRef = useRef<number | null>(null);
  const [markSeenDelay, setMarkSeenDelay] = useState(0);
  const markSeenDelayRef = useRef(0);
  markSeenDelayRef.current = markSeenDelay;
  // Tracks UIDs we've already attempted to prefetch in the current
  // account+folder, so successive envelope updates only fetch newcomers.
  const prefetchedBodiesRef = useRef<{ key: string; uids: Set<number> }>({
    key: "",
    uids: new Set(),
  });
  // Absolute timestamp (ms since epoch) until which background prefetch
  // should yield to user-initiated IMAP work (select, mark-seen, delete).
  const prefetchPausedUntilRef = useRef(0);

  const PAGE_SIZE = 50;
  // How many cached envelopes to load up-front when opening a folder.
  // Keep generous so previously-fetched messages stay visible across restarts.
  const CACHED_INITIAL_CAP = 5000;
  // Spacing between background body prefetches so the IMAP session stays
  // responsive for user-initiated reads.
  const BODY_PREFETCH_INTERVAL_MS = 1500;
  const BODY_PREFETCH_ERROR_BACKOFF_MS = 5000;

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [composeDraft, setComposeDraft] = useState<ComposeDraft | null>(null);
  const [folderLabels, setFolderLabels] = useState<Record<string, string>>(
    DEFAULT_FOLDER_LABELS,
  );

  const [promptState, setPromptState] = useState<{
    title: string;
    label?: string;
    defaultValue: string;
    resolve: (v: string | null) => void;
  } | null>(null);
  const [confirmState, setConfirmState] = useState<{
    message: string;
    resolve: (v: boolean) => void;
  } | null>(null);
  const [pwPromptState, setPwPromptState] = useState<{
    account: Account;
    message: string;
    resolve: (v: string | null) => void;
  } | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{
    x: number;
    y: number;
    folder: Folder | null;
  } | null>(null);

  const SIDEBAR_MIN = 180;
  const SIDEBAR_MAX = 480;
  const LIST_MIN = 260;
  const LIST_MAX = 720;
  const VIEWER_MIN = 320;
  const panesRef = useRef<HTMLDivElement | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    const v = Number(localStorage.getItem("mb.sidebarWidth"));
    return Number.isFinite(v) && v >= SIDEBAR_MIN && v <= SIDEBAR_MAX ? v : 260;
  });
  const [listWidth, setListWidth] = useState<number>(() => {
    const v = Number(localStorage.getItem("mb.listWidth"));
    return Number.isFinite(v) && v >= LIST_MIN && v <= LIST_MAX ? v : 360;
  });
  useEffect(() => {
    localStorage.setItem("mb.sidebarWidth", String(sidebarWidth));
  }, [sidebarWidth]);
  useEffect(() => {
    localStorage.setItem("mb.listWidth", String(listWidth));
  }, [listWidth]);

  function startResize(which: "sidebar" | "list") {
    return (e: React.MouseEvent) => {
      e.preventDefault();
      const startX = e.clientX;
      const startSidebar = sidebarWidth;
      const startList = listWidth;
      const containerW = panesRef.current?.clientWidth ?? 0;
      const onMove = (ev: MouseEvent) => {
        const dx = ev.clientX - startX;
        if (which === "sidebar") {
          let next = startSidebar + dx;
          next = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, next));
          const maxBySpace = containerW - startList - VIEWER_MIN - 10;
          if (maxBySpace > SIDEBAR_MIN) next = Math.min(next, maxBySpace);
          setSidebarWidth(next);
        } else {
          let next = startList + dx;
          next = Math.max(LIST_MIN, Math.min(LIST_MAX, next));
          const maxBySpace = containerW - startSidebar - VIEWER_MIN - 10;
          if (maxBySpace > LIST_MIN) next = Math.min(next, maxBySpace);
          setListWidth(next);
        }
      };
      const onUp = () => {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    };
  }

  function askPrompt(
    title: string,
    defaultValue = "",
    label?: string,
  ): Promise<string | null> {
    return new Promise((resolve) =>
      setPromptState({ title, label, defaultValue, resolve }),
    );
  }
  function askConfirm(message: string): Promise<boolean> {
    return new Promise((resolve) => setConfirmState({ message, resolve }));
  }
  function askPassword(
    acc: Account,
    message: string,
  ): Promise<string | null> {
    return new Promise((resolve) =>
      setPwPromptState({ account: acc, message, resolve }),
    );
  }

  useEffect(() => {
    if (!ctxMenu) return;
    function close(e: MouseEvent) {
      const t = e.target as HTMLElement | null;
      if (t && t.closest(".ctx-menu")) return;
      setCtxMenu(null);
    }
    document.addEventListener("mousedown", close);
    return () => {
      document.removeEventListener("mousedown", close);
    };
  }, [ctxMenu]);

  useEffect(() => {
    api
      .getFolderLabels()
      .then((labels) =>
        setFolderLabels({ ...DEFAULT_FOLDER_LABELS, ...labels }),
      )
      .catch((e) => console.warn("getFolderLabels failed", e));
  }, [settingsOpen]);

  useEffect(() => {
    api
      .getMarkSeenDelay()
      .then(setMarkSeenDelay)
      .catch((e) => console.warn("getMarkSeenDelay failed", e));
  }, [settingsOpen]);

  function openCompose(draft?: Partial<ComposeDraft>) {
    setComposeDraft({
      to: "",
      cc: "",
      bcc: "",
      subject: "",
      body: "",
      isHtml: false,
      attachments: [],
      inReplyTo: null,
      references: [],
      ...(draft || {}),
    });
  }

  function openReply(b: MessageBody) {
    const subject = b.subject.startsWith("Re:") ? b.subject : `Re: ${b.subject}`;
    const fromEsc = escapeHtml(b.from);
    const dateEsc = escapeHtml(b.date);
    const originalHtml = b.html ?? plainToHtml(b.text);
    const body = `<p><br/></p><p>${dateEsc}, ${fromEsc} wrote:</p><blockquote>${originalHtml}</blockquote>`;
    openCompose({
      to: b.from,
      subject,
      body,
      isHtml: true,
      inReplyTo: b.message_id,
      references: appendReference(b.references, b.message_id),
    });
  }

  async function startWithAccount(acc: Account) {
    setAccount(acc);
    let pw = await api.loadPassword(acc);
    const savedExpanded = await api
      .getExpandedFolders()
      .catch(() => [] as string[]);
    const expandedFromConfig =
      savedExpanded.length > 0 ? new Set(savedExpanded) : null;
    try {
      const cachedF = await api.cachedFolders(acc);
      if (cachedF.length > 0) {
        setFolders(cachedF);
        setExpanded(
          expandedFromConfig ??
            new Set(cachedF.filter((f) => f.has_children).map((f) => f.raw)),
        );
        setScreen("mailbox");
        setStatus(`Cached · ${cachedF.length} folders · reconnecting…`);
      }
    } catch (e) {
      console.warn("cachedFolders failed", e);
    }
    if (!pw) return;
    setPassword(pw);
    setConnecting(true);
    try {
      while (true) {
        try {
          const fs = await api.connectImap(
            acc.host,
            acc.port,
            acc.username,
            pw,
          );
          setFolders(fs);
          setExpanded(
            expandedFromConfig ??
              new Set(fs.filter((f) => f.has_children).map((f) => f.raw)),
          );
          setScreen("mailbox");
          setStatus(`Connected · ${fs.length} folders`);
          try {
            const last = await api.getLastMailbox();
            if (last) {
              const match = fs.find((f) => f.raw === last);
              if (match) handleSelectFolder(match);
            }
          } catch (e) {
            console.warn("restore last mailbox failed", e);
          }
          return;
        } catch (err) {
          if (isAuthError(err)) {
            const newPw = await askPassword(
              acc,
              "비밀번호가 올바르지 않습니다. 다시 입력해주세요.",
            );
            if (!newPw) {
              setStatus("비밀번호가 올바르지 않습니다");
              setError(true);
              return;
            }
            pw = newPw;
            setPassword(newPw);
            await api.savePassword(acc, newPw);
          } else {
            setStatus(`Auto-connect failed: ${err}`);
            setError(true);
            return;
          }
        }
      }
    } finally {
      setConnecting(false);
    }
  }

  useEffect(() => {
    (async () => {
      const cur = await api.currentAccount();
      if (cur) {
        await startWithAccount(cur);
      }
      await refreshAllAccountFolders();
    })().catch((e) => console.error("init failed", e));
  }, []);

  // Sync the multi-account folder cache with active account changes
  // and folder updates so non-current account trees stay reasonably fresh.
  useEffect(() => {
    if (folders.length === 0) return;
    const key = accountKey(account);
    setFoldersByAccount((prev) => ({ ...prev, [key]: folders }));
  }, [folders, account]);

  // Refresh the multi-account list whenever Settings closes (user may
  // have added/edited/removed accounts).
  useEffect(() => {
    if (!settingsOpen) refreshAllAccountFolders().catch(() => {});
  }, [settingsOpen]);

  async function refreshAllAccountFolders() {
    try {
      const cfg = await api.listAccounts();
      setAllAccounts(cfg.accounts);
      const map: Record<string, Folder[]> = {};
      for (const a of cfg.accounts) {
        try {
          const fs = await api.cachedFolders(a);
          map[accountKey(a)] = fs;
        } catch (e) {
          console.warn("cachedFolders failed for", a, e);
          map[accountKey(a)] = [];
        }
      }
      setFoldersByAccount(map);
      // Expand current account root by default
      if (cfg.current_key) {
        setExpandedRoots((prev) => {
          const next = new Set(prev);
          next.add(cfg.current_key!);
          return next;
        });
      }
    } catch (e) {
      console.warn("refreshAllAccountFolders failed", e);
    }
  }

  useEffect(() => {
    function onMessage(ev: MessageEvent) {
      const data = ev.data;
      if (
        data &&
        typeof data === "object" &&
        data.type === "manty-open-link" &&
        typeof data.url === "string"
      ) {
        api.openUrl(data.url).catch((e) => console.warn("open_url failed", e));
      }
    }
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  useEffect(() => {
    const unlistenPromise = listen("settings:open", () => {
      setSettingsOpen(true);
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<Folder[]>("folders:changed", (event) => {
      setFolders(event.payload);
      setExpanded((prev) => {
        // Keep previously-expanded entries that still exist; auto-expand
        // any new parent nodes so the new folders are visible.
        const valid = new Set(event.payload.map((f) => f.raw));
        const next = new Set<string>();
        prev.forEach((p) => {
          if (valid.has(p)) next.add(p);
        });
        event.payload.forEach((f) => {
          if (f.has_children) next.add(f.raw);
        });
        return next;
      });
      setStatus("폴더 목록 갱신");
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<{ mailbox: string }>(
      "mail:new",
      async (event) => {
        const mb = event.payload.mailbox;
        if (selectedFolderRef.current !== mb) return;
        const gen = listGenRef.current;
        try {
          const envs = await api.fetchEnvelopes(mb, 0, PAGE_SIZE);
          if (listGenRef.current !== gen || selectedFolderRef.current !== mb)
            return;
          let total = 0;
          setEnvelopes((prev) => {
            const merged = mergeEnvelopes(envs, prev);
            total = merged.length;
            return merged;
          });
          setStatus(`${total} messages · 새 메일 도착`);
        } catch (e) {
          console.warn("mail:new refetch failed", e);
        }
        // IDLE fires NewData on EXPUNGE too — reconcile after every push so
        // messages deleted from another client disappear here as well.
        try {
          // Only check the UIDs that the user can plausibly see (top 1000).
          // Verifying every cached UID is prohibitively expensive on large
          // mailboxes; this keeps the wire payload bounded.
          const visibleUids = envelopesRef.current
            .slice(0, 1000)
            .map((e) => e.uid);
          if (visibleUids.length === 0) return;
          const removed = await api.pruneDeleted(mb, visibleUids);
          if (listGenRef.current !== gen || selectedFolderRef.current !== mb)
            return;
          if (removed.length > 0) applyServerDeletions(removed);
        } catch (e) {
          console.warn("pruneDeleted (mail:new) failed", e);
        }
      },
    );
    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
      } else if (e.key === "Escape" && settingsOpen) {
        setSettingsOpen(false);
      }
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [settingsOpen]);

  // Backspace / Delete on the mailbox view: trash the selected message,
  // or permanently delete (with confirm) when the user is already inside
  // the Junk or Trash folder where "move to trash" makes no sense.
  // Capture phase so the browser's default "navigate back" on Backspace
  // can't pre-empt us, and so an iframe-focused body view still bubbles
  // the event up before something else swallows it.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Backspace" && e.key !== "Delete") return;
      if (screen !== "mailbox") return;
      const t = e.target as HTMLElement | null;
      if (t) {
        if (t.tagName === "INPUT" || t.tagName === "TEXTAREA") return;
        if (t.isContentEditable) return;
      }
      if (composeDraft || promptState || confirmState || settingsOpen) return;
      // Suppress the browser's default action regardless of whether we can
      // delete — otherwise focus on `body` lets Backspace navigate back.
      e.preventDefault();
      e.stopPropagation();
      if (selectedUid === null || !selectedFolder) {
        setStatus("삭제할 메일을 먼저 선택하세요");
        return;
      }
      deleteSelectedFromKeyboard();
    }
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [
    screen,
    selectedUid,
    selectedFolder,
    folders,
    composeDraft,
    promptState,
    confirmState,
    settingsOpen,
  ]);

  async function handleConnect(e: React.FormEvent) {
    e.preventDefault();
    if (!account.host || !account.username || !password) {
      setStatus("host/port/username/password를 모두 입력하세요");
      setError(true);
      return;
    }
    setConnecting(true);
    setError(false);
    setStatus("Connecting…");
    let pw = password;
    try {
      while (true) {
        try {
          const fs = await api.connectImap(
            account.host,
            account.port,
            account.username,
            pw,
          );
          await api.upsertAccount(account);
          await api.setCurrentAccount(account);
          await api.savePassword(account, pw);
          setPassword(pw);
          setFolders(fs);
          setExpanded(
            new Set(fs.filter((f) => f.has_children).map((f) => f.raw)),
          );
          setStatus(`Connected · ${fs.length} folders`);
          setScreen("mailbox");
          return;
        } catch (err) {
          if (isAuthError(err)) {
            const newPw = await askPassword(
              account,
              "비밀번호가 올바르지 않습니다. 다시 입력해주세요.",
            );
            if (!newPw) {
              setStatus("비밀번호가 올바르지 않습니다");
              setError(true);
              return;
            }
            pw = newPw;
            setPassword(newPw);
          } else {
            setStatus(`Connect failed: ${err}`);
            setError(true);
            return;
          }
        }
      }
    } finally {
      setConnecting(false);
    }
  }

  async function handleSelectFolder(folder: Folder) {
    if (folder.no_select) {
      if (folder.has_children) {
        toggleFolder(folder.raw);
      }
      setStatus(`선택할 수 없는 폴더: ${folder.leaf}`);
      return;
    }
    selectionGenRef.current++;
    const gen = ++listGenRef.current;
    setSelectedFolder(folder.raw);
    setBody(null);
    setSelectedUid(null);
    setEnvelopes([]);
    setNoMore(false);
    setServerResults(null);
    setExpandedThreads(new Set());
    api.setLastMailbox(folder.raw).catch(() => {});
    setStatus(`Loading ${folder.leaf}…`);

    try {
      // Load all cached envelopes so the user sees the full backlog
      // they previously scrolled to, not just the newest page.
      const cachedE = await api.cachedEnvelopes(
        account,
        folder.raw,
        null,
        CACHED_INITIAL_CAP,
      );
      if (listGenRef.current !== gen) return;
      setEnvelopes(cachedE);
    } catch (e) {
      if (listGenRef.current !== gen) return;
      console.warn("cachedEnvelopes failed", e);
    }

    try {
      const envs = await api.fetchEnvelopes(folder.raw, 0, PAGE_SIZE);
      if (listGenRef.current !== gen) return;
      let total = 0;
      setEnvelopes((prev) => {
        const merged = mergeEnvelopes(envs, prev);
        total = merged.length;
        return merged;
      });
      setStatus(`${total} messages`);
      if (envs.length < PAGE_SIZE) setNoMore(true);
    } catch (err) {
      if (listGenRef.current !== gen) return;
      setStatus(`Fetch failed: ${err}`);
    }

    refreshFolderUnreadCount(folder.raw).catch((e) =>
      console.warn("refreshFolderUnreadCount failed", e),
    );

    // Reconcile against server-side deletions for the UIDs the user can
    // actually see (top 1000). Verifying everything cached would be
    // prohibitively expensive on large mailboxes.
    try {
      const visibleUids = envelopesRef.current
        .slice(0, 1000)
        .map((e) => e.uid);
      if (visibleUids.length === 0) return;
      const removed = await api.pruneDeleted(folder.raw, visibleUids);
      if (listGenRef.current !== gen) return;
      if (removed.length > 0) {
        applyServerDeletions(removed);
      }
    } catch (e) {
      console.warn("pruneDeleted failed", e);
    }
  }

  function applyServerDeletions(uids: number[]) {
    if (uids.length === 0) return;
    const removed = new Set(uids);
    setEnvelopes((prev) => prev.filter((e) => !removed.has(e.uid)));
    setSelectedUid((prev) => {
      if (prev !== null && removed.has(prev)) {
        setBody(null);
        return null;
      }
      return prev;
    });
    setStatus(`서버에서 ${uids.length}건 삭제 감지 — 목록 정리`);
  }

  async function refreshFolderUnreadCount(raw: string): Promise<void> {
    const unreadCount = await api.refreshMailboxCount(raw);
    setFolders((prev) =>
      prev.map((f) =>
        f.raw === raw ? { ...f, unread_count: unreadCount } : f,
      ),
    );
  }

  async function refreshFolderUnreadCounts(raws: string[]): Promise<void> {
    const unique = Array.from(new Set(raws.filter(Boolean)));
    await Promise.all(unique.map((raw) => refreshFolderUnreadCount(raw)));
  }

  async function loadMore() {
    const folder = selectedFolderRef.current;
    if (!folder || loadingMore || noMore) return;
    const gen = listGenRef.current;
    setLoadingMore(true);
    const current = envelopesRef.current;
    const oldestUid = current.length > 0 ? current[current.length - 1].uid : null;

    // Cached page below oldest
    try {
      const cachedE = await api.cachedEnvelopes(
        account,
        folder,
        oldestUid,
        PAGE_SIZE,
      );
      if (listGenRef.current !== gen) {
        setLoadingMore(false);
        return;
      }
      if (cachedE.length > 0) {
        setEnvelopes((prev) => mergeEnvelopes(prev, cachedE));
      }
    } catch (e) {
      if (listGenRef.current !== gen) {
        setLoadingMore(false);
        return;
      }
      console.warn("cachedEnvelopes (more) failed", e);
    }

    // Live page
    try {
      const offset = envelopesRef.current.length;
      const envs = await api.fetchEnvelopes(folder, offset, PAGE_SIZE);
      if (listGenRef.current !== gen) {
        setLoadingMore(false);
        return;
      }
      if (envs.length === 0) {
        setNoMore(true);
      } else {
        let total = 0;
        setEnvelopes((prev) => {
          const merged = mergeEnvelopes(prev, envs);
          total = merged.length;
          return merged;
        });
        setStatus(`${total} messages`);
        if (envs.length < PAGE_SIZE) setNoMore(true);
      }
    } catch (err) {
      console.warn("fetchEnvelopes (more) failed", err);
    } finally {
      setLoadingMore(false);
    }
  }

  useEffect(() => {
    if (!sentinelRef.current) return;
    const sentinel = sentinelRef.current;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          loadMore();
        }
      },
      { root: null, rootMargin: "120px", threshold: 0 },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [selectedFolder, loadingMore, noMore, account]);

  // Background body prefetch: once envelopes are listed for the current
  // folder, slowly walk the list and cache each body so a user click feels
  // instant. Cancels cleanly on folder/account switch.
  useEffect(() => {
    if (!selectedFolder || !account.host || envelopes.length === 0) return;
    const folder = selectedFolder;
    const acc = account;
    const key = `${accountKey(acc)}::${folder}`;
    if (prefetchedBodiesRef.current.key !== key) {
      prefetchedBodiesRef.current = { key, uids: new Set() };
    }
    const processed = prefetchedBodiesRef.current.uids;
    const snapshot = envelopes.slice();
    let cancelled = false;
    const sleep = (ms: number) =>
      new Promise<void>((resolve) => setTimeout(resolve, ms));

    const stillCurrent = () =>
      !cancelled &&
      selectedFolderRef.current === folder &&
      accountKey(account) === accountKey(acc);

    (async () => {
      // Give the user a moment to click around before we start hitting IMAP.
      await sleep(800);
      for (const env of snapshot) {
        if (!stillCurrent()) return;
        // STORE / select / fetchBody initiated by the user must take
        // priority over background body fetching. Yield until the
        // pause window passes before each iteration.
        while (
          Date.now() < prefetchPausedUntilRef.current &&
          stillCurrent()
        ) {
          await sleep(200);
        }
        if (!stillCurrent()) return;
        if (processed.has(env.uid)) continue;
        processed.add(env.uid);
        try {
          const cached = await api.cachedBody(acc, folder, env.uid);
          if (!stillCurrent()) return;
          if (cached) {
            // Already on disk — yield briefly and move on.
            await sleep(30);
            continue;
          }
          if (!stillCurrent()) return;
          await api.fetchBody(folder, env.uid);
          if (!stillCurrent()) return;
          await sleep(BODY_PREFETCH_INTERVAL_MS);
        } catch (e) {
          // Keep the uid in `processed` so a malformed/empty message
          // doesn't get retried indefinitely on every envelope update.
          console.warn("body prefetch failed", env.uid, e);
          await sleep(BODY_PREFETCH_ERROR_BACKOFF_MS);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedFolder, envelopes, account]);

  function pausePrefetch(durationMs: number) {
    const next = Date.now() + durationMs;
    if (next > prefetchPausedUntilRef.current) {
      prefetchPausedUntilRef.current = next;
    }
  }

  async function handleSelectMessage(uid: number) {
    // Body prefetch yields so the follow-up STORE for mark-seen and any
    // live fetchBody for this click hit the IMAP session immediately.
    pausePrefetch(8000);
    const gen = ++selectionGenRef.current;
    const folder = selectedFolder;
    if (markSeenTimerRef.current !== null) {
      clearTimeout(markSeenTimerRef.current);
      markSeenTimerRef.current = null;
    }
    setSelectedUid(uid);
    setBody(null);
    setLoadingBody(true);
    if (folder) {
      // The configured delay starts when the user selects the message, not
      // after a potentially slow cache lookup or full-body IMAP fetch.
      scheduleMarkSeen(gen, folder, uid);
    }
    let hasCachedBody = false;
    if (folder) {
      try {
        const cached = await api.cachedBody(account, folder, uid);
        if (selectionGenRef.current !== gen) return;
        if (cached) {
          hasCachedBody = true;
          setBody(cached);
          setLoadingBody(false);
        }
      } catch (e) {
        if (selectionGenRef.current !== gen) return;
        console.warn("cachedBody failed", e);
      }
    }
    if (!folder) {
      if (selectionGenRef.current === gen) setLoadingBody(false);
      return;
    }
    const selectedFolderInfo = folders.find((item) => item.raw === folder);
    const shouldRefreshFromServer =
      !hasCachedBody || selectedFolderInfo?.special === "Drafts";
    if (!shouldRefreshFromServer) {
      return;
    }
    try {
      const b = await api.fetchBody(folder, uid);
      if (selectionGenRef.current !== gen) return;
      setBody(b);
    } catch (err) {
      if (selectionGenRef.current !== gen) return;
      setStatus(`Body fetch failed: ${err}`);
    } finally {
      if (selectionGenRef.current === gen) setLoadingBody(false);
    }
  }

  function scheduleMarkSeen(gen: number, folder: string, uid: number) {
    const target = envelopesRef.current.find((e) => e.uid === uid);
    if (!target || target.seen) return;
    const delay = markSeenDelayRef.current;
    const run = () => {
      markSeenTimerRef.current = null;
      if (selectionGenRef.current !== gen) return;
      applyMarkSeen(folder, uid);
    };
    if (delay > 0) {
      markSeenTimerRef.current = window.setTimeout(run, delay * 1000);
    } else {
      run();
    }
  }

  async function applyMarkSeen(folder: string, uid: number) {
    pausePrefetch(5000);
    // Optimistic envelope + folder unread-count update.
    setEnvelopes((prev) =>
      prev.map((e) =>
        e.uid === uid
          ? {
              ...e,
              seen: true,
              flags: e.flags.some((f) => f.toLowerCase() === "\\seen")
                ? e.flags
                : [...e.flags, "\\Seen"],
            }
          : e,
      ),
    );
    setFolders((prev) =>
      prev.map((f) =>
        f.raw === folder
          ? { ...f, unread_count: Math.max(0, f.unread_count - 1) }
          : f,
      ),
    );
    try {
      const newFlags = await api.markSeen(folder, uid, true);
      setEnvelopes((prev) =>
        prev.map((e) =>
          e.uid === uid ? { ...e, flags: newFlags, seen: true } : e,
        ),
      );
      refreshFolderUnreadCount(folder).catch((e) =>
        console.warn("refreshFolderUnreadCount after markSeen failed", e),
      );
    } catch (err) {
      console.warn("markSeen failed; rolling back", err);
      setEnvelopes((prev) =>
        prev.map((e) =>
          e.uid === uid
            ? {
                ...e,
                seen: false,
                flags: e.flags.filter(
                  (f) => f.toLowerCase() !== "\\seen",
                ),
              }
            : e,
        ),
      );
      setFolders((prev) =>
        prev.map((f) =>
          f.raw === folder
            ? { ...f, unread_count: f.unread_count + 1 }
            : f,
        ),
      );
    }
  }

  function toggleFolder(raw: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(raw)) next.delete(raw);
      else next.add(raw);
      api.setExpandedFolders(Array.from(next)).catch(() => {});
      return next;
    });
  }

  async function handleCreateMailbox(parent?: Folder) {
    if (parent && parent.special !== "Other") {
      setStatus(`특수 폴더 아래에는 하위 폴더를 만들 수 없습니다: ${parent.leaf}`);
      return;
    }
    if (parent?.no_inferiors) {
      setStatus(`하위 폴더 생성 불가: ${parent.leaf}`);
      return;
    }
    const title = parent
      ? `${parent.leaf} 안에 새 하위 폴더`
      : "새 폴더";
    const name = await askPrompt(title, "", "폴더 이름");
    if (!name) return;
    try {
      const fs = await api.createMailbox(name, parent?.raw ?? null);
      setFolders(fs);
      setStatus(`폴더 생성: ${name}`);
    } catch (err) {
      setStatus(`폴더 생성 실패: ${err}`);
    }
  }

  async function handleRenameMailbox(folder: Folder) {
    const newLeaf = await askPrompt("폴더 이름 변경", folder.leaf, "새 이름");
    if (!newLeaf || newLeaf === folder.leaf) return;
    try {
      const fs = await api.renameMailbox(folder.raw, newLeaf);
      setFolders(fs);
      setStatus(`이름 변경: ${folder.leaf} → ${newLeaf}`);
    } catch (err) {
      setStatus(`이름 변경 실패: ${err}`);
    }
  }

  async function handleDeleteMailbox(folder: Folder) {
    const ok = await askConfirm(
      `'${folder.leaf}' 폴더를 삭제할까요?\n안의 메일이 모두 사라집니다.`,
    );
    if (!ok) return;
    try {
      const fs = await api.deleteMailbox(folder.raw);
      setFolders(fs);
      if (selectedFolder === folder.raw) {
        setSelectedFolder(null);
        setEnvelopes([]);
        setBody(null);
        setSelectedUid(null);
      }
      setStatus(`삭제: ${folder.leaf}`);
    } catch (err) {
      setStatus(`삭제 실패: ${err}`);
    }
  }

  async function handleToggleFlag(uid: number) {
    if (!selectedFolder) return;
    const env = envelopes.find((e) => e.uid === uid);
    const isFlagged = env?.flags.some((f) => f.toLowerCase() === "\\flagged") ?? false;
    try {
      const newFlags = await api.setFlag(selectedFolder, uid, "\\Flagged", !isFlagged);
      setEnvelopes((prev) =>
        prev.map((e) =>
          e.uid === uid
            ? {
                ...e,
                flags: newFlags,
                seen: newFlags.some((f) => f.toLowerCase() === "\\seen"),
              }
            : e,
        ),
      );
    } catch (err) {
      setStatus(`플래그 설정 실패: ${err}`);
    }
  }

  // Optimistically removes a UID from envelopes and server-search results,
  // clears the body if it was the active selection, and returns a rollback
  // closure for the caller to invoke if the underlying API call fails.
  function optimisticallyRemove(uid: number): () => void {
    const prevEnvelopes = envelopes;
    const prevServerResults = serverResults;
    const prevSelectedUid = selectedUid;
    const prevBody = body;
    setEnvelopes((prev) => prev.filter((e) => e.uid !== uid));
    setServerResults((prev) =>
      prev ? prev.filter((e) => e.uid !== uid) : prev,
    );
    if (selectedUid === uid) {
      setSelectedUid(null);
      setBody(null);
    }
    return () => {
      setEnvelopes(prevEnvelopes);
      setServerResults(prevServerResults);
      if (prevSelectedUid === uid) {
        setSelectedUid(prevSelectedUid);
        setBody(prevBody);
      }
    };
  }

  async function handleMoveToTrash(uid: number) {
    if (!selectedFolder) return;
    const sourceFolder = selectedFolder;
    const trashFolder = folders.find((f) => f.special === "Trash")?.raw;
    const ok = await askConfirm("이 메일을 휴지통으로 옮길까요?");
    if (!ok) return;
    pausePrefetch(5000);
    const rollback = optimisticallyRemove(uid);
    setStatus("휴지통으로 이동 중…");
    try {
      await api.moveToTrash(sourceFolder, uid);
      await refreshFolderUnreadCounts(
        trashFolder ? [sourceFolder, trashFolder] : [sourceFolder],
      );
      setStatus("휴지통으로 이동");
    } catch (err) {
      rollback();
      setStatus(`삭제 실패: ${err}`);
    }
  }

  async function deleteSelectedFromKeyboard() {
    const uid = selectedUid;
    const folder = selectedFolder;
    if (uid === null || !folder) return;
    const current = folders.find((f) => f.raw === folder);
    const isProtected =
      current?.special === "Junk" || current?.special === "Trash";
    if (isProtected) {
      const label = current?.special === "Junk" ? "스팸함" : "휴지통";
      const ok = await askConfirm(
        `${label}의 메일을 완전히 삭제합니다. 복구할 수 없습니다. 계속할까요?`,
      );
      if (!ok) return;
      pausePrefetch(5000);
      const rollback = optimisticallyRemove(uid);
      setStatus("완전 삭제 중…");
      try {
        await api.deletePermanent(folder, uid);
        await refreshFolderUnreadCount(folder);
        setStatus("완전 삭제됨");
      } catch (err) {
        rollback();
        setStatus(`완전 삭제 실패: ${err}`);
      }
      return;
    }
    pausePrefetch(5000);
    const rollback = optimisticallyRemove(uid);
    const trashFolder = folders.find((f) => f.special === "Trash")?.raw;
    setStatus("휴지통으로 이동 중…");
    try {
      await api.moveToTrash(folder, uid);
      await refreshFolderUnreadCounts(
        trashFolder ? [folder, trashFolder] : [folder],
      );
      setStatus("휴지통으로 이동");
    } catch (err) {
      rollback();
      setStatus(`삭제 실패: ${err}`);
    }
  }

  async function handleSubscribe(folder: Folder) {
    try {
      const fs = await api.subscribeMailbox(folder.raw);
      setFolders(fs);
      setStatus(`구독: ${folder.leaf}`);
    } catch (err) {
      setStatus(`구독 실패: ${err}`);
    }
  }

  async function handleUnsubscribe(folder: Folder) {
    try {
      const fs = await api.unsubscribeMailbox(folder.raw);
      setFolders(fs);
      setStatus(`구독 해제: ${folder.leaf}`);
    } catch (err) {
      setStatus(`구독 해제 실패: ${err}`);
    }
  }

  async function handleSwitchAccount(target: Account) {
    setSettingsOpen(false);
    // Invalidate any envelope/body fetches still in flight for the
    // previous account so their late results can't overwrite the new
    // account's mailbox view.
    listGenRef.current++;
    selectionGenRef.current++;
    try {
      await api.disconnectImap();
    } catch (e) {
      console.warn("disconnect on switch failed", e);
    }
    await api.setCurrentAccount(target);
    setFolders([]);
    setExpanded(new Set());
    setEnvelopes([]);
    setSelectedFolder(null);
    setSelectedUid(null);
    setBody(null);
    setScreen("login");
    setPassword("");
    await startWithAccount(target);
  }

  function toggleRootExpand(key: string) {
    setExpandedRoots((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function emailOf(a: Account): string {
    return a.username.includes("@") ? a.username : `${a.username}@${a.host}`;
  }

  function filterVisibleFolders(list: Folder[]): Folder[] {
    const byPath = new Map(list.map((f) => [f.raw, f]));
    return list.filter((f) => {
      let p = f.parent_path;
      while (p) {
        if (!expanded.has(p)) return false;
        p = byPath.get(p)?.parent_path ?? null;
      }
      return true;
    });
  }

  function renderFolderRow(
    f: Folder,
    idx: number,
    visible: Folder[],
    owner: Account,
    isCurrent: boolean,
  ) {
    const locked = f.special !== "Other";
    const prev = idx > 0 ? visible[idx - 1] : null;
    const showSeparator =
      prev && prev.special !== "Other" && f.special === "Other";
    const folderEl = (
      <div
        key={f.raw}
        className={`folder ${selectedFolder === f.raw && isCurrent ? "selected" : ""} ${f.subscribed ? "" : "unsubscribed"} ${f.unread_count > 0 ? "has-unread" : ""} ${f.no_select ? "no-select" : ""}`}
        style={{ paddingLeft: 8 + f.depth * 14 }}
        onClick={async () => {
          if (!isCurrent) {
            await handleSwitchAccount(owner);
            return;
          }
          handleSelectFolder(f);
        }}
        onContextMenu={(e) => {
          if (!isCurrent) return;
          e.preventDefault();
          setCtxMenu({ x: e.clientX, y: e.clientY, folder: f });
        }}
      >
        <span
          className={`toggle ${f.has_children ? "clickable" : ""}`}
          onClick={(e) => {
            if (!f.has_children) return;
            e.stopPropagation();
            toggleFolder(f.raw);
          }}
        >
          {f.has_children ? (expanded.has(f.raw) ? "▼" : "▶") : ""}
        </span>
        <span className="leaf">
          {locked && folderLabels[f.special]
            ? folderLabels[f.special]
            : f.leaf}
        </span>
        {f.unread_count > 0 && (
          <span className="unread-count">{f.unread_count}</span>
        )}
        {locked && f.special !== "Inbox" && (
          <span className="badge">{f.special}</span>
        )}
      </div>
    );
    if (showSeparator) {
      return (
        <div key={`sep-${f.raw}`} className="folder-separator">
          {folderEl}
        </div>
      );
    }
    return folderEl;
  }

  function renderAccountTrees() {
    const currentKey = accountKey(account);
    const accountsList =
      allAccounts.length > 0
        ? allAccounts
        : account.host
          ? [account]
          : [];
    return accountsList.map((acc) => {
      const aKey = accountKey(acc);
      const isCurrent = aKey === currentKey;
      const accFolders = isCurrent
        ? folders
        : foldersByAccount[aKey] ?? [];
      const isRootExpanded = expandedRoots.has(aKey);
      const visible = isCurrent
        ? filterVisibleFolders(accFolders)
        : accFolders;
      return (
        <div key={aKey}>
          <div
            className={`folder folder-root ${isCurrent ? "active-root" : ""}`}
            onClick={() => toggleRootExpand(aKey)}
            onContextMenu={(e) => {
              if (!isCurrent) return;
              e.preventDefault();
              setCtxMenu({ x: e.clientX, y: e.clientY, folder: null });
            }}
            title={
              isCurrent
                ? "클릭하여 펼치기/접기, 우클릭하여 메뉴"
                : "클릭하여 펼치기/접기"
            }
          >
            <span className="root-icon">{isRootExpanded ? "▾" : "▸"}</span>
            <span className="leaf">
              {acc.name ? (
                <>
                  {acc.name}{" "}
                  <span className="root-email">({emailOf(acc)})</span>
                </>
              ) : (
                emailOf(acc)
              )}
            </span>
          </div>
          {isRootExpanded &&
            visible.map((f, idx) =>
              renderFolderRow(f, idx, visible, acc, isCurrent),
            )}
        </div>
      );
    });
  }

  const visibleFolders = useMemo(() => {
    const byPath = new Map(folders.map((f) => [f.raw, f]));
    return folders.filter((f) => {
      let p = f.parent_path;
      while (p) {
        if (!expanded.has(p)) return false;
        p = byPath.get(p)?.parent_path ?? null;
      }
      return true;
    });
  }, [folders, expanded]);

  const displayedEnvelopes = useMemo(() => {
    if (serverResults) return serverResults;
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      return envelopes.filter(
        (e) =>
          e.subject.toLowerCase().includes(q) ||
          e.from.toLowerCase().includes(q),
      );
    }
    return envelopes;
  }, [serverResults, searchQuery, envelopes]);

  const threads = useMemo(
    () => buildThreads(displayedEnvelopes),
    [displayedEnvelopes],
  );
  const visibleThreadEnvelopes = useMemo(
    () =>
      threads.flatMap((thread) =>
        thread.messages.length > 1 && !expandedThreads.has(thread.id)
          ? [thread.latest]
          : thread.messages,
      ),
    [threads, expandedThreads],
  );

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (screen !== "mailbox") return;
      if (
        settingsOpen ||
        composeDraft ||
        promptState ||
        confirmState ||
        pwPromptState ||
        ctxMenu
      )
        return;
      if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
      const target = e.target as HTMLElement | null;
      const tag = target?.tagName;
      if (
        tag === "INPUT" ||
        tag === "TEXTAREA" ||
        tag === "SELECT" ||
        target?.isContentEditable
      )
        return;
      const list = visibleThreadEnvelopes;
      if (list.length === 0) return;
      e.preventDefault();
      const currentIdx =
        selectedUid === null
          ? -1
          : list.findIndex((env) => env.uid === selectedUid);
      let nextIdx: number;
      if (e.key === "ArrowDown") {
        nextIdx = currentIdx < 0 ? 0 : Math.min(currentIdx + 1, list.length - 1);
      } else {
        nextIdx =
          currentIdx < 0 ? list.length - 1 : Math.max(currentIdx - 1, 0);
      }
      if (nextIdx === currentIdx) return;
      handleSelectMessage(list[nextIdx].uid);
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [
    screen,
    settingsOpen,
    composeDraft,
    promptState,
    confirmState,
    pwPromptState,
    ctxMenu,
    visibleThreadEnvelopes,
    selectedUid,
  ]);

  useEffect(() => {
    if (selectedUid === null) return;
    const el = document.querySelector(
      `.envelope[data-uid="${selectedUid}"]`,
    );
    if (el)
      (el as HTMLElement).scrollIntoView({ block: "nearest" });
  }, [selectedUid]);

  return (
    <>
      {screen === "login" ? (
        <div className="login">
          <form onSubmit={handleConnect}>
            <h1>Mantybird</h1>
            <p className="sub">Add an IMAP account</p>
            <input
              placeholder="Account name (optional)"
              value={account.name}
              onChange={(e) =>
                setAccount({ ...account, name: e.target.value })
              }
            />
            <input
              placeholder="Host (e.g. imap.gmail.com)"
              value={account.host}
              onChange={(e) =>
                setAccount({ ...account, host: e.target.value })
              }
            />
            <input
              placeholder="Port"
              value={account.port}
              onChange={(e) =>
                setAccount({ ...account, port: Number(e.target.value) || 0 })
              }
            />
            <input
              placeholder="Username"
              value={account.username}
              onChange={(e) =>
                setAccount({ ...account, username: e.target.value })
              }
            />
            <input
              placeholder="SMTP host (메일 발송용 — 예: smtp.gmail.com)"
              value={account.smtp_host}
              onChange={(e) =>
                setAccount({ ...account, smtp_host: e.target.value })
              }
            />
            <input
              placeholder="SMTP port (465 = SSL, 587 = STARTTLS)"
              value={account.smtp_port}
              onChange={(e) =>
                setAccount({
                  ...account,
                  smtp_port: Number(e.target.value) || 0,
                })
              }
            />
            <input
              placeholder="Password"
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
            <button type="submit" disabled={connecting}>
              {connecting ? "Connecting…" : "Connect"}
            </button>
            <div className={`status ${error ? "error" : ""}`}>{status}</div>
            <p className="sub">⌘, to open settings</p>
          </form>
        </div>
      ) : (
        <div className="mailbox">
          <header className="topbar">
            <button
              className="primary"
              title="새 메일"
              onClick={() => openCompose()}
            >
              새 메일
            </button>
            <span className="spacer" />
            <input
              className="search-input"
              placeholder="제목 / 보낸이 검색"
              value={searchQuery}
              onChange={(e) => {
                setSearchQuery(e.target.value);
                if (serverResults) setServerResults(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleServerSearch();
              }}
            />
            <button
              onClick={handleServerSearch}
              disabled={
                !searchQuery.trim() || !selectedFolder || searchingServer
              }
              title="서버에서 검색 (제목/본문/보낸이/받는이)"
            >
              {searchingServer ? "검색 중…" : "서버 검색"}
            </button>
            {(searchQuery || serverResults) && (
              <button onClick={clearSearch} title="검색 해제">
                ✕
              </button>
            )}
          </header>
          <div
            className="panes"
            ref={panesRef}
            style={{
              gridTemplateColumns: `${sidebarWidth}px 5px ${listWidth}px 5px 1fr`,
            }}
          >
            <div className="pane">
              {renderAccountTrees()}
            </div>
            <div
              className="pane-resizer"
              onMouseDown={startResize("sidebar")}
              title="드래그하여 너비 조절"
            />
            <div className="pane">
              {threads.map((thread) => {
                const isExpanded = expandedThreads.has(thread.id);
                const visible =
                  thread.messages.length > 1 && !isExpanded
                    ? [thread.latest]
                    : thread.messages;
                return (
                  <div className="thread" key={thread.id}>
                    {visible.map((e, index) => {
                      const isSummary =
                        thread.messages.length > 1 && !isExpanded;
                      const unread = isSummary
                        ? thread.messages.some((message) => !message.seen)
                        : !e.seen;
                      const flagged = isSummary
                        ? thread.messages.some((message) =>
                            message.flags.some(
                              (flag) =>
                                flag.toLowerCase() === "\\flagged",
                            ),
                          )
                        : e.flags.some(
                            (flag) => flag.toLowerCase() === "\\flagged",
                          );
                      return (
                        <div
                          key={e.uid}
                          data-uid={e.uid}
                          className={`envelope ${thread.messages.length > 1 && isExpanded ? "thread-child" : ""} ${selectedUid === e.uid ? "selected" : ""} ${unread ? "unread" : ""}`}
                          onClick={() => handleSelectMessage(e.uid)}
                        >
                          <div className="subject">
                            {thread.messages.length > 1 && index === 0 && (
                              <button
                                type="button"
                                className="thread-toggle"
                                title={isExpanded ? "스레드 접기" : "스레드 펼치기"}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setExpandedThreads((prev) => {
                                    const next = new Set(prev);
                                    if (next.has(thread.id)) next.delete(thread.id);
                                    else next.add(thread.id);
                                    return next;
                                  });
                                }}
                              >
                                {isExpanded ? "▼" : "▶"}
                              </button>
                            )}
                            {unread && <span className="dot" />}
                            {flagged && (
                              <span className="star" title="중요">
                                ★
                              </span>
                            )}
                            {e.subject}
                            {thread.messages.length > 1 && index === 0 && (
                              <span className="thread-count">
                                {thread.messages.length}
                              </span>
                            )}
                          </div>
                          <div className="meta">{e.from}</div>
                          <div className="meta">{e.date}</div>
                        </div>
                      );
                    })}
                  </div>
                );
              })}
              <div ref={sentinelRef} className="sentinel">
                {loadingMore
                  ? "더 불러오는 중…"
                  : noMore
                    ? envelopes.length > 0
                      ? "끝"
                      : ""
                    : ""}
              </div>
            </div>
            <div
              className="pane-resizer"
              onMouseDown={startResize("list")}
              title="드래그하여 너비 조절"
            />
            <div className="pane viewer">
              {loadingBody ? (
                <div className="empty">Loading…</div>
              ) : body ? (
                <>
                  <div className="viewer-head">
                    <h2>{body.subject}</h2>
                    <div className="viewer-actions">
                      <button
                        type="button"
                        onClick={() =>
                          selectedUid !== null && handleToggleFlag(selectedUid)
                        }
                        title="중요 표시"
                      >
                        {envelopes
                          .find((e) => e.uid === selectedUid)
                          ?.flags.some((f) => f.toLowerCase() === "\\flagged")
                          ? "★ 중요"
                          : "☆ 중요"}
                      </button>
                      <button onClick={() => openReply(body)}>답장</button>
                      <button
                        type="button"
                        onClick={() =>
                          selectedUid !== null && handleMoveToTrash(selectedUid)
                        }
                      >
                        삭제
                      </button>
                    </div>
                  </div>
                  <div className="headers">
                    <div>From: {body.from}</div>
                    <div>To: {body.to}</div>
                    <div>Date: {body.date}</div>
                  </div>
                  {body.attachments.length > 0 &&
                    selectedUid !== null &&
                    selectedFolder && (
                      <AttachmentList
                        mailbox={selectedFolder}
                        uid={selectedUid}
                        attachments={body.attachments}
                      />
                    )}
                  {body.html ? (
                    <iframe
                      className="html"
                      sandbox="allow-scripts"
                      srcDoc={wrapHtml(body.subject, body.html)}
                      title="message body"
                    />
                  ) : (
                    <pre className="text">{body.text}</pre>
                  )}
                </>
              ) : (
                <div className="empty">Select a message</div>
              )}
            </div>
          </div>
          <footer className="status">{status}</footer>
        </div>
      )}
      {settingsOpen && (
        <SettingsModal
          currentAccount={account}
          onClose={() => setSettingsOpen(false)}
          onSwitch={handleSwitchAccount}
          onMarkSeenDelayChange={(seconds) => {
            markSeenDelayRef.current = seconds;
            setMarkSeenDelay(seconds);
          }}
        />
      )}
      {composeDraft && (
        <ComposeModal
          draft={composeDraft}
          onChange={setComposeDraft}
          onClose={() => setComposeDraft(null)}
        />
      )}
      {promptState && (
        <PromptDialog
          title={promptState.title}
          label={promptState.label}
          defaultValue={promptState.defaultValue}
          onSubmit={(v) => {
            const r = promptState.resolve;
            setPromptState(null);
            r(v);
          }}
        />
      )}
      {confirmState && (
        <ConfirmDialog
          message={confirmState.message}
          onAnswer={(v) => {
            const r = confirmState.resolve;
            setConfirmState(null);
            r(v);
          }}
        />
      )}
      {pwPromptState && (
        <PasswordPromptDialog
          account={pwPromptState.account}
          message={pwPromptState.message}
          onSubmit={(v) => {
            const r = pwPromptState.resolve;
            setPwPromptState(null);
            r(v);
          }}
        />
      )}
      {ctxMenu && (
        <FolderContextMenu
          x={ctxMenu.x}
          y={ctxMenu.y}
          folder={ctxMenu.folder}
          onNewChild={() => {
            const f = ctxMenu.folder;
            setCtxMenu(null);
            handleCreateMailbox(f ?? undefined);
          }}
          onRename={() => {
            const f = ctxMenu.folder;
            setCtxMenu(null);
            if (f) handleRenameMailbox(f);
          }}
          onDelete={() => {
            const f = ctxMenu.folder;
            setCtxMenu(null);
            if (f) handleDeleteMailbox(f);
          }}
          onSubscribe={() => {
            const f = ctxMenu.folder;
            setCtxMenu(null);
            if (f) handleSubscribe(f);
          }}
          onUnsubscribe={() => {
            const f = ctxMenu.folder;
            setCtxMenu(null);
            if (f) handleUnsubscribe(f);
          }}
        />
      )}
    </>
  );
}

function SettingsModal(props: {
  currentAccount: Account;
  onClose: () => void;
  onSwitch: (acc: Account) => Promise<void>;
  onMarkSeenDelayChange: (seconds: number) => void;
}) {
  const [config, setConfig] = useState<StoredConfig | null>(null);
  const [editing, setEditing] = useState<Account | null>(null);
  const [editPassword, setEditPassword] = useState("");
  const [isNew, setIsNew] = useState(false);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  const [tab, setTab] = useState<"general" | "accounts">("general");

  async function refresh() {
    const c = await api.listAccounts();
    setConfig(c);
  }

  useEffect(() => {
    refresh().catch((e) => console.error("list_accounts failed", e));
  }, []);

  function openAdd() {
    setIsNew(true);
    setEditing(defaultAccount());
    setEditPassword("");
    setMsg("");
  }
  function openEdit(acc: Account) {
    setIsNew(false);
    setEditing(acc);
    api.loadPassword(acc).then((pw) => setEditPassword(pw || ""));
    setMsg("");
  }
  function closeEdit() {
    setEditing(null);
    setEditPassword("");
    setMsg("");
  }

  async function handleSave(e: React.FormEvent) {
    e.preventDefault();
    if (!editing) return;
    if (!editing.host || !editing.username || !editPassword) {
      setMsg("host/username/password 필수");
      return;
    }
    setBusy(true);
    try {
      await api.upsertAccount(editing);
      await api.savePassword(editing, editPassword);
      await refresh();
      closeEdit();
    } catch (err) {
      setMsg(`Save failed: ${err}`);
    } finally {
      setBusy(false);
    }
  }

  async function handleDelete(acc: Account) {
    if (!confirm(`삭제: ${acc.name || accountKey(acc)}?`)) return;
    setBusy(true);
    try {
      await api.deleteAccount(acc);
      await refresh();
    } catch (err) {
      setMsg(`Delete failed: ${err}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="modal-backdrop" onClick={props.onClose}>
      <div
        className="modal settings-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="modal-header">
          <h2>Settings</h2>
          <button onClick={props.onClose}>✕</button>
        </header>
        <div className="modal-tabbed">
          <nav className="modal-tabs">
            <button
              type="button"
              className={`tab ${tab === "general" ? "active" : ""}`}
              onClick={() => {
                setTab("general");
                closeEdit();
              }}
            >
              일반
            </button>
            <button
              type="button"
              className={`tab ${tab === "accounts" ? "active" : ""}`}
              onClick={() => setTab("accounts")}
            >
              계정
            </button>
          </nav>
          {tab === "general" && (
            <div className="modal-body">
              <h3>환경설정</h3>
              <label className="field">
                <span>다운로드 폴더</span>
                <input
                  placeholder="비워두면 ~/Downloads (브라우저 기본)"
                  value={config?.download_dir ?? ""}
                  onChange={(e) =>
                    setConfig((prev) =>
                      prev
                        ? { ...prev, download_dir: e.target.value || null }
                        : prev,
                    )
                  }
                />
              </label>
              <div className="row">
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      const c = await api.setDownloadDir(
                        config?.download_dir || null,
                      );
                      setConfig(c);
                      setMsg("저장됨");
                    } catch (e) {
                      setMsg(`저장 실패: ${e}`);
                    }
                  }}
                  disabled={busy}
                >
                  저장
                </button>
              </div>
              <hr className="modal-divider" />
              <label className="field">
                <span>본문 읽음 처리 지연 (초, 0 = 즉시)</span>
                <input
                  type="number"
                  min={0}
                  placeholder="예: 5"
                  value={config?.mark_seen_delay_seconds ?? 0}
                  onChange={(e) => {
                    const n = Math.max(0, Number(e.target.value) || 0);
                    setConfig((prev) =>
                      prev ? { ...prev, mark_seen_delay_seconds: n } : prev,
                    );
                  }}
                />
              </label>
              <p className="sub">
                본문을 N초 이상 보고 있을 때만 IMAP에 \Seen을 기록합니다.
                0이면 클릭 즉시 처리.
              </p>
              <div className="row">
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      const n = config?.mark_seen_delay_seconds ?? 0;
                      const c = await api.setMarkSeenDelay(n);
                      setConfig(c);
                      props.onMarkSeenDelayChange(c.mark_seen_delay_seconds);
                      setMsg("저장됨");
                    } catch (e) {
                      setMsg(`저장 실패: ${e}`);
                    }
                  }}
                  disabled={busy}
                >
                  저장
                </button>
              </div>
              <hr className="modal-divider" />
              <h3>폴더 라벨</h3>
              <p className="sub">
                특수 용도 폴더(RFC 6154 \Sent, \Drafts 등) 화면 표시 이름.
                IMAP 통신에는 영향 없음.
              </p>
              <FolderLabelsEditor />
              {msg && (
                <div
                  className={`status ${msg.includes("실패") ? "error" : "success"}`}
                >
                  {msg}
                </div>
              )}
            </div>
          )}
          {tab === "accounts" &&
            (editing ? (
              <form className="modal-body" onSubmit={handleSave}>
                <h3>{isNew ? "Add account" : "Edit account"}</h3>
                <input
                  placeholder="Account name (optional)"
                  value={editing.name}
                  onChange={(e) =>
                    setEditing({ ...editing, name: e.target.value })
                  }
                />
                <input
                  placeholder="Host"
                  value={editing.host}
                  onChange={(e) =>
                    setEditing({ ...editing, host: e.target.value })
                  }
                />
                <input
                  placeholder="Port"
                  value={editing.port}
                  onChange={(e) =>
                    setEditing({
                      ...editing,
                      port: Number(e.target.value) || 0,
                    })
                  }
                />
                <input
                  placeholder="Username"
                  value={editing.username}
                  onChange={(e) =>
                    setEditing({ ...editing, username: e.target.value })
                  }
                />
                <input
                  placeholder="SMTP host (e.g. smtp.gmail.com) — for sending mail"
                  value={editing.smtp_host}
                  onChange={(e) =>
                    setEditing({ ...editing, smtp_host: e.target.value })
                  }
                />
                <input
                  placeholder="SMTP port (465 = SSL, 587 = STARTTLS)"
                  value={editing.smtp_port}
                  onChange={(e) =>
                    setEditing({
                      ...editing,
                      smtp_port: Number(e.target.value) || 0,
                    })
                  }
                />
                <input
                  placeholder="Password"
                  type="password"
                  value={editPassword}
                  onChange={(e) => setEditPassword(e.target.value)}
                />
                {msg && <div className="status error">{msg}</div>}
                <div className="row">
                  <button type="submit" disabled={busy}>
                    Save
                  </button>
                  <button
                    type="button"
                    onClick={closeEdit}
                    disabled={busy}
                  >
                    Cancel
                  </button>
                </div>
              </form>
            ) : (
              <div className="modal-body">
                <div className="account-list">
                  {(config?.accounts || []).map((a) => {
                    const isCurrent =
                      config?.current_key === accountKey(a);
                    return (
                      <div className="account-row" key={accountKey(a)}>
                        <div className="info">
                          <div className="name">
                            {a.name || accountKey(a)}
                            {isCurrent && (
                              <span className="badge">현재</span>
                            )}
                          </div>
                          <div className="key">{accountKey(a)}</div>
                        </div>
                        <div className="actions">
                          {!isCurrent && (
                            <button
                              onClick={() => props.onSwitch(a)}
                              disabled={busy}
                            >
                              Switch
                            </button>
                          )}
                          <button
                            onClick={() => openEdit(a)}
                            disabled={busy}
                          >
                            Edit
                          </button>
                          <button
                            onClick={() => handleDelete(a)}
                            disabled={busy}
                          >
                            Delete
                          </button>
                        </div>
                      </div>
                    );
                  })}
                  {(config?.accounts || []).length === 0 && (
                    <div className="empty">No accounts</div>
                  )}
                </div>
                <button onClick={openAdd}>+ Add account</button>
              </div>
            ))}
        </div>
      </div>
    </div>
  );
}

function DebugWindowFrame() {
  return (
    <div
      style={{
        height: "100vh",
        display: "flex",
        flexDirection: "column",
        background: "#1b1b1b",
        color: "#e0e0e0",
        padding: "0.75rem",
        boxSizing: "border-box",
      }}
    >
      <DebugConsole />
    </div>
  );
}

function DebugConsole() {
  const [entries, setEntries] = useState<api.DebugLogEntry[]>([]);
  const [enabled, setEnabled] = useState<Record<string, boolean>>({
    imap: true,
    smtp: true,
    idle: true,
  });
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stickyRef = useRef(true);

  useEffect(() => {
    let alive = true;
    api
      .getDebugLog()
      .then((snap) => {
        if (alive) setEntries(snap);
      })
      .catch((e) => console.warn("getDebugLog failed", e));
    const unlisten = listen<api.DebugLogEntry>("debug:log", (ev) => {
      if (!alive) return;
      setEntries((prev) => {
        const next = prev.length >= 1000 ? prev.slice(-999) : prev.slice();
        next.push(ev.payload);
        return next;
      });
    });
    return () => {
      alive = false;
      unlisten.then((u) => u()).catch(() => {});
    };
  }, []);

  useEffect(() => {
    if (!stickyRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [entries]);

  function onScroll(e: React.UIEvent<HTMLDivElement>) {
    const el = e.currentTarget;
    stickyRef.current = el.scrollTop + el.clientHeight >= el.scrollHeight - 20;
  }

  const visible = entries.filter((e) => enabled[e.channel] ?? true);

  return (
    <div className="modal-body debug-console">
      <h3>Developer Console</h3>
      <div
        style={{
          display: "flex",
          flexWrap: "wrap",
          alignItems: "center",
          gap: "1.25rem",
          marginBottom: "0.25rem",
        }}
      >
        {["imap", "smtp", "idle"].map((ch) => (
          <label
            key={ch}
            style={{
              display: "inline-flex",
              alignItems: "center",
              whiteSpace: "nowrap",
              cursor: "pointer",
              userSelect: "none",
              flexShrink: 0,
            }}
          >
            <input
              type="checkbox"
              checked={enabled[ch] ?? true}
              onChange={(e) =>
                setEnabled((prev) => ({ ...prev, [ch]: e.target.checked }))
              }
              style={{
                width: 14,
                height: 14,
                margin: 0,
                marginRight: 8,
                flexShrink: 0,
              }}
            />
            {ch}
          </label>
        ))}
        <button onClick={() => setEntries([])} style={{ flexShrink: 0 }}>
          화면 비우기
        </button>
        <span style={{ marginLeft: "auto", opacity: 0.7, flexShrink: 0 }}>
          {visible.length} / {entries.length}
        </span>
      </div>
      <div
        ref={scrollRef}
        onScroll={onScroll}
        style={{
          marginTop: "0.5rem",
          height: "55vh",
          overflow: "auto",
          background: "#111",
          color: "#cfd8dc",
          padding: "0.5rem",
          fontFamily: "ui-monospace, Menlo, monospace",
          fontSize: "12px",
          lineHeight: 1.4,
          whiteSpace: "pre-wrap",
          wordBreak: "break-all",
        }}
      >
        {visible.map((e) => {
          const t = new Date(e.ts);
          const hh = String(t.getHours()).padStart(2, "0");
          const mm = String(t.getMinutes()).padStart(2, "0");
          const ss = String(t.getSeconds()).padStart(2, "0");
          const ms = String(t.getMilliseconds()).padStart(3, "0");
          const color =
            e.channel === "imap"
              ? "#80cbc4"
              : e.channel === "smtp"
              ? "#ce93d8"
              : "#ffe082";
          return (
            <div key={e.id}>
              <span style={{ opacity: 0.6 }}>
                {hh}:{mm}:{ss}.{ms}
              </span>
              <span style={{ color, marginLeft: 8 }}>
                [{e.channel}]
              </span>
              <span style={{ marginLeft: 8 }}>{e.direction}</span>
              <span style={{ marginLeft: 8 }}>{e.text}</span>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ComposeModal(props: {
  draft: ComposeDraft;
  onChange: (d: ComposeDraft) => void;
  onClose: () => void;
}) {
  const { draft, onChange } = props;
  const [sending, setSending] = useState(false);
  const [msg, setMsg] = useState("");

  async function handleSend(e: React.FormEvent) {
    e.preventDefault();
    setSending(true);
    setMsg("Sending…");
    try {
      const html = draft.isHtml ? draft.body : null;
      const text = draft.isHtml ? stripHtml(draft.body) : draft.body;
      await api.sendMail({
        to: draft.to,
        cc: draft.cc,
        bcc: draft.bcc,
        subject: draft.subject,
        body: text,
        html,
        attachments: draft.attachments.map((a) => ({
          filename: a.filename,
          mime: a.mime,
          data_base64: a.data_base64,
        })),
        inReplyTo: draft.inReplyTo,
        references: draft.references,
      });
      props.onClose();
    } catch (err) {
      setMsg(`Send failed: ${err}`);
    } finally {
      setSending(false);
    }
  }

  async function handleAttachFiles(files: FileList | null) {
    if (!files || files.length === 0) return;
    const additions: ComposeAttachment[] = [];
    for (const file of Array.from(files)) {
      const buffer = await file.arrayBuffer();
      additions.push({
        filename: file.name,
        mime: file.type || "application/octet-stream",
        data_base64: arrayBufferToBase64(buffer),
        size: file.size,
      });
    }
    onChange({ ...draft, attachments: [...draft.attachments, ...additions] });
  }

  function removeAttachment(i: number) {
    onChange({
      ...draft,
      attachments: draft.attachments.filter((_, j) => j !== i),
    });
  }

  return (
    <div className="modal-backdrop" onClick={props.onClose}>
      <div className="modal compose" onClick={(e) => e.stopPropagation()}>
        <header className="modal-header">
          <h2>새 메일</h2>
          <button onClick={props.onClose}>닫기</button>
        </header>
        <form className="modal-body" onSubmit={handleSend}>
          <input
            placeholder="To (콤마로 여러 명)"
            value={draft.to}
            onChange={(e) => onChange({ ...draft, to: e.target.value })}
          />
          <input
            placeholder="Cc"
            value={draft.cc}
            onChange={(e) => onChange({ ...draft, cc: e.target.value })}
          />
          <input
            placeholder="Bcc"
            value={draft.bcc}
            onChange={(e) => onChange({ ...draft, bcc: e.target.value })}
          />
          <input
            placeholder="Subject"
            value={draft.subject}
            onChange={(e) => onChange({ ...draft, subject: e.target.value })}
          />
          <div className="compose-toolbar">
            <label>
              <input
                type="checkbox"
                checked={draft.isHtml}
                onChange={(e) => {
                  const next = e.target.checked;
                  const body = next
                    ? plainToHtml(draft.body)
                    : stripHtml(draft.body);
                  onChange({ ...draft, isHtml: next, body });
                }}
              />{" "}
              HTML 본문
            </label>
            <label className="file-attach">
              파일 첨부
              <input
                type="file"
                multiple
                onChange={(e) => {
                  handleAttachFiles(e.target.files);
                  e.target.value = "";
                }}
              />
            </label>
          </div>
          {draft.attachments.length > 0 && (
            <div className="attachments">
              {draft.attachments.map((a, i) => (
                <div className="attachment" key={`${a.filename}-${i}`}>
                  <span className="file">
                    {a.filename}{" "}
                    <span className="size">({formatBytes(a.size)})</span>
                  </span>
                  <button type="button" onClick={() => removeAttachment(i)}>
                    삭제
                  </button>
                </div>
              ))}
            </div>
          )}
          {draft.isHtml ? (
            <RichEditor
              value={draft.body}
              onChange={(html) => onChange({ ...draft, body: html })}
            />
          ) : (
            <textarea
              placeholder="본문"
              rows={14}
              value={draft.body}
              onChange={(e) => onChange({ ...draft, body: e.target.value })}
            />
          )}
          {msg && (
            <div
              className={`status ${msg.startsWith("Send failed") ? "error" : ""}`}
            >
              {msg}
            </div>
          )}
          <div className="row">
            <button type="submit" disabled={sending}>
              {sending ? "Sending…" : "Send"}
            </button>
            <button type="button" onClick={props.onClose} disabled={sending}>
              Cancel
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function AttachmentList(props: {
  mailbox: string;
  uid: number;
  attachments: import("./types").AttachmentMeta[];
}) {
  const [busy, setBusy] = useState<number | null>(null);
  const [errorMsg, setErrorMsg] = useState("");
  const [doneMsg, setDoneMsg] = useState("");

  async function download(index: number) {
    setBusy(index);
    setErrorMsg("");
    setDoneMsg("");
    try {
      const r = await api.downloadAttachment(props.mailbox, props.uid, index);
      if (r.saved_path) {
        setDoneMsg(`다운로드 완료 · ${r.saved_path}`);
        api
          .revealInFileManager(r.saved_path)
          .catch((e) => console.warn("reveal failed", e));
      } else {
        saveDownload(r.filename, r.mime, r.data_base64);
        setDoneMsg(`다운로드 완료 · ${r.filename} (~/Downloads)`);
      }
      setTimeout(() => setDoneMsg(""), 5000);
    } catch (err) {
      console.error("download failed", err);
      setErrorMsg(`다운로드 실패: ${err}`);
    } finally {
      setBusy(null);
    }
  }
  return (
    <div className="attachments view">
      <div className="label">첨부파일 ({props.attachments.length})</div>
      {props.attachments.map((a) => (
        <div className="attachment" key={a.index}>
          <span className="file">
            {a.filename}{" "}
            <span className="size">({formatBytes(a.size)})</span>
          </span>
          <button onClick={() => download(a.index)} disabled={busy === a.index}>
            {busy === a.index
              ? `받는 중 ${formatBytes(a.size)}…`
              : "다운로드"}
          </button>
        </div>
      ))}
      {doneMsg && <div className="status success">{doneMsg}</div>}
      {errorMsg && <div className="status error">{errorMsg}</div>}
    </div>
  );
}

function PromptDialog(props: {
  title: string;
  label?: string;
  defaultValue: string;
  onSubmit: (value: string | null) => void;
}) {
  const [value, setValue] = useState(props.defaultValue);
  return (
    <div className="modal-backdrop" onClick={() => props.onSubmit(null)}>
      <div
        className="modal"
        style={{ width: 380 }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="modal-header">
          <h2>{props.title}</h2>
        </header>
        <form
          className="modal-body"
          onSubmit={(e) => {
            e.preventDefault();
            props.onSubmit(value);
          }}
        >
          {props.label && (
            <label className="field">
              <span>{props.label}</span>
            </label>
          )}
          <input
            autoFocus
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <div className="row">
            <button type="submit">확인</button>
            <button type="button" onClick={() => props.onSubmit(null)}>
              취소
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function ConfirmDialog(props: {
  message: string;
  onAnswer: (yes: boolean) => void;
}) {
  return (
    <div className="modal-backdrop" onClick={() => props.onAnswer(false)}>
      <div
        className="modal"
        style={{ width: 380 }}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-body">
          <div style={{ whiteSpace: "pre-wrap" }}>{props.message}</div>
          <div className="row">
            <button autoFocus onClick={() => props.onAnswer(true)}>
              확인
            </button>
            <button onClick={() => props.onAnswer(false)}>취소</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function PasswordPromptDialog(props: {
  account: Account;
  message: string;
  onSubmit: (value: string | null) => void;
}) {
  const [value, setValue] = useState("");
  return (
    <div className="modal-backdrop" onClick={() => props.onSubmit(null)}>
      <div
        className="modal"
        style={{ width: 380 }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="modal-header">
          <h2>비밀번호 입력</h2>
        </header>
        <form
          className="modal-body"
          onSubmit={(e) => {
            e.preventDefault();
            if (!value) return;
            props.onSubmit(value);
          }}
        >
          <div className="status error">{props.message}</div>
          <label className="field">
            <span>
              {props.account.username}@{props.account.host}
            </span>
          </label>
          <input
            autoFocus
            type="password"
            placeholder="비밀번호"
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
          <div className="row">
            <button type="submit" disabled={!value}>
              확인
            </button>
            <button type="button" onClick={() => props.onSubmit(null)}>
              취소
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function isAuthError(err: unknown): boolean {
  const s = String(err);
  return /IMAP login failed|AUTHENTICATIONFAILED|authentication failed|invalid credentials|invalid username|invalid password|wrong password|password.*incorrect|LOGIN failed/i.test(
    s,
  );
}

function FolderContextMenu(props: {
  x: number;
  y: number;
  folder: Folder | null;
  onNewChild: () => void;
  onRename: () => void;
  onDelete: () => void;
  onSubscribe: () => void;
  onUnsubscribe: () => void;
}) {
  const isRoot = props.folder === null;
  const locked = !isRoot && props.folder!.special !== "Other";
  const cannotCreateChild =
    !isRoot && (locked || props.folder!.no_inferiors);
  return (
    <div
      className="ctx-menu"
      style={{ left: props.x, top: props.y }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.stopPropagation()}
    >
      <button onClick={props.onNewChild} disabled={cannotCreateChild}>
        {isRoot ? "새 폴더" : "새 하위 폴더"}
      </button>
      {!isRoot && (
        <>
          <button onClick={props.onRename} disabled={locked}>
            이름 변경
          </button>
          <button onClick={props.onDelete} disabled={locked}>
            삭제
          </button>
          <div className="ctx-sep" />
          {props.folder!.subscribed ? (
            <button onClick={props.onUnsubscribe}>구독 해제</button>
          ) : (
            <button onClick={props.onSubscribe}>구독</button>
          )}
        </>
      )}
    </div>
  );
}

function FolderLabelsEditor() {
  const [labels, setLabels] = useState<Record<string, string>>(
    DEFAULT_FOLDER_LABELS,
  );
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");

  useEffect(() => {
    api
      .getFolderLabels()
      .then((l) => setLabels({ ...DEFAULT_FOLDER_LABELS, ...l }))
      .catch((e) => console.warn("getFolderLabels failed", e));
  }, []);

  const keys: SpecialUse[] = [
    "Inbox",
    "Sent",
    "Drafts",
    "Archive",
    "Junk",
    "Trash",
  ];

  async function save() {
    setBusy(true);
    try {
      await api.setFolderLabels(labels);
      setMsg("저장됨");
      setTimeout(() => setMsg(""), 2000);
    } catch (e) {
      setMsg(`저장 실패: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="folder-labels">
      {keys.map((k) => (
        <label className="field" key={k}>
          <span>{k}</span>
          <input
            value={labels[k] ?? ""}
            onChange={(e) =>
              setLabels((prev) => ({ ...prev, [k]: e.target.value }))
            }
          />
        </label>
      ))}
      <div className="row">
        <button type="button" onClick={save} disabled={busy}>
          저장
        </button>
      </div>
      {msg && (
        <div
          className={`status ${msg.includes("실패") ? "error" : "success"}`}
        >
          {msg}
        </div>
      )}
    </div>
  );
}

function RichEditor(props: {
  value: string;
  onChange: (html: string) => void;
}) {
  const editor = useEditor({
    extensions: [
      StarterKit,
      Underline,
      Link.configure({ openOnClick: false, autolink: true }),
    ],
    content: props.value || "",
    onUpdate: ({ editor }) => {
      props.onChange(editor.getHTML());
    },
  });

  if (!editor) return null;

  function btn(
    label: React.ReactNode,
    isActive: boolean,
    onClick: () => void,
  ) {
    return (
      <button
        type="button"
        className={isActive ? "tool active" : "tool"}
        onMouseDown={(e) => e.preventDefault()}
        onClick={onClick}
      >
        {label}
      </button>
    );
  }

  return (
    <div className="rich-editor">
      <div className="rich-toolbar">
        {btn(
          <b>B</b>,
          editor.isActive("bold"),
          () => editor.chain().focus().toggleBold().run(),
        )}
        {btn(
          <i>I</i>,
          editor.isActive("italic"),
          () => editor.chain().focus().toggleItalic().run(),
        )}
        {btn(
          <u>U</u>,
          editor.isActive("underline"),
          () => editor.chain().focus().toggleUnderline().run(),
        )}
        {btn(
          <s>S</s>,
          editor.isActive("strike"),
          () => editor.chain().focus().toggleStrike().run(),
        )}
        <span className="sep" />
        {btn(
          "• 목록",
          editor.isActive("bulletList"),
          () => editor.chain().focus().toggleBulletList().run(),
        )}
        {btn(
          "1. 번호",
          editor.isActive("orderedList"),
          () => editor.chain().focus().toggleOrderedList().run(),
        )}
        {btn(
          "❝",
          editor.isActive("blockquote"),
          () => editor.chain().focus().toggleBlockquote().run(),
        )}
        {btn(
          "</>",
          editor.isActive("codeBlock"),
          () => editor.chain().focus().toggleCodeBlock().run(),
        )}
        <span className="sep" />
        {btn("링크", editor.isActive("link"), () => {
          const prev = editor.getAttributes("link").href as string | undefined;
          const url = window.prompt("URL", prev ?? "https://");
          if (url === null) return;
          if (url === "") {
            editor.chain().focus().unsetLink().run();
          } else {
            editor.chain().focus().extendMarkRange("link").setLink({ href: url }).run();
          }
        })}
      </div>
      <EditorContent editor={editor} className="rich-content" />
    </div>
  );
}

function saveDownload(filename: string, mime: string, base64: string) {
  const bin = atob(base64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const blob = new Blob([bytes], { type: mime || "application/octet-stream" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename || "attachment";
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

function mergeEnvelopes(first: Envelope[], second: Envelope[]): Envelope[] {
  const seen = new Set<number>();
  const out: Envelope[] = [];
  for (const e of first) {
    if (!seen.has(e.uid)) {
      out.push(e);
      seen.add(e.uid);
    }
  }
  for (const e of second) {
    if (!seen.has(e.uid)) {
      out.push(e);
      seen.add(e.uid);
    }
  }
  return out.sort((a, b) => b.uid - a.uid);
}

interface MailThread {
  id: string;
  latest: Envelope;
  messages: Envelope[];
}

function normalizeMessageId(value: string): string {
  return value.trim().toLowerCase();
}

function appendReference(
  references: string[],
  messageId: string | null,
): string[] {
  const values = messageId ? [...references, messageId] : references;
  return Array.from(
    new Map(
      values
        .map((value) => value.trim())
        .filter(Boolean)
        .map((value) => [normalizeMessageId(value), value]),
    ).values(),
  );
}

function buildThreads(envelopes: Envelope[]): MailThread[] {
  const parent = envelopes.map((_, index) => index);
  const find = (index: number): number => {
    while (parent[index] !== index) {
      parent[index] = parent[parent[index]];
      index = parent[index];
    }
    return index;
  };
  const union = (left: number, right: number) => {
    const leftRoot = find(left);
    const rightRoot = find(right);
    if (leftRoot !== rightRoot) parent[rightRoot] = leftRoot;
  };
  const ownerById = new Map<string, number>();

  envelopes.forEach((envelope, index) => {
    const ids = [
      envelope.message_id,
      envelope.in_reply_to,
      ...envelope.references,
    ]
      .filter((value): value is string => Boolean(value))
      .map(normalizeMessageId)
      .filter(Boolean);
    for (const id of new Set(ids)) {
      const owner = ownerById.get(id);
      if (owner === undefined) ownerById.set(id, index);
      else union(index, owner);
    }
  });

  const grouped = new Map<number, Envelope[]>();
  envelopes.forEach((envelope, index) => {
    const root = find(index);
    const group = grouped.get(root) ?? [];
    group.push(envelope);
    grouped.set(root, group);
  });

  return Array.from(grouped.values())
    .map((messages) => {
      messages.sort((a, b) => a.uid - b.uid);
      const latest = messages[messages.length - 1];
      const id =
        messages
          .flatMap((message) => [
            message.message_id,
            message.in_reply_to,
            ...message.references,
          ])
          .find((value): value is string => Boolean(value)) ?? `uid:${latest.uid}`;
      return { id: normalizeMessageId(id), latest, messages };
    })
    .sort((a, b) => b.latest.uid - a.latest.uid);
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function plainToHtml(text: string): string {
  if (!text) return "";
  const escaped = text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  return escaped
    .split(/\n{2,}/)
    .map((para) => `<p>${para.replace(/\n/g, "<br/>") || "<br/>"}</p>`)
    .join("");
}

function stripHtml(html: string): string {
  return html
    .replace(/<style[\s\S]*?<\/style>/gi, "")
    .replace(/<script[\s\S]*?<\/script>/gi, "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/p>/gi, "\n\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&nbsp;/g, " ")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .trim();
}

function arrayBufferToBase64(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer);
  let binary = "";
  const chunk = 0x8000;
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode.apply(
      null,
      Array.from(bytes.subarray(i, i + chunk)),
    );
  }
  return btoa(binary);
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

function wrapHtml(subject: string, html: string): string {
  const interceptor = `
<script>
(function(){
  document.addEventListener('click', function(ev){
    var node = ev.target;
    while (node && node.nodeType === 1) {
      if (node.tagName === 'A' && node.href) {
        ev.preventDefault();
        try {
          window.parent.postMessage({type: 'manty-open-link', url: node.href}, '*');
        } catch (e) {}
        return;
      }
      node = node.parentNode;
    }
  }, true);
})();
</script>`;
  const trimmed = html.trimStart().toLowerCase();
  const safeTitle = subject.replace(/[<>&]/g, (c) =>
    c === "<" ? "&lt;" : c === ">" ? "&gt;" : "&amp;",
  );
  if (trimmed.startsWith("<!doctype") || trimmed.startsWith("<html")) {
    // Inject interceptor before </body> or as fallback append.
    if (/<\/body>/i.test(html)) {
      return html.replace(/<\/body>/i, `${interceptor}</body>`);
    }
    return `${html}${interceptor}`;
  }
  return `<!doctype html><html><head><meta charset="utf-8"><title>${safeTitle}</title></head><body>${html}${interceptor}</body></html>`;
}
