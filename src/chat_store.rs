use std::fs;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[cfg(all(feature = "sqlite", feature = "rocksdb"))]
compile_error!("features `sqlite` and `rocksdb` are mutually exclusive");
#[cfg(not(any(feature = "sqlite", feature = "rocksdb")))]
compile_error!("enable one chat storage feature: `sqlite` or `rocksdb`");

#[cfg(feature = "rocksdb")]
use fs4::FileExt;
#[cfg(feature = "rocksdb")]
use rocksdb::{DB, IteratorMode};
#[cfg(feature = "sqlite")]
use rusqlite::Connection;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedChat {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub entries: Vec<SavedEntry>,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub config: ChatConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatConfig {
    pub model: String,
    pub system: String,
    pub context: u32,
    pub temperature: Option<f32>,
    pub markdown: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            model: String::new(),
            system: String::new(),
            context: 0,
            temperature: None,
            markdown: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SavedEntry {
    User(String),
    Assistant {
        variants: Vec<SavedReply>,
        selected: usize,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedReply {
    pub model: String,
    pub content: String,
    pub thinking: String,
    pub stats: Option<crate::ollama::GenerateStats>,
    pub error: Option<String>,
    pub stopped: bool,
}

#[derive(Clone, Debug)]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub updated: String,
    pub group: String,
}

pub struct ChatStore {
    path: PathBuf,
}

impl ChatStore {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        let data_dir = path.parent().context("chats.db has no parent folder")?;
        fs::create_dir_all(data_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(feature = "rocksdb")]
        if !path.exists() {
            let previous = data_dir.join("chats.db");
            if previous.is_dir() {
                fs::rename(previous, &path).context("preserving existing RocksDB chat history")?;
            }
        }
        #[cfg(feature = "sqlite")]
        if path.is_dir() {
            let backup = path.with_file_name("chats-rocksdb");
            anyhow::ensure!(
                !backup.exists(),
                "legacy RocksDB chat history exists at {}; enable `rocksdb` to access it",
                backup.display()
            );
            fs::rename(&path, &backup).context("preserving existing RocksDB chat history")?;
        }
        let store = Self { path };
        #[cfg(feature = "rocksdb")]
        store.with_rocksdb(|_| Ok(()))?;
        #[cfg(feature = "sqlite")]
        store.with_sqlite(|_| Ok(()))?;
        Ok(store)
    }

    pub fn list(&self) -> anyhow::Result<Vec<ChatSummary>> {
        #[cfg(feature = "rocksdb")]
        return self.with_rocksdb(|db| {
            let mut chats = Vec::new();
            for item in db.iterator(IteratorMode::Start) {
                let (key, value) = item?;
                if key.starts_with(CHAT_PREFIX) {
                    let chat: SavedChat = serde_json::from_slice(&value)?;
                    chats.push(ChatSummary {
                        id: chat.id,
                        title: chat.title,
                        updated: chat.updated,
                        group: chat.group,
                    });
                }
            }
            chats.sort_by(|a, b| b.updated.cmp(&a.updated));
            Ok(chats)
        });
        #[cfg(feature = "sqlite")]
        self.with_sqlite(|db| {
            let mut stmt = db.prepare("SELECT value FROM chats ORDER BY updated DESC")?;
            Ok(stmt
                .query_map([], |row| row.get::<_, Vec<u8>>(0))?
                .map(|value| {
                    let chat: SavedChat = serde_json::from_slice(&value?)?;
                    Ok(ChatSummary {
                        id: chat.id,
                        title: chat.title,
                        updated: chat.updated,
                        group: chat.group,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?)
        })
    }

    pub fn load(&self, id: &str) -> anyhow::Result<SavedChat> {
        #[cfg(feature = "rocksdb")]
        return self.with_rocksdb(|db| {
            let value = db.get(chat_key(id))?.context("saved chat was not found")?;
            Ok(serde_json::from_slice(&value)?)
        });
        #[cfg(feature = "sqlite")]
        self.with_sqlite(|db| {
            let value: Vec<u8> = db
                .query_row("SELECT value FROM chats WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .context("saved chat was not found")?;
            Ok(serde_json::from_slice(&value)?)
        })
    }

    pub fn save(&self, chat: &SavedChat) -> anyhow::Result<()> {
        let value = serde_json::to_vec(chat)?;
        #[cfg(feature = "rocksdb")]
        return self.with_rocksdb(|db| {
            let key = chat_key(&chat.id);
            if let Some(old) = db.get(&key)? {
                let old: SavedChat = serde_json::from_slice(&old)?;
                if old.updated > chat.updated {
                    return Ok(());
                }
            }
            db.put(key, value).map_err(Into::into)
        });
        #[cfg(feature = "sqlite")]
        self.with_sqlite(|db| {
            db.execute(
                "INSERT INTO chats (id, title, updated, value) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(id) DO UPDATE SET title = excluded.title, updated = excluded.updated, value = excluded.value \
                 WHERE chats.updated <= excluded.updated",
                rusqlite::params![chat.id, chat.title, chat.updated, value],
            )?;
            Ok(())
        })
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<()> {
        #[cfg(feature = "rocksdb")]
        return self.with_rocksdb(|db| db.delete(chat_key(id)).map_err(Into::into));
        #[cfg(feature = "sqlite")]
        self.with_sqlite(|db| {
            db.execute("DELETE FROM chats WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    #[cfg(feature = "rocksdb")]
    fn with_rocksdb<T>(&self, f: impl FnOnce(&DB) -> anyhow::Result<T>) -> anyhow::Result<T> {
        let lock_path = self.path.with_extension("db.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)?;
        lock.lock_exclusive()?;
        let db = DB::open_default(&self.path)?;
        let result = f(&db);
        drop(db);
        let _ = FileExt::unlock(&lock);
        result
    }

    #[cfg(feature = "sqlite")]
    fn with_sqlite<T>(
        &self,
        f: impl FnOnce(&Connection) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        let db = Connection::open(&self.path)?;
        db.busy_timeout(std::time::Duration::from_secs(5))?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS chats (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 updated TEXT NOT NULL,
                 value BLOB NOT NULL
             );",
        )?;
        f(&db)
    }
}

#[cfg(feature = "rocksdb")]
fn chat_key(id: &str) -> Vec<u8> {
    [CHAT_PREFIX, id.as_bytes()].concat()
}
