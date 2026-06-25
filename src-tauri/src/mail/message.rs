use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpecialUse {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    Other,
}

impl SpecialUse {
    pub fn priority(self) -> u8 {
        match self {
            Self::Inbox => 0,
            Self::Sent => 1,
            Self::Drafts => 2,
            Self::Archive => 3,
            Self::Junk => 4,
            Self::Trash => 5,
            Self::Other => 6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub raw: String,
    pub leaf: String,
    pub depth: u16,
    pub parent_path: Option<String>,
    pub has_children: bool,
    pub special: SpecialUse,
    #[serde(default = "default_true")]
    pub subscribed: bool,
    #[serde(default)]
    pub unread_count: u32,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub uid: u32,
    pub subject: String,
    pub from: String,
    pub date: String,
    pub flags: Vec<String>,
    pub seen: bool,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub index: usize,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageBody {
    pub subject: String,
    pub from: String,
    pub to: String,
    pub date: String,
    pub text: String,
    pub html: Option<String>,
    #[serde(default)]
    pub attachments: Vec<AttachmentMeta>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
}
