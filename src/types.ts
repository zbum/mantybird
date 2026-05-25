export type SpecialUse =
  | "Inbox"
  | "Sent"
  | "Drafts"
  | "Archive"
  | "Junk"
  | "Trash"
  | "Other";

export interface Account {
  name: string;
  host: string;
  port: number;
  username: string;
  smtp_host: string;
  smtp_port: number;
}

export interface StoredConfig {
  accounts: Account[];
  current_key: string | null;
  download_dir: string | null;
  folder_labels: Record<string, string> | null;
}

export const DEFAULT_FOLDER_LABELS: Record<SpecialUse, string> = {
  Inbox: "받은 메일함",
  Sent: "보낸 메일함",
  Drafts: "임시 보관함",
  Archive: "보관 메일함",
  Junk: "스팸 메일함",
  Trash: "휴지통",
  Other: "",
};

export function accountKey(a: Account): string {
  return `${a.username}@${a.host}:${a.port}`;
}

export interface Folder {
  raw: string;
  leaf: string;
  depth: number;
  parent_path: string | null;
  has_children: boolean;
  special: SpecialUse;
  subscribed: boolean;
  unread_count: number;
}

export interface Envelope {
  uid: number;
  subject: string;
  from: string;
  date: string;
  flags: string[];
  seen: boolean;
}

export interface AttachmentMeta {
  index: number;
  filename: string;
  mime: string;
  size: number;
  content_id: string | null;
  is_inline: boolean;
}

export interface MessageBody {
  subject: string;
  from: string;
  to: string;
  date: string;
  text: string;
  html: string | null;
  attachments: AttachmentMeta[];
}
