import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  Envelope,
  Folder,
  MessageBody,
  StoredConfig,
} from "./types";

export function connectImap(
  host: string,
  port: number,
  username: string,
  password: string,
): Promise<Folder[]> {
  return invoke("connect_imap", { host, port, username, password });
}

export function disconnectImap(): Promise<void> {
  return invoke("disconnect_imap");
}

export function fetchEnvelopes(
  mailbox: string,
  offset: number,
  limit: number,
): Promise<Envelope[]> {
  return invoke("fetch_envelopes", { mailbox, offset, limit });
}

export function fetchBody(
  mailbox: string,
  uid: number,
): Promise<MessageBody> {
  return invoke("fetch_body", { mailbox, uid });
}

export function pruneDeleted(
  mailbox: string,
  uids: number[],
): Promise<number[]> {
  return invoke("prune_deleted", { mailbox, uids });
}

export function deletePermanent(mailbox: string, uid: number): Promise<void> {
  return invoke("delete_permanent", { mailbox, uid });
}

export interface DebugLogEntry {
  id: number;
  ts: number;
  channel: string;
  direction: string;
  text: string;
}

export function getDebugLog(): Promise<DebugLogEntry[]> {
  return invoke("get_debug_log");
}

export function searchMailbox(
  mailbox: string,
  query: string,
  limit: number,
): Promise<Envelope[]> {
  return invoke("search_mailbox", { mailbox, query, limit });
}

export interface DownloadResult {
  filename: string;
  mime: string;
  data_base64: string;
  saved_path: string | null;
}

export function getDownloadDir(): Promise<string | null> {
  return invoke("get_download_dir");
}

export function openUrl(url: string): Promise<void> {
  return invoke("open_url", { url });
}

export function revealInFileManager(path: string): Promise<void> {
  return invoke("reveal_in_file_manager", { path });
}

export function setDownloadDir(dir: string | null): Promise<StoredConfig> {
  return invoke("set_download_dir", { dir });
}

export function getMarkSeenDelay(): Promise<number> {
  return invoke("get_mark_seen_delay");
}

export function setMarkSeenDelay(seconds: number): Promise<StoredConfig> {
  return invoke("set_mark_seen_delay", { seconds });
}

export function getFolderLabels(): Promise<Record<string, string>> {
  return invoke("get_folder_labels");
}

export function setFolderLabels(
  labels: Record<string, string>,
): Promise<StoredConfig> {
  return invoke("set_folder_labels", { labels });
}

export function getLastMailbox(): Promise<string | null> {
  return invoke("get_last_mailbox");
}

export function setLastMailbox(mailbox: string | null): Promise<StoredConfig> {
  return invoke("set_last_mailbox", { mailbox });
}

export function getExpandedFolders(): Promise<string[]> {
  return invoke("get_expanded_folders");
}

export function setExpandedFolders(paths: string[]): Promise<StoredConfig> {
  return invoke("set_expanded_folders", { paths });
}

export function downloadAttachment(
  mailbox: string,
  uid: number,
  index: number,
): Promise<DownloadResult> {
  return invoke("download_attachment", { mailbox, uid, index });
}

export function markSeen(
  mailbox: string,
  uid: number,
  seen: boolean,
): Promise<string[]> {
  return invoke("mark_seen", { mailbox, uid, seen });
}

export function setFlag(
  mailbox: string,
  uid: number,
  flag: string,
  on: boolean,
): Promise<string[]> {
  return invoke("set_flag", { mailbox, uid, flag, on });
}

export function moveToTrash(mailbox: string, uid: number): Promise<void> {
  return invoke("move_to_trash", { mailbox, uid });
}

export function cachedFolders(account: Account): Promise<Folder[]> {
  return invoke("cached_folders", { account });
}

export function cachedEnvelopes(
  account: Account,
  mailbox: string,
  beforeUid: number | null,
  limit: number,
): Promise<Envelope[]> {
  return invoke("cached_envelopes", {
    account,
    mailbox,
    beforeUid,
    limit,
  });
}

export function cachedBody(
  account: Account,
  mailbox: string,
  uid: number,
): Promise<MessageBody | null> {
  return invoke("cached_body", { account, mailbox, uid });
}

export function listAccounts(): Promise<StoredConfig> {
  return invoke("list_accounts");
}

export function currentAccount(): Promise<Account | null> {
  return invoke("current_account");
}

export function upsertAccount(account: Account): Promise<StoredConfig> {
  return invoke("upsert_account", { account });
}

export function deleteAccount(account: Account): Promise<StoredConfig> {
  return invoke("delete_account", { account });
}

export function setCurrentAccount(account: Account): Promise<StoredConfig> {
  return invoke("set_current_account", { account });
}

export function savePassword(account: Account, password: string): Promise<void> {
  return invoke("save_password", { account, password });
}

export function loadPassword(account: Account): Promise<string | null> {
  return invoke("load_password", { account });
}

export function createMailbox(
  name: string,
  parentRaw: string | null,
): Promise<Folder[]> {
  return invoke("create_mailbox", { parentRaw, name });
}

export function renameMailbox(
  fromRaw: string,
  newLeaf: string,
): Promise<Folder[]> {
  return invoke("rename_mailbox", { fromRaw, newLeaf });
}

export function deleteMailbox(raw: string): Promise<Folder[]> {
  return invoke("delete_mailbox", { raw });
}

export function subscribeMailbox(raw: string): Promise<Folder[]> {
  return invoke("subscribe_mailbox", { raw });
}

export function unsubscribeMailbox(raw: string): Promise<Folder[]> {
  return invoke("unsubscribe_mailbox", { raw });
}

export interface AttachmentPayload {
  filename: string;
  mime: string;
  data_base64: string;
}

export function sendMail(args: {
  to: string;
  cc: string;
  bcc: string;
  subject: string;
  body: string;
  html: string | null;
  attachments: AttachmentPayload[];
}): Promise<void> {
  return invoke("send_mail", args);
}
