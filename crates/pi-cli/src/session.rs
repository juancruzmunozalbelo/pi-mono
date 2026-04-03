use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use pi_ai::Message;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub model: String,
    pub messages: Vec<Message>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn sessions_dir() -> PathBuf {
    crate::config::config_dir().join("sessions")
}

#[allow(dead_code)]
pub fn save_session(session: &Session) -> anyhow::Result<()> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[allow(dead_code)]
pub fn load_session(id: &str) -> anyhow::Result<Option<Session>> {
    let path = sessions_dir().join(format!("{id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

pub fn list_sessions() -> anyhow::Result<Vec<Session>> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut sessions = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            let content = std::fs::read_to_string(&path)?;
            if let Ok(session) = serde_json::from_str::<Session>(&content) {
                sessions.push(session);
            }
        }
    }

    // Sort by updated_at descending
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    Ok(sessions)
}
