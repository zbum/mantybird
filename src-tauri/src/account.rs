use serde::{Deserialize, Serialize};

fn default_smtp_port() -> u16 {
    465
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
}

impl Default for Account {
    fn default() -> Self {
        Self {
            name: String::new(),
            host: String::new(),
            port: 993,
            username: String::new(),
            smtp_host: String::new(),
            smtp_port: 465,
        }
    }
}

