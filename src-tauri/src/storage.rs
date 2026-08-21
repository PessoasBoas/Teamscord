use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::access::Role;
use crate::protocol::MessageEnvelope;

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let exists = connection
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|error| format!("não foi possível inspecionar migração: {error}"))?
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("não foi possível inspecionar migração: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("não foi possível inspecionar migração: {error}"))?
        .into_iter()
        .any(|name| name == column);
    if !exists {
        connection
            .execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )
            .map_err(|error| format!("não foi possível atualizar tabela {table}: {error}"))?;
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupRecord {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub color: String,
    pub owner_peer_id: String,
    pub created_at: i64,
    pub current_key_epoch: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub kind: String,
    pub position: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelPermissionRecord {
    pub group_id: String,
    pub channel_id: String,
    pub role: Role,
    pub can_join_voice: bool,
    pub can_share_screen: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberRecord {
    pub group_id: String,
    pub peer_id: String,
    pub display_name: String,
    pub role: Role,
    pub public_key: Vec<u8>,
    pub x25519_public_key: Vec<u8>,
    pub status: String,
    pub joined_at: i64,
    pub timeout_until: Option<i64>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEventRecord {
    pub event_id: String,
    pub group_id: String,
    pub key_epoch: i64,
    pub issuer_peer_id: String,
    pub kind: String,
    pub target_peer_id: Option<String>,
    pub payload: serde_json::Value,
    pub logical_timestamp: i64,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerAddressRecord {
    pub peer_id: String,
    pub address: String,
    pub source: String,
    pub last_seen: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxRecord {
    pub event_id: String,
    pub kind: String,
    pub payload: String,
    pub created_at: i64,
    pub attempts: i64,
    pub last_attempt_at: Option<i64>,
}

pub struct Database {
    connection: Mutex<Connection>,
}

fn member_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemberRecord> {
    let role: String = row.get(3)?;
    Ok(MemberRecord {
        group_id: row.get(0)?,
        peer_id: row.get(1)?,
        display_name: row.get(2)?,
        role: Role::parse(&role).unwrap_or(Role::Member),
        public_key: row.get(4)?,
        x25519_public_key: row.get(5)?,
        status: row.get(6)?,
        joined_at: row.get(7)?,
        timeout_until: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

impl Database {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let connection = Connection::open(path)
            .map_err(|error| format!("não foi possível abrir banco local: {error}"))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("não foi possível configurar banco: {error}"))?;
        let database = Self {
            connection: Mutex::new(connection),
        };
        database.migrate()?;
        Ok(database)
    }

    pub fn migrate(&self) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS groups (
               id TEXT PRIMARY KEY,
               name TEXT NOT NULL,
               initials TEXT NOT NULL,
               color TEXT NOT NULL,
               owner_peer_id TEXT NOT NULL,
               created_at INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS channels (
               id TEXT PRIMARY KEY,
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               name TEXT NOT NULL,
               kind TEXT NOT NULL,
               position INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
               event_id TEXT PRIMARY KEY,
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               channel_id TEXT NOT NULL,
               author_peer_id TEXT NOT NULL,
               author_public_key BLOB NOT NULL,
               author_name TEXT NOT NULL,
               initials TEXT NOT NULL,
               color TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               key_epoch INTEGER NOT NULL DEFAULT 1,
               nonce BLOB NOT NULL,
               ciphertext BLOB NOT NULL,
               signature BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages(group_id, channel_id, created_at, event_id);
             CREATE TABLE IF NOT EXISTS peers (
               peer_id TEXT PRIMARY KEY,
               last_seen INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS peer_addresses (
               peer_id TEXT NOT NULL,
               address TEXT NOT NULL,
               source TEXT NOT NULL,
               last_seen INTEGER NOT NULL,
               PRIMARY KEY(peer_id, address)
             );
             CREATE INDEX IF NOT EXISTS idx_peer_addresses_seen ON peer_addresses(last_seen DESC);
             CREATE TABLE IF NOT EXISTS outbox (
               event_id TEXT PRIMARY KEY,
               kind TEXT NOT NULL,
               payload TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               attempts INTEGER NOT NULL DEFAULT 0,
               last_attempt_at INTEGER
             );
             CREATE INDEX IF NOT EXISTS idx_outbox_kind ON outbox(kind, created_at, event_id);
             CREATE TABLE IF NOT EXISTS sync_cursors (
               peer_id TEXT NOT NULL,
               group_id TEXT NOT NULL,
               channel_id TEXT NOT NULL,
               last_created_at INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY(peer_id, group_id, channel_id)
             );
             CREATE TABLE IF NOT EXISTS group_members (
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               peer_id TEXT NOT NULL,
               display_name TEXT NOT NULL,
               role TEXT NOT NULL,
               public_key BLOB NOT NULL,
               x25519_public_key BLOB NOT NULL,
               status TEXT NOT NULL DEFAULT 'active',
               joined_at INTEGER NOT NULL,
               timeout_until INTEGER,
               updated_at INTEGER NOT NULL,
               PRIMARY KEY(group_id, peer_id)
             );
             CREATE INDEX IF NOT EXISTS idx_members_group ON group_members(group_id, status);
             CREATE TABLE IF NOT EXISTS audit_events (
               event_id TEXT PRIMARY KEY,
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               key_epoch INTEGER NOT NULL,
               issuer_peer_id TEXT NOT NULL,
               kind TEXT NOT NULL,
               target_peer_id TEXT,
               payload TEXT NOT NULL,
               logical_timestamp INTEGER NOT NULL,
               signature BLOB NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_audit_group ON audit_events(group_id, logical_timestamp, event_id);
             CREATE TABLE IF NOT EXISTS key_epochs (
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               epoch INTEGER NOT NULL,
               created_by TEXT NOT NULL,
               created_at INTEGER NOT NULL,
               key_hash BLOB NOT NULL,
               PRIMARY KEY(group_id, epoch)
             );
             CREATE TABLE IF NOT EXISTS key_shares (
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               epoch INTEGER NOT NULL,
               recipient_peer_id TEXT NOT NULL,
               recipient_public_key BLOB NOT NULL,
               nonce BLOB NOT NULL,
               ciphertext BLOB NOT NULL,
               signature BLOB NOT NULL,
               PRIMARY KEY(group_id, epoch, recipient_peer_id)
             );
             CREATE TABLE IF NOT EXISTS channel_permissions (
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
               role TEXT NOT NULL,
               can_join_voice INTEGER NOT NULL DEFAULT 1,
               can_share_screen INTEGER NOT NULL DEFAULT 1,
               PRIMARY KEY(channel_id, role)
             );
             CREATE TABLE IF NOT EXISTS message_deletions (
               message_id TEXT PRIMARY KEY,
               group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
               channel_id TEXT NOT NULL,
               deleted_by TEXT NOT NULL,
               deleted_at INTEGER NOT NULL
             );",
        ).map_err(|error| format!("migração SQLite falhou: {error}"))
        .and_then(|_| {
             ensure_column(&connection, "groups", "current_key_epoch", "INTEGER NOT NULL DEFAULT 1")?;
             ensure_column(&connection, "messages", "key_epoch", "INTEGER NOT NULL DEFAULT 1")?;
             ensure_column(&connection, "sync_cursors", "last_event_id", "TEXT NOT NULL DEFAULT ''")?;
             connection
                 .execute("UPDATE channels SET kind = 'voice' WHERE kind = 'screen'", [])
                 .map_err(|error| format!("migração de canais falhou: {error}"))?;
             Ok(())
        })
    }

    pub fn insert_group(&self, group: &GroupRecord) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection.execute("INSERT OR IGNORE INTO groups (id, name, initials, color, owner_peer_id, created_at, current_key_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)", params![group.id, group.name, group.initials, group.color, group.owner_peer_id, group.created_at, group.current_key_epoch]).map_err(|error| format!("não foi possível salvar grupo: {error}"))?;
        Ok(())
    }

    pub fn insert_default_channels(&self, group_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let channels = [
            ("geral", "geral", "text", 0),
            ("ideias", "ideias", "text", 1),
            ("design", "design", "text", 2),
            ("lounge", "lounge", "voice", 3),
            ("pairing", "pairing", "voice", 4),
        ];
        for (id, name, kind, position) in channels {
            let full_id = format!("{group_id}:{id}");
            connection.execute("INSERT OR IGNORE INTO channels (id, group_id, name, kind, position) VALUES (?1, ?2, ?3, ?4, ?5)", params![full_id, group_id, name, kind, position]).map_err(|error| format!("não foi possível salvar canal: {error}"))?;
        }
        Ok(())
    }

    pub fn list_groups(&self) -> Result<Vec<GroupRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection.prepare("SELECT id, name, initials, color, owner_peer_id, created_at, current_key_epoch FROM groups ORDER BY created_at ASC").map_err(|error| format!("não foi possível ler grupos: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(GroupRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    initials: row.get(2)?,
                    color: row.get(3)?,
                    owner_peer_id: row.get(4)?,
                    created_at: row.get(5)?,
                    current_key_epoch: row.get(6)?,
                })
            })
            .map_err(|error| format!("não foi possível ler grupos: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler grupos: {error}"))
    }

    pub fn get_group(&self, group_id: &str) -> Result<Option<GroupRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT id, name, initials, color, owner_peer_id, created_at, current_key_epoch FROM groups WHERE id = ?1",
                [group_id],
                |row| {
                    Ok(GroupRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        initials: row.get(2)?,
                        color: row.get(3)?,
                        owner_peer_id: row.get(4)?,
                        created_at: row.get(5)?,
                        current_key_epoch: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("não foi possível ler grupo: {error}"))
    }

    pub fn list_channels(&self, group_id: &str) -> Result<Vec<ChannelRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection.prepare("SELECT id, group_id, name, kind, position FROM channels WHERE group_id = ?1 ORDER BY position ASC").map_err(|error| format!("não foi possível ler canais: {error}"))?;
        let rows = statement
            .query_map([group_id], |row| {
                Ok(ChannelRecord {
                    id: row.get(0)?,
                    group_id: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    position: row.get(4)?,
                })
            })
            .map_err(|error| format!("não foi possível ler canais: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler canais: {error}"))
    }

    pub fn insert_channel(&self, channel: &ChannelRecord) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let kind = if channel.kind == "screen" {
            "voice"
        } else {
            channel.kind.as_str()
        };
        connection
            .execute(
                "INSERT OR IGNORE INTO channels (id, group_id, name, kind, position) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![channel.id, channel.group_id, channel.name, kind, channel.position],
            )
            .map_err(|error| format!("não foi possível salvar canal: {error}"))?;
        Ok(())
    }

    pub fn update_channel(&self, channel: &ChannelRecord) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let kind = if channel.kind == "screen" {
            "voice"
        } else {
            channel.kind.as_str()
        };
        connection
            .execute(
                "UPDATE channels SET name = ?3, kind = ?4, position = ?5 WHERE id = ?1 AND group_id = ?2",
                params![channel.id, channel.group_id, channel.name, kind, channel.position],
            )
            .map_err(|error| format!("não foi possível atualizar canal: {error}"))?;
        Ok(())
    }

    pub fn delete_channel(&self, group_id: &str, channel_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "DELETE FROM channels WHERE group_id = ?1 AND id = ?2",
                params![group_id, channel_id],
            )
            .map_err(|error| format!("não foi possível excluir canal: {error}"))?;
        Ok(())
    }

    pub fn reorder_channels(&self, group_id: &str, channel_ids: &[String]) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        for (position, channel_id) in channel_ids.iter().enumerate() {
            connection
                .execute(
                    "UPDATE channels SET position = ?3 WHERE group_id = ?1 AND id = ?2",
                    params![group_id, channel_id, position as i32],
                )
                .map_err(|error| format!("não foi possível reorganizar canais: {error}"))?;
        }
        Ok(())
    }

    pub fn get_channel_permission(
        &self,
        channel_id: &str,
        role: &Role,
    ) -> Result<Option<ChannelPermissionRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT group_id, channel_id, role, can_join_voice, can_share_screen FROM channel_permissions WHERE channel_id = ?1 AND role = ?2",
                params![channel_id, role.as_str()],
                |row| {
                    Ok(ChannelPermissionRecord {
                        group_id: row.get(0)?,
                        channel_id: row.get(1)?,
                        role: Role::parse(&row.get::<_, String>(2)?).unwrap_or(Role::Member),
                        can_join_voice: row.get::<_, i64>(3)? != 0,
                        can_share_screen: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("não foi possível ler permissão do canal: {error}"))
    }

    pub fn list_channel_permissions(
        &self,
        group_id: &str,
        channel_id: &str,
    ) -> Result<Vec<ChannelPermissionRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT group_id, channel_id, role, can_join_voice, can_share_screen FROM channel_permissions WHERE group_id = ?1 AND channel_id = ?2 ORDER BY role")
            .map_err(|error| format!("não foi possível ler permissões do canal: {error}"))?;
        let rows = statement
            .query_map(params![group_id, channel_id], |row| {
                Ok(ChannelPermissionRecord {
                    group_id: row.get(0)?,
                    channel_id: row.get(1)?,
                    role: Role::parse(&row.get::<_, String>(2)?).unwrap_or(Role::Member),
                    can_join_voice: row.get::<_, i64>(3)? != 0,
                    can_share_screen: row.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(|error| format!("não foi possível ler permissões do canal: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler permissões do canal: {error}"))
    }

    pub fn set_channel_permission(
        &self,
        permission: &ChannelPermissionRecord,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT INTO channel_permissions (group_id, channel_id, role, can_join_voice, can_share_screen) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(channel_id, role) DO UPDATE SET group_id = excluded.group_id, can_join_voice = excluded.can_join_voice, can_share_screen = excluded.can_share_screen",
                params![
                    permission.group_id,
                    permission.channel_id,
                    permission.role.as_str(),
                    permission.can_join_voice,
                    permission.can_share_screen,
                ],
            )
            .map_err(|error| format!("não foi possível salvar permissão do canal: {error}"))?;
        Ok(())
    }

    pub fn insert_message(&self, message: &MessageEnvelope) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let inserted = connection.execute("INSERT OR IGNORE INTO messages (event_id, group_id, channel_id, author_peer_id, author_public_key, author_name, initials, color, created_at, key_epoch, nonce, ciphertext, signature) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)", params![message.event_id, message.group_id, message.channel_id, message.author_peer_id, message.author_public_key, message.author_name, message.initials, message.color, message.created_at, message.key_epoch, message.nonce, message.ciphertext, message.signature]).map_err(|error| format!("não foi possível salvar mensagem: {error}"))?;
        Ok(inserted > 0)
    }

    pub fn get_message(&self, event_id: &str) -> Result<Option<MessageEnvelope>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT event_id, group_id, channel_id, author_peer_id, author_public_key, author_name, initials, color, created_at, key_epoch, nonce, ciphertext, signature FROM messages WHERE event_id = ?1",
                params![event_id],
                |row| {
                    Ok(MessageEnvelope {
                        event_id: row.get(0)?,
                        group_id: row.get(1)?,
                        channel_id: row.get(2)?,
                        author_peer_id: row.get(3)?,
                        author_public_key: row.get(4)?,
                        author_name: row.get(5)?,
                        initials: row.get(6)?,
                        color: row.get(7)?,
                        created_at: row.get(8)?,
                        key_epoch: row.get(9)?,
                        nonce: row.get(10)?,
                        ciphertext: row.get(11)?,
                        signature: row.get(12)?,
                    })
                },
            )
            .optional()
            .map_err(|error| format!("não foi possível ler mensagem: {error}"))
    }

    pub fn delete_message(
        &self,
        message_id: &str,
        group_id: &str,
        channel_id: &str,
        deleted_by: &str,
        deleted_at: i64,
    ) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO message_deletions (message_id, group_id, channel_id, deleted_by, deleted_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![message_id, group_id, channel_id, deleted_by, deleted_at],
            )
            .map_err(|error| format!("não foi possível registrar exclusão da mensagem: {error}"))?;
        Ok(inserted > 0)
    }

    pub fn is_message_deleted(&self, message_id: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM message_deletions WHERE message_id = ?1)",
                params![message_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("não foi possível verificar exclusão da mensagem: {error}"))
    }

    pub fn list_messages(
        &self,
        group_id: &str,
        channel_id: &str,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection.prepare("SELECT event_id, group_id, channel_id, author_peer_id, author_public_key, author_name, initials, color, created_at, key_epoch, nonce, ciphertext, signature FROM messages WHERE group_id = ?1 AND channel_id = ?2 AND NOT EXISTS (SELECT 1 FROM message_deletions WHERE message_id = messages.event_id) ORDER BY created_at ASC, event_id ASC LIMIT ?3").map_err(|error| format!("não foi possível ler mensagens: {error}"))?;
        let rows = statement
            .query_map(params![group_id, channel_id, limit], |row| {
                Ok(MessageEnvelope {
                    event_id: row.get(0)?,
                    group_id: row.get(1)?,
                    channel_id: row.get(2)?,
                    author_peer_id: row.get(3)?,
                    author_public_key: row.get(4)?,
                    author_name: row.get(5)?,
                    initials: row.get(6)?,
                    color: row.get(7)?,
                    created_at: row.get(8)?,
                    key_epoch: row.get(9)?,
                    nonce: row.get(10)?,
                    ciphertext: row.get(11)?,
                    signature: row.get(12)?,
                })
            })
            .map_err(|error| format!("não foi possível ler mensagens: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler mensagens: {error}"))
    }

    #[allow(dead_code)]
    pub fn latest_message_cursor(
        &self,
        group_id: &str,
        channel_id: &str,
    ) -> Result<(i64, String), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT created_at, event_id FROM messages WHERE group_id = ?1 AND channel_id = ?2 ORDER BY created_at DESC, event_id DESC LIMIT 1",
                params![group_id, channel_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map(|cursor| cursor.unwrap_or((0, String::new())))
            .map_err(|error| format!("não foi possível ler cursor: {error}"))
    }

    pub fn get_sync_cursor(
        &self,
        peer_id: &str,
        group_id: &str,
        channel_id: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT last_created_at, last_event_id FROM sync_cursors WHERE peer_id = ?1 AND group_id = ?2 AND channel_id = ?3",
                params![peer_id, group_id, channel_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("não foi possível ler cursor de sincronização: {error}"))
    }

    pub fn set_sync_cursor(
        &self,
        peer_id: &str,
        group_id: &str,
        channel_id: &str,
        created_at: i64,
        event_id: &str,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT INTO sync_cursors (peer_id, group_id, channel_id, last_created_at, last_event_id) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(peer_id, group_id, channel_id) DO UPDATE SET last_created_at = excluded.last_created_at, last_event_id = excluded.last_event_id WHERE excluded.last_created_at > sync_cursors.last_created_at OR (excluded.last_created_at = sync_cursors.last_created_at AND excluded.last_event_id > sync_cursors.last_event_id)",
                params![peer_id, group_id, channel_id, created_at, event_id],
            )
            .map_err(|error| format!("não foi possível salvar cursor de sincronização: {error}"))?;
        Ok(())
    }

    pub fn has_message_event(&self, event_id: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE event_id = ?1)",
                params![event_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("não foi possível verificar mensagem sincronizada: {error}"))
    }

    pub fn list_messages_after(
        &self,
        group_id: &str,
        channel_id: &str,
        after_created_at: i64,
        after_event_id: &str,
        limit: u32,
    ) -> Result<Vec<MessageEnvelope>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT event_id, group_id, channel_id, author_peer_id, author_public_key, author_name, initials, color, created_at, key_epoch, nonce, ciphertext, signature FROM messages WHERE group_id = ?1 AND channel_id = ?2 AND NOT EXISTS (SELECT 1 FROM message_deletions WHERE message_id = messages.event_id) AND (created_at > ?3 OR (created_at = ?3 AND event_id > ?4)) ORDER BY created_at ASC, event_id ASC LIMIT ?5")
            .map_err(|error| format!("não foi possível ler mensagens novas: {error}"))?;
        let rows = statement
            .query_map(
                params![
                    group_id,
                    channel_id,
                    after_created_at,
                    after_event_id,
                    limit.min(500)
                ],
                |row| {
                    Ok(MessageEnvelope {
                        event_id: row.get(0)?,
                        group_id: row.get(1)?,
                        channel_id: row.get(2)?,
                        author_peer_id: row.get(3)?,
                        author_public_key: row.get(4)?,
                        author_name: row.get(5)?,
                        initials: row.get(6)?,
                        color: row.get(7)?,
                        created_at: row.get(8)?,
                        key_epoch: row.get(9)?,
                        nonce: row.get(10)?,
                        ciphertext: row.get(11)?,
                        signature: row.get(12)?,
                    })
                },
            )
            .map_err(|error| format!("não foi possível ler mensagens novas: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler mensagens novas: {error}"))
    }

    pub fn set_current_key_epoch(&self, group_id: &str, epoch: i64) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "UPDATE groups SET current_key_epoch = ?2 WHERE id = ?1",
                params![group_id, epoch],
            )
            .map_err(|error| format!("não foi possível atualizar época do grupo: {error}"))?;
        Ok(())
    }

    pub fn set_owner_peer_id(&self, group_id: &str, owner_peer_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "UPDATE groups SET owner_peer_id = ?2 WHERE id = ?1",
                params![group_id, owner_peer_id],
            )
            .map_err(|error| format!("não foi possível atualizar Owner: {error}"))?;
        Ok(())
    }

    pub fn delete_group(&self, group_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute("DELETE FROM groups WHERE id = ?1", [group_id])
            .map_err(|error| format!("não foi possível excluir grupo: {error}"))?;
        Ok(())
    }

    pub fn insert_member(&self, member: &MemberRecord) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT INTO group_members (group_id, peer_id, display_name, role, public_key, x25519_public_key, status, joined_at, timeout_until, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT(group_id, peer_id) DO UPDATE SET display_name=excluded.display_name, role=excluded.role, public_key=excluded.public_key, x25519_public_key=excluded.x25519_public_key, status=excluded.status, timeout_until=excluded.timeout_until, updated_at=excluded.updated_at",
                params![
                    member.group_id,
                    member.peer_id,
                    member.display_name,
                    member.role.as_str(),
                    member.public_key,
                    member.x25519_public_key,
                    member.status,
                    member.joined_at,
                    member.timeout_until,
                    member.updated_at
                ],
            )
            .map_err(|error| format!("não foi possível salvar membro: {error}"))?;
        Ok(())
    }

    pub fn get_member(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<Option<MemberRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT group_id, peer_id, display_name, role, public_key, x25519_public_key, status, joined_at, timeout_until, updated_at FROM group_members WHERE group_id = ?1 AND peer_id = ?2",
                params![group_id, peer_id],
                member_from_row,
            )
            .optional()
            .map_err(|error| format!("não foi possível ler membro: {error}"))
    }

    pub fn list_members(&self, group_id: &str) -> Result<Vec<MemberRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT group_id, peer_id, display_name, role, public_key, x25519_public_key, status, joined_at, timeout_until, updated_at FROM group_members WHERE group_id = ?1 ORDER BY joined_at ASC, peer_id ASC")
            .map_err(|error| format!("não foi possível ler membros: {error}"))?;
        let rows = statement
            .query_map([group_id], member_from_row)
            .map_err(|error| format!("não foi possível ler membros: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler membros: {error}"))
    }

    pub fn insert_audit_event(&self, event: &AuditEventRecord) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let inserted = connection
            .execute(
                "INSERT OR IGNORE INTO audit_events (event_id, group_id, key_epoch, issuer_peer_id, kind, target_peer_id, payload, logical_timestamp, signature) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.event_id,
                    event.group_id,
                    event.key_epoch,
                    event.issuer_peer_id,
                    event.kind,
                    event.target_peer_id,
                    serde_json::to_string(&event.payload).map_err(|error| format!("evento administrativo inválido: {error}"))?,
                    event.logical_timestamp,
                    event.signature
                ],
            )
            .map_err(|error| format!("não foi possível salvar auditoria: {error}"))?;
        Ok(inserted > 0)
    }

    pub fn list_audit_events(
        &self,
        group_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditEventRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT event_id, group_id, key_epoch, issuer_peer_id, kind, target_peer_id, payload, logical_timestamp, signature FROM audit_events WHERE group_id = ?1 ORDER BY logical_timestamp ASC, event_id ASC LIMIT ?2")
            .map_err(|error| format!("não foi possível ler auditoria: {error}"))?;
        let rows = statement
            .query_map(params![group_id, limit.min(500)], |row| {
                let payload: String = row.get(6)?;
                Ok(AuditEventRecord {
                    event_id: row.get(0)?,
                    group_id: row.get(1)?,
                    key_epoch: row.get(2)?,
                    issuer_peer_id: row.get(3)?,
                    kind: row.get(4)?,
                    target_peer_id: row.get(5)?,
                    payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
                    logical_timestamp: row.get(7)?,
                    signature: row.get(8)?,
                })
            })
            .map_err(|error| format!("não foi possível ler auditoria: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler auditoria: {error}"))
    }

    pub fn list_audit_events_after(
        &self,
        group_id: &str,
        after_timestamp: i64,
        after_event_id: &str,
        limit: u32,
    ) -> Result<Vec<AuditEventRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT event_id, group_id, key_epoch, issuer_peer_id, kind, target_peer_id, payload, logical_timestamp, signature FROM audit_events WHERE group_id = ?1 AND (logical_timestamp > ?2 OR (logical_timestamp = ?2 AND event_id > ?3)) ORDER BY logical_timestamp ASC, event_id ASC LIMIT ?4")
            .map_err(|error| format!("não foi possível ler eventos novos: {error}"))?;
        let rows = statement
            .query_map(
                params![group_id, after_timestamp, after_event_id, limit.min(500)],
                |row| {
                    let payload: String = row.get(6)?;
                    Ok(AuditEventRecord {
                        event_id: row.get(0)?,
                        group_id: row.get(1)?,
                        key_epoch: row.get(2)?,
                        issuer_peer_id: row.get(3)?,
                        kind: row.get(4)?,
                        target_peer_id: row.get(5)?,
                        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
                        logical_timestamp: row.get(7)?,
                        signature: row.get(8)?,
                    })
                },
            )
            .map_err(|error| format!("não foi possível ler eventos novos: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler eventos novos: {error}"))
    }

    pub fn owner_authorized_admin(
        &self,
        group_id: &str,
        owner_peer_id: &str,
        admin_peer_id: &str,
    ) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let latest: Option<(String, String)> = connection
            .query_row(
                "SELECT issuer_peer_id, payload FROM audit_events WHERE group_id = ?1 AND kind = 'role_changed' AND target_peer_id = ?2 ORDER BY logical_timestamp DESC, event_id DESC LIMIT 1",
                params![group_id, admin_peer_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("não foi possível ler delegação de cargo: {error}"))?;
        let Some((issuer_peer_id, payload)) = latest else {
            return Ok(false);
        };
        let role = serde_json::from_str::<serde_json::Value>(&payload)
            .ok()
            .and_then(|value| {
                value
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        Ok(issuer_peer_id == owner_peer_id && role == "admin")
    }

    pub fn has_audit_event(&self, event_id: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_events WHERE event_id = ?1)",
                params![event_id],
                |row| row.get(0),
            )
            .map_err(|error| format!("não foi possível verificar evento sincronizado: {error}"))
    }

    #[allow(dead_code)]
    pub fn latest_audit_cursor(&self, group_id: &str) -> Result<(i64, String), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT logical_timestamp, event_id FROM audit_events WHERE group_id = ?1 ORDER BY logical_timestamp DESC, event_id DESC LIMIT 1",
                [group_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map(|cursor| cursor.unwrap_or((0, String::new())))
            .map_err(|error| format!("não foi possível ler cursor administrativo: {error}"))
    }

    pub fn latest_member_control_cursor(
        &self,
        group_id: &str,
        peer_id: &str,
    ) -> Result<Option<(i64, String)>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row(
                "SELECT logical_timestamp, event_id FROM audit_events WHERE group_id = ?1 AND target_peer_id = ?2 AND kind IN ('member_joined', 'role_changed', 'member_kicked', 'member_banned', 'member_unbanned', 'member_timeout', 'member_left') ORDER BY logical_timestamp DESC, event_id DESC LIMIT 1",
                params![group_id, peer_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| format!("não foi possível ler cursor do membro: {error}"))
    }

    pub fn has_audit_event_before(
        &self,
        group_id: &str,
        kind: &str,
        key_epoch: i64,
        target_peer_id: Option<&str>,
        logical_timestamp: i64,
    ) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let exists = if let Some(target_peer_id) = target_peer_id {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_events WHERE group_id = ?1 AND kind = ?2 AND key_epoch = ?3 AND target_peer_id = ?4 AND logical_timestamp <= ?5)",
                params![group_id, kind, key_epoch, target_peer_id, logical_timestamp],
                |row| row.get::<_, i64>(0),
            ).map_err(|error| format!("não foi possível ler autorização de chave: {error}"))?
        } else {
            connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM audit_events WHERE group_id = ?1 AND kind = ?2 AND key_epoch = ?3 AND target_peer_id IS NULL AND logical_timestamp <= ?4)",
                params![group_id, kind, key_epoch, logical_timestamp],
                |row| row.get::<_, i64>(0),
            ).map_err(|error| format!("não foi possível ler autorização de chave: {error}"))?
        };
        Ok(exists != 0)
    }

    pub fn member_is_active_at(
        &self,
        group_id: &str,
        peer_id: &str,
        logical_timestamp: i64,
    ) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let joined_at = connection
            .query_row(
                "SELECT joined_at FROM group_members WHERE group_id = ?1 AND peer_id = ?2",
                params![group_id, peer_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("não foi possível ler histórico de membro: {error}"))?;
        let Some(joined_at) = joined_at else {
            return Ok(false);
        };
        let mut active = joined_at <= logical_timestamp;
        let mut statement = connection
            .prepare("SELECT kind FROM audit_events WHERE group_id = ?1 AND target_peer_id = ?2 AND logical_timestamp <= ?3 AND kind IN ('member_joined', 'member_kicked', 'member_banned', 'member_unbanned', 'member_left') ORDER BY logical_timestamp ASC, event_id ASC")
            .map_err(|error| format!("não foi possível ler histórico de membro: {error}"))?;
        let events = statement
            .query_map(params![group_id, peer_id, logical_timestamp], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("não foi possível ler histórico de membro: {error}"))?;
        for event in events {
            match event
                .map_err(|error| format!("não foi possível ler histórico de membro: {error}"))?
                .as_str()
            {
                "member_joined" | "member_unbanned" => active = true,
                "member_kicked" | "member_banned" | "member_left" => active = false,
                _ => {}
            }
        }
        Ok(active)
    }

    pub fn timeout_until_at(
        &self,
        group_id: &str,
        peer_id: &str,
        logical_timestamp: i64,
    ) -> Result<Option<i64>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT payload FROM audit_events WHERE group_id = ?1 AND target_peer_id = ?2 AND kind = 'member_timeout' AND logical_timestamp <= ?3 ORDER BY logical_timestamp ASC, event_id ASC")
            .map_err(|error| format!("não foi possível ler timeout histórico: {error}"))?;
        let rows = statement
            .query_map(params![group_id, peer_id, logical_timestamp], |row| {
                row.get::<_, String>(0)
            })
            .map_err(|error| format!("não foi possível ler timeout histórico: {error}"))?;
        let mut timeout_until = None;
        for row in rows {
            let payload =
                row.map_err(|error| format!("não foi possível ler timeout histórico: {error}"))?;
            timeout_until = serde_json::from_str::<serde_json::Value>(&payload)
                .ok()
                .and_then(|value| value.get("until").and_then(serde_json::Value::as_i64));
        }
        Ok(timeout_until)
    }

    pub fn insert_key_epoch(
        &self,
        group_id: &str,
        epoch: i64,
        created_by: &str,
        created_at: i64,
        key_hash: &[u8],
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO key_epochs (group_id, epoch, created_by, created_at, key_hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![group_id, epoch, created_by, created_at, key_hash],
            )
            .map_err(|error| format!("não foi possível salvar época de chave: {error}"))?;
        Ok(())
    }

    pub fn remember_peer(&self, peer_id: &str, last_seen: i64) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection.execute("INSERT INTO peers(peer_id, last_seen) VALUES(?1, ?2) ON CONFLICT(peer_id) DO UPDATE SET last_seen=excluded.last_seen", params![peer_id, last_seen]).map_err(|error| format!("não foi possível salvar peer: {error}"))?;
        Ok(())
    }

    pub fn remember_peer_address(
        &self,
        peer_id: &str,
        address: &str,
        source: &str,
        last_seen: i64,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT INTO peer_addresses(peer_id, address, source, last_seen) VALUES(?1, ?2, ?3, ?4) ON CONFLICT(peer_id, address) DO UPDATE SET source=excluded.source, last_seen=excluded.last_seen",
                params![peer_id, address, source, last_seen],
            )
            .map_err(|error| format!("não foi possível salvar endereço de peer: {error}"))?;
        Ok(())
    }

    pub fn list_peer_addresses(&self) -> Result<Vec<PeerAddressRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT peer_id, address, source, last_seen FROM peer_addresses ORDER BY last_seen DESC, peer_id, address")
            .map_err(|error| format!("não foi possível ler endereços de peers: {error}"))?;
        let rows = statement
            .query_map([], |row| {
                Ok(PeerAddressRecord {
                    peer_id: row.get(0)?,
                    address: row.get(1)?,
                    source: row.get(2)?,
                    last_seen: row.get(3)?,
                })
            })
            .map_err(|error| format!("não foi possível ler endereços de peers: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler endereços de peers: {error}"))
    }

    pub fn enqueue_outbox(
        &self,
        event_id: &str,
        kind: &str,
        payload: &str,
        created_at: i64,
    ) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "INSERT OR IGNORE INTO outbox(event_id, kind, payload, created_at) VALUES(?1, ?2, ?3, ?4)",
                params![event_id, kind, payload, created_at],
            )
            .map_err(|error| format!("não foi possível enfileirar evento: {error}"))?;
        Ok(())
    }

    pub fn list_outbox(&self, limit: u32) -> Result<Vec<OutboxRecord>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        let mut statement = connection
            .prepare("SELECT event_id, kind, payload, created_at, attempts, last_attempt_at FROM outbox ORDER BY created_at ASC, event_id ASC LIMIT ?1")
            .map_err(|error| format!("não foi possível ler fila de saída: {error}"))?;
        let rows = statement
            .query_map([i64::from(limit.min(1000))], |row| {
                Ok(OutboxRecord {
                    event_id: row.get(0)?,
                    kind: row.get(1)?,
                    payload: row.get(2)?,
                    created_at: row.get(3)?,
                    attempts: row.get(4)?,
                    last_attempt_at: row.get(5)?,
                })
            })
            .map_err(|error| format!("não foi possível ler fila de saída: {error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("não foi possível ler fila de saída: {error}"))
    }

    pub fn mark_outbox_attempt(&self, event_id: &str, attempted_at: i64) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute(
                "UPDATE outbox SET attempts = attempts + 1, last_attempt_at = ?2 WHERE event_id = ?1",
                params![event_id, attempted_at],
            )
            .map_err(|error| format!("não foi possível atualizar fila de saída: {error}"))?;
        Ok(())
    }

    pub fn remove_outbox(&self, event_id: &str) -> Result<(), String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .execute("DELETE FROM outbox WHERE event_id = ?1", [event_id])
            .map_err(|error| format!("não foi possível confirmar evento enviado: {error}"))?;
        Ok(())
    }

    pub fn has_group(&self, group_id: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "banco local bloqueado".to_string())?;
        connection
            .query_row("SELECT 1 FROM groups WHERE id = ?1", [group_id], |_| {
                Ok(true)
            })
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(|error| format!("não foi possível verificar grupo: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_and_message_deduplication_work() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        let group = GroupRecord {
            id: "g1".into(),
            name: "Amigos".into(),
            initials: "AM".into(),
            color: "#8276ff".into(),
            owner_peer_id: "peer".into(),
            created_at: 1,
            current_key_epoch: 1,
        };
        database.insert_group(&group).expect("group");
        database.insert_default_channels("g1").expect("channels");
        let envelope = MessageEnvelope {
            event_id: "e1".into(),
            group_id: "g1".into(),
            channel_id: "g1:geral".into(),
            author_peer_id: "peer".into(),
            author_public_key: vec![1],
            author_name: "Você".into(),
            initials: "VC".into(),
            color: "#8276ff".into(),
            created_at: 2,
            key_epoch: 1,
            nonce: vec![0; 24],
            ciphertext: vec![2],
            signature: vec![3],
        };
        assert!(database.insert_message(&envelope).unwrap());
        assert!(!database.insert_message(&envelope).unwrap());
        assert_eq!(
            database.list_messages("g1", "g1:geral", 50).unwrap().len(),
            1
        );
        assert_eq!(
            database.latest_message_cursor("g1", "g1:geral").unwrap(),
            (2, "e1".to_string())
        );
        database
            .set_sync_cursor("remote-peer", "g1", "g1:geral", 2, "e1")
            .expect("cursor de mensagem");
        database
            .set_sync_cursor("remote-peer", "g1", "g1:geral", 1, "old")
            .expect("cursor antigo ignorado");
        assert_eq!(
            database
                .get_sync_cursor("remote-peer", "g1", "g1:geral")
                .expect("ler cursor")
                .expect("cursor presente"),
            (2, "e1".to_string())
        );
        assert!(database.has_message_event("e1").expect("mensagem presente"));
        assert!(database
            .list_messages_after("g1", "g1:geral", 2, "e1", 50)
            .unwrap()
            .is_empty());
        let member = MemberRecord {
            group_id: "g1".into(),
            peer_id: "peer-2".into(),
            display_name: "Amigo".into(),
            role: Role::Member,
            public_key: vec![4],
            x25519_public_key: vec![5; 32],
            status: "active".into(),
            joined_at: 3,
            timeout_until: None,
            updated_at: 3,
        };
        database.insert_member(&member).expect("member");
        assert_eq!(database.list_members("g1").unwrap().len(), 1);
        assert!(database
            .insert_audit_event(&AuditEventRecord {
                event_id: "audit-1".into(),
                group_id: "g1".into(),
                key_epoch: 1,
                issuer_peer_id: "peer".into(),
                kind: "role_changed".into(),
                target_peer_id: Some("peer-2".into()),
                payload: serde_json::json!({ "role": "mod" }),
                logical_timestamp: 4,
                signature: vec![6],
            })
            .unwrap());
        assert!(!database
            .insert_audit_event(&AuditEventRecord {
                event_id: "audit-1".into(),
                group_id: "g1".into(),
                key_epoch: 1,
                issuer_peer_id: "peer".into(),
                kind: "role_changed".into(),
                target_peer_id: Some("peer-2".into()),
                payload: serde_json::json!({ "role": "mod" }),
                logical_timestamp: 4,
                signature: vec![6],
            })
            .unwrap());
        database
            .insert_audit_event(&AuditEventRecord {
                event_id: "audit-ban".into(),
                group_id: "g1".into(),
                key_epoch: 1,
                issuer_peer_id: "peer".into(),
                kind: "member_banned".into(),
                target_peer_id: Some("peer-2".into()),
                payload: serde_json::json!({}),
                logical_timestamp: 5,
                signature: vec![7],
            })
            .expect("ban audit");
        assert!(database
            .has_audit_event("audit-ban")
            .expect("auditoria presente"));
        database
            .insert_audit_event(&AuditEventRecord {
                event_id: "audit-unban".into(),
                group_id: "g1".into(),
                key_epoch: 1,
                issuer_peer_id: "peer".into(),
                kind: "member_unbanned".into(),
                target_peer_id: Some("peer-2".into()),
                payload: serde_json::json!({}),
                logical_timestamp: 6,
                signature: vec![8],
            })
            .expect("unban audit");
        database
            .insert_audit_event(&AuditEventRecord {
                event_id: "audit-timeout".into(),
                group_id: "g1".into(),
                key_epoch: 1,
                issuer_peer_id: "peer".into(),
                kind: "member_timeout".into(),
                target_peer_id: Some("peer-2".into()),
                payload: serde_json::json!({ "until": 20 }),
                logical_timestamp: 7,
                signature: vec![9],
            })
            .expect("timeout audit");
        assert!(database
            .member_is_active_at("g1", "peer-2", 4)
            .expect("active before ban"));
        assert!(!database
            .member_is_active_at("g1", "peer-2", 5)
            .expect("inactive at ban"));
        assert!(database
            .member_is_active_at("g1", "peer-2", 6)
            .expect("active after unban"));
        assert_eq!(
            database
                .timeout_until_at("g1", "peer-2", 6)
                .expect("timeout before action"),
            None
        );
        assert_eq!(
            database
                .timeout_until_at("g1", "peer-2", 8)
                .expect("timeout after action"),
            Some(20)
        );
    }

    #[test]
    fn existing_sync_cursor_schema_migrates_without_data_loss() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("teamscord.sqlite");
        {
            let connection = Connection::open(&path).expect("legacy connection");
            connection
                .execute_batch(
                    "CREATE TABLE sync_cursors (
                       peer_id TEXT NOT NULL,
                       group_id TEXT NOT NULL,
                       channel_id TEXT NOT NULL,
                       last_created_at INTEGER NOT NULL DEFAULT 0,
                       PRIMARY KEY(peer_id, group_id, channel_id)
                     );
                     INSERT INTO sync_cursors(peer_id, group_id, channel_id, last_created_at)
                     VALUES ('peer', 'group', 'channel', 10);",
                )
                .expect("legacy cursor schema");
        }
        let database = Database::open(&path).expect("migrated database");
        assert_eq!(
            database
                .get_sync_cursor("peer", "group", "channel")
                .expect("read migrated cursor")
                .expect("cursor exists"),
            (10, String::new())
        );
        database
            .set_sync_cursor("peer", "group", "channel", 10, "event-10")
            .expect("update migrated cursor");
        assert_eq!(
            database
                .get_sync_cursor("peer", "group", "channel")
                .expect("read updated cursor")
                .expect("updated cursor exists"),
            (10, "event-10".into())
        );
    }

    #[test]
    fn peer_contacts_and_outbox_survive_reopen_and_deduplicate() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("teamscord.sqlite");
        {
            let database = Database::open(&path).expect("db");
            database
                .remember_peer_address("peer-1", "/ip4/127.0.0.1/tcp/4001/p2p/peer-1", "invite", 10)
                .expect("peer contact");
            database
                .enqueue_outbox("event-1", "message", "{}", 11)
                .expect("outbox");
            database
                .enqueue_outbox("event-1", "message", "different", 12)
                .expect("outbox dedup");
            assert_eq!(database.list_peer_addresses().unwrap().len(), 1);
            assert_eq!(database.list_outbox(20).unwrap().len(), 1);
        }
        let database = Database::open(&path).expect("reopen db");
        assert_eq!(database.list_peer_addresses().unwrap()[0].source, "invite");
        assert_eq!(database.list_outbox(20).unwrap()[0].payload, "{}");
        database
            .mark_outbox_attempt("event-1", 13)
            .expect("attempt");
        database.remove_outbox("event-1").expect("ack");
        assert!(database.list_outbox(20).unwrap().is_empty());
    }

    #[test]
    fn legacy_screen_channels_are_migrated_to_voice_and_cursor_orders_same_timestamp() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        let group = GroupRecord {
            id: "g2".into(),
            name: "Migração".into(),
            initials: "MI".into(),
            color: "#8276ff".into(),
            owner_peer_id: "peer".into(),
            created_at: 1,
            current_key_epoch: 1,
        };
        database.insert_group(&group).expect("group");
        {
            let connection = database.connection.lock().expect("connection");
            connection
                .execute(
                    "INSERT INTO channels (id, group_id, name, kind, position) VALUES (?1, ?2, ?3, 'screen', 0)",
                    params!["g2:screen", "g2", "compartilhar"],
                )
                .expect("legacy channel");
        }
        database.migrate().expect("migrate");
        assert_eq!(database.list_channels("g2").unwrap()[0].kind, "voice");

        let first = MessageEnvelope {
            event_id: "same-a".into(),
            group_id: "g2".into(),
            channel_id: "g2:screen".into(),
            author_peer_id: "peer".into(),
            author_public_key: vec![1],
            author_name: "A".into(),
            initials: "A".into(),
            color: "#8276ff".into(),
            created_at: 10,
            key_epoch: 1,
            nonce: vec![0; 24],
            ciphertext: vec![2],
            signature: vec![3],
        };
        let mut second = first.clone();
        second.event_id = "same-b".into();
        database.insert_message(&first).expect("first message");
        database.insert_message(&second).expect("second message");
        let next = database
            .list_messages_after("g2", "g2:screen", 10, "same-a", 50)
            .expect("cursor");
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].event_id, "same-b");
    }

    #[test]
    fn channel_permissions_and_message_deletion_are_persistent_tombstones() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        database
            .insert_group(&GroupRecord {
                id: "g3".into(),
                name: "Permissões".into(),
                initials: "PE".into(),
                color: "#8276ff".into(),
                owner_peer_id: "owner".into(),
                created_at: 1,
                current_key_epoch: 1,
            })
            .expect("group");
        database.insert_default_channels("g3").expect("channels");
        let permission = ChannelPermissionRecord {
            group_id: "g3".into(),
            channel_id: "g3:lounge".into(),
            role: Role::Member,
            can_join_voice: true,
            can_share_screen: false,
        };
        database
            .set_channel_permission(&permission)
            .expect("permission");
        assert!(
            !database
                .get_channel_permission("g3:lounge", &Role::Member)
                .expect("read permission")
                .expect("permission exists")
                .can_share_screen
        );

        let message = MessageEnvelope {
            event_id: "delete-me".into(),
            group_id: "g3".into(),
            channel_id: "g3:geral".into(),
            author_peer_id: "owner".into(),
            author_public_key: vec![1],
            author_name: "Dono".into(),
            initials: "DO".into(),
            color: "#8276ff".into(),
            created_at: 2,
            key_epoch: 1,
            nonce: vec![0; 24],
            ciphertext: vec![2],
            signature: vec![3],
        };
        assert!(database.insert_message(&message).expect("message"));
        assert!(database
            .delete_message("delete-me", "g3", "g3:geral", "owner", 3)
            .expect("delete"));
        assert!(database.is_message_deleted("delete-me").expect("tombstone"));
        assert!(database
            .list_messages("g3", "g3:geral", 50)
            .expect("visible messages")
            .is_empty());
        assert!(!database
            .delete_message("delete-me", "g3", "g3:geral", "owner", 4)
            .expect("deduplicated delete"));
    }
}
