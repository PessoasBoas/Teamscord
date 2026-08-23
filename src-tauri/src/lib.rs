mod access;
mod crypto;
mod protocol;
mod storage;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures::StreamExt;
use keyring::Entry;
use libp2p::{
    gossipsub::{self, IdentTopic, MessageAuthenticity},
    identify,
    identity::Keypair,
    mdns,
    multiaddr::Protocol,
    noise, ping, relay,
    request_response::{self, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    access::{
        has_permission, sign_control_event, verify_control_event, ControlEvent, Permission, Role,
        CONTROL_CHANNEL_CREATED, CONTROL_CHANNEL_DELETED, CONTROL_CHANNEL_PERMISSION_UPDATED,
        CONTROL_CHANNEL_REORDERED, CONTROL_CHANNEL_UPDATED, CONTROL_GROUP_DELETED,
        CONTROL_GROUP_OWNER_TRANSFERRED, CONTROL_KEY_ROTATED, CONTROL_KEY_SHARE,
        CONTROL_MEMBER_BANNED, CONTROL_MEMBER_JOINED, CONTROL_MEMBER_KICKED, CONTROL_MEMBER_LEFT,
        CONTROL_MEMBER_TIMEOUT, CONTROL_MEMBER_UNBANNED, CONTROL_MESSAGE_DELETED,
        CONTROL_ROLE_CHANGED,
    },
    crypto::{
        decrypt_from_sender, decrypt_message, encrypt_for_recipient, encrypt_message,
        new_group_key, new_x25519_keypair, validate_invite, verify_author, GROUP_KEY_BYTES,
        X25519_KEY_BYTES,
    },
    protocol::{
        aad_for_envelope, contact_card_signing_bytes, direct_aad, direct_signing_bytes,
        signing_bytes, CallSignal, CallSignalBody, ContactCard, DirectBody, DirectEnvelope,
        MessageEnvelope, PresenceAnnouncement, PresenceCall, SyncRequest, SyncResponse,
        CALL_SIGNAL_PROTOCOL, DIRECT_PROTOCOL, PRESENCE_PROTOCOL, SYNC_PROTOCOL,
    },
    storage::{
        AuditEventRecord, ChannelPermissionRecord, ChannelRecord, ContactRecord, Database,
        DirectMessageRecord, GroupRecord, MemberRecord,
    },
};

const EVENT_NAME: &str = "node://event";
const IDENTITY_SERVICE: &str = "com.teamscord.desktop";
const IDENTITY_USERNAME: &str = "node-identity";
const GROUP_KEY_SERVICE: &str = "com.teamscord.desktop.group";
const AGREEMENT_SERVICE: &str = "com.teamscord.desktop.agreement";
const AGREEMENT_USERNAME: &str = "node-x25519";
const MEDIA_CONFIG_SERVICE: &str = "com.teamscord.desktop.media";
const MEDIA_CONFIG_USERNAME: &str = "ice-servers";
const MAX_CALL_PARTICIPANTS: usize = 8;
const BUILTIN_DEFAULT_RELAY_ADDRESS: &str = "/dns4/altaria.proxy.rlwy.net/tcp/46712/p2p/12D3KooWNw8qUoVxFy8XcRkXhwPF4rdGjz4mqRf3hgqnoJbBvtwt";

fn default_relay_addresses() -> Vec<String> {
    option_env!("TEAMSCORD_DEFAULT_RELAY_ADDRESS")
        .or(Some(BUILTIN_DEFAULT_RELAY_ADDRESS))
        .into_iter()
        .map(str::trim)
        .filter(|address| !address.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub peer_id: String,
    pub listen_addresses: Vec<String>,
    pub connected_peers: usize,
    pub is_running: bool,
    pub relay_addresses: Vec<String>,
    pub bootstrap_addresses: Vec<String>,
    #[serde(default)]
    pub relay_connected: bool,
    #[serde(default)]
    pub last_sync_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkDiagnosticCheck {
    pub name: String,
    pub status: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkDiagnostics {
    pub status: String,
    pub summary: String,
    pub checks: Vec<NetworkDiagnosticCheck>,
    pub connected_peers: usize,
    pub checked_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub group_id: String,
    pub channel_id: String,
    pub author_peer_id: String,
    pub author: String,
    pub initials: String,
    pub color: String,
    pub content: String,
    pub timestamp: String,
    pub created_at: i64,
    pub mine: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactCardView {
    pub peer_id: String,
    pub display_name: String,
    pub public_key: Vec<u8>,
    pub x25519_public_key: Vec<u8>,
    pub addresses: Vec<String>,
    pub encoded: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FriendView {
    pub peer_id: String,
    pub display_name: String,
    pub status: String,
    pub requested_by: String,
    pub online: bool,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectMessageView {
    pub id: String,
    pub conversation_id: String,
    pub from_peer_id: String,
    pub to_peer_id: String,
    pub author: String,
    pub content: String,
    pub created_at: i64,
    pub mine: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchResult {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub group_id: String,
    pub channel_id: Option<String>,
    pub created_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupView {
    pub id: String,
    pub name: String,
    pub initials: String,
    pub color: String,
    pub owner_peer_id: String,
    pub created_at: i64,
    pub channels: Vec<ChannelRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupCreateResult {
    pub group: GroupView,
    pub invite: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoleView {
    pub role: Role,
    pub permissions: Vec<Permission>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallParticipant {
    pub peer_id: String,
    pub display_name: String,
    pub role: Role,
    pub muted: bool,
    pub sharing_screen: bool,
    pub connection_state: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallState {
    pub group_id: String,
    pub channel_id: String,
    pub call_id: String,
    pub participants: Vec<CallParticipant>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MediaConfig {
    pub ice_servers: Vec<IceServerConfig>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub relay_addresses: Vec<String>,
    #[serde(default)]
    pub bootstrap_addresses: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeContact {
    pub peer_id: String,
    pub addresses: Vec<String>,
    pub source: String,
    pub last_seen: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayStatus {
    pub address: String,
    pub peer_id: Option<String>,
    pub state: String,
    pub last_seen: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerPresence {
    pub peer_id: String,
    pub state: String,
    pub last_seen: i64,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionDiagnostic {
    pub peer_id: String,
    pub state: String,
    pub detail: String,
    pub checked_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkStatusView {
    pub snapshot: NodeSnapshot,
    pub relays: Vec<RelayStatus>,
    pub peers: Vec<NodeContact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NodeEvent {
    kind: String,
    message: Option<ChatMessage>,
    snapshot: Option<NodeSnapshot>,
    error: Option<String>,
    data: Option<serde_json::Value>,
}

enum NodeCommand {
    Dial { address: Multiaddr },
    ConfigureRelay { address: Multiaddr },
    Publish { envelope: Box<MessageEnvelope> },
    PublishControl { event: Box<ControlEvent> },
    PublishCall { signal: Box<CallSignal> },
    PublishDirect { envelope: Box<DirectEnvelope> },
    SubscribeGroup { group_id: String },
    RequestSync,
    BroadcastPresence,
}

#[derive(Clone)]
pub struct NodeState {
    database: Arc<Database>,
    data_dir: Arc<PathBuf>,
    keypair: Arc<Mutex<Option<Keypair>>>,
    snapshot: Arc<Mutex<NodeSnapshot>>,
    command_tx: Arc<Mutex<Option<mpsc::UnboundedSender<NodeCommand>>>>,
    network_config: Arc<Mutex<NetworkConfig>>,
    call_states: Arc<Mutex<HashMap<String, CallState>>>,
    pending_messages: Arc<Mutex<VecDeque<MessageEnvelope>>>,
    pending_controls: Arc<Mutex<VecDeque<ControlEvent>>>,
    pending_call_signals: Arc<Mutex<VecDeque<CallSignal>>>,
    handled_call_signals: Arc<Mutex<HashSet<String>>>,
    media_config: Arc<Mutex<MediaConfig>>,
}

impl NodeState {
    fn open(data_dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&data_dir)
            .map_err(|error| format!("não foi possível criar dados do app: {error}"))?;
        let database = Arc::new(Database::open(data_dir.join("teamscord.sqlite"))?);
        let mut network_config = load_network_config(&data_dir)?;
        for address in default_relay_addresses() {
            if !network_config
                .relay_addresses
                .iter()
                .any(|item| item == &address)
            {
                network_config.relay_addresses.push(address);
            }
        }
        Ok(Self {
            database,
            data_dir: Arc::new(data_dir),
            keypair: Arc::new(Mutex::new(None)),
            snapshot: Arc::new(Mutex::new(NodeSnapshot {
                peer_id: String::new(),
                listen_addresses: Vec::new(),
                connected_peers: 0,
                is_running: false,
                relay_addresses: network_config.relay_addresses.clone(),
                bootstrap_addresses: network_config.bootstrap_addresses.clone(),
                relay_connected: false,
                last_sync_at: None,
            })),
            command_tx: Arc::new(Mutex::new(None)),
            network_config: Arc::new(Mutex::new(network_config)),
            call_states: Arc::new(Mutex::new(HashMap::new())),
            pending_messages: Arc::new(Mutex::new(VecDeque::new())),
            pending_controls: Arc::new(Mutex::new(VecDeque::new())),
            pending_call_signals: Arc::new(Mutex::new(VecDeque::new())),
            handled_call_signals: Arc::new(Mutex::new(HashSet::new())),
            media_config: Arc::new(Mutex::new(load_media_config()?)),
        })
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    mdns: mdns::tokio::Behaviour,
    ping: ping::Behaviour,
    relay: relay::client::Behaviour,
    sync: request_response::json::Behaviour<SyncRequest, SyncResponse>,
    call_signal: request_response::json::Behaviour<CallSignal, CallSignal>,
    presence: request_response::json::Behaviour<PresenceAnnouncement, PresenceAnnouncement>,
    direct: request_response::json::Behaviour<DirectEnvelope, DirectEnvelope>,
}

#[tauri::command]
async fn start_node(app: AppHandle, state: State<'_, NodeState>) -> Result<NodeSnapshot, String> {
    {
        let snapshot = state
            .snapshot
            .lock()
            .map_err(|_| "estado do node indisponível")?;
        if snapshot.is_running {
            return Ok(snapshot.clone());
        }
    }

    let keypair = ensure_keypair(&state)?;
    ensure_legacy_owner_memberships(&state, &keypair)?;
    let peer_id = keypair.public().to_peer_id();
    let relay_addresses = state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")?
        .relay_addresses
        .clone();
    let bootstrap_addresses = state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")?
        .bootstrap_addresses
        .clone();
    let snapshot = NodeSnapshot {
        peer_id: peer_id.to_string(),
        listen_addresses: Vec::new(),
        connected_peers: 0,
        is_running: true,
        relay_addresses,
        bootstrap_addresses,
        relay_connected: false,
        last_sync_at: None,
    };
    *state
        .snapshot
        .lock()
        .map_err(|_| "estado do node indisponível")? = snapshot.clone();
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    *state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")? = Some(command_tx);
    let runtime_state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_node(app.clone(), runtime_state.clone(), keypair, command_rx).await
        {
            emit_error(&app, error);
            if let Ok(mut current) = runtime_state.snapshot.lock() {
                current.is_running = false;
                emit_snapshot(&app, &current);
            }
        }
    });
    Ok(snapshot)
}

#[tauri::command]
fn get_node_snapshot(state: State<'_, NodeState>) -> Result<NodeSnapshot, String> {
    state
        .snapshot
        .lock()
        .map(|snapshot| snapshot.clone())
        .map_err(|_| "estado do node indisponível".into())
}

#[tauri::command]
fn get_known_peers(state: State<'_, NodeState>) -> Result<Vec<NodeContact>, String> {
    let mut grouped = HashMap::<String, NodeContact>::new();
    for record in state.database.list_peer_addresses()? {
        let entry = grouped
            .entry(record.peer_id.clone())
            .or_insert_with(|| NodeContact {
                peer_id: record.peer_id.clone(),
                addresses: Vec::new(),
                source: record.source.clone(),
                last_seen: record.last_seen,
            });
        if !entry.addresses.contains(&record.address) {
            entry.addresses.push(record.address);
        }
        entry.last_seen = entry.last_seen.max(record.last_seen);
    }
    Ok(grouped.into_values().collect())
}

#[tauri::command]
fn get_contact_card(
    display_name: Option<String>,
    state: State<'_, NodeState>,
) -> Result<ContactCardView, String> {
    let card = local_contact_card(&state, display_name.as_deref().unwrap_or("Você"))?;
    let encoded = encode_contact_card(&card)?;
    Ok(ContactCardView {
        peer_id: card.peer_id,
        display_name: card.display_name,
        public_key: card.public_key,
        x25519_public_key: card.x25519_public_key,
        addresses: card.addresses,
        encoded: format!("teamscord://contact/v1/{encoded}"),
    })
}

#[tauri::command]
fn list_friends(state: State<'_, NodeState>) -> Result<Vec<FriendView>, String> {
    let contacts = state.database.list_contacts(None)?;
    Ok(contacts
        .into_iter()
        .map(|contact| friend_view(&state, contact))
        .collect())
}

#[tauri::command]
fn create_friend_request(
    contact: String,
    display_name: Option<String>,
    app: AppHandle,
    state: State<'_, NodeState>,
) -> Result<FriendView, String> {
    let card = decode_contact_card(&contact)?;
    let local = ensure_keypair(&state)?;
    let local_peer_id = local.public().to_peer_id().to_string();
    if card.peer_id == local_peer_id {
        return Err("não é possível adicionar a própria identidade".into());
    }
    validate_contact_card(&card)?;
    for address in &card.addresses {
        if let Ok(address) = address.parse::<Multiaddr>() {
            let _ = state.database.remember_peer_address(
                &card.peer_id,
                address.to_string().as_str(),
                "contact",
                now_millis(),
            );
            if let Ok(sender) = state.command_tx.lock() {
                if let Some(sender) = sender.clone() {
                    let _ = sender.send(NodeCommand::Dial {
                        address: peer_dial_address(
                            address,
                            &card
                                .peer_id
                                .parse()
                                .map_err(|_| "peer id do contato inválido")?,
                        ),
                    });
                }
            }
        }
    }
    let existing = state.database.get_contact(&card.peer_id)?;
    if existing
        .as_ref()
        .is_some_and(|contact| contact.status == "blocked")
    {
        return Err("este contato está bloqueado".into());
    }
    let now = now_millis();
    let requested_by = "local";
    state.database.upsert_contact(&ContactRecord {
        peer_id: card.peer_id.clone(),
        display_name: card.display_name.clone(),
        public_key: card.public_key.clone(),
        x25519_public_key: card.x25519_public_key.clone(),
        status: "pending".into(),
        requested_by: requested_by.into(),
        created_at: existing
            .as_ref()
            .map(|contact| contact.created_at)
            .unwrap_or(now),
        updated_at: now,
        last_seen: now,
    })?;
    let envelope = new_direct_envelope(
        &state,
        &card,
        "friend_request",
        DirectBody {
            request_id: Uuid::new_v4().to_string(),
            conversation_id: String::new(),
            display_name: Some(display_name.clone().unwrap_or_else(|| "Você".into())),
            contact_card: Some(local_contact_card(
                &state,
                display_name.as_deref().unwrap_or("Você"),
            )?),
            content: None,
        },
    )?;
    enqueue_and_publish_direct(&app, &state, envelope)?;
    state
        .database
        .get_contact(&card.peer_id)?
        .map(|contact| friend_view(&state, contact))
        .ok_or_else(|| "contato não foi salvo".into())
}

#[tauri::command]
fn respond_friend_request(
    peer_id: String,
    response: String,
    display_name: Option<String>,
    app: AppHandle,
    state: State<'_, NodeState>,
) -> Result<FriendView, String> {
    let contact = state
        .database
        .get_contact(&peer_id)?
        .ok_or("solicitação de amizade não encontrada")?;
    let next_status = match response.as_str() {
        "accept" => "accepted",
        "reject" => "rejected",
        "block" => "blocked",
        _ => return Err("resposta de amizade inválida".into()),
    };
    let local_card = local_contact_card(&state, display_name.as_deref().unwrap_or("Você"))?;
    let now = now_millis();
    state.database.upsert_contact(&ContactRecord {
        status: next_status.into(),
        updated_at: now,
        last_seen: now,
        ..contact.clone()
    })?;
    let envelope = new_direct_envelope(
        &state,
        &contact_to_card(&contact),
        match next_status {
            "accepted" => "friend_accept",
            "rejected" => "friend_reject",
            _ => "friend_block",
        },
        DirectBody {
            request_id: Uuid::new_v4().to_string(),
            conversation_id: String::new(),
            display_name: Some(local_card.display_name.clone()),
            contact_card: Some(local_card),
            content: None,
        },
    )?;
    enqueue_and_publish_direct(&app, &state, envelope)?;
    Ok(friend_view(
        &state,
        ContactRecord {
            status: next_status.into(),
            updated_at: now,
            last_seen: now,
            ..contact
        },
    ))
}

#[tauri::command]
fn get_direct_messages(
    peer_id: String,
    state: State<'_, NodeState>,
) -> Result<Vec<DirectMessageView>, String> {
    let local_peer_id = ensure_keypair(&state)?.public().to_peer_id().to_string();
    state
        .database
        .get_contact(&peer_id)?
        .ok_or("contato não encontrado")?;
    let conversation_id = conversation_id_for(&local_peer_id, &peer_id);
    state
        .database
        .list_direct_messages(&conversation_id, 200)?
        .into_iter()
        .map(|message| direct_message_view(&state, message))
        .collect()
}

#[tauri::command]
fn send_direct_message(
    peer_id: String,
    content: String,
    display_name: Option<String>,
    app: AppHandle,
    state: State<'_, NodeState>,
) -> Result<DirectMessageView, String> {
    let content = content.trim().to_string();
    if content.is_empty() || content.len() > 8_000 {
        return Err("mensagem privada vazia ou grande demais".into());
    }
    let contact = state
        .database
        .get_contact(&peer_id)?
        .ok_or("contato não encontrado")?;
    if contact.status != "accepted" {
        return Err("aceite a amizade antes de enviar mensagens privadas".into());
    }
    let local_peer_id = ensure_keypair(&state)?.public().to_peer_id().to_string();
    let conversation_id = conversation_id_for(&local_peer_id, &peer_id);
    let envelope = new_direct_envelope(
        &state,
        &contact_to_card(&contact),
        "direct_message",
        DirectBody {
            request_id: Uuid::new_v4().to_string(),
            conversation_id: conversation_id.clone(),
            display_name,
            contact_card: None,
            content: Some(content),
        },
    )?;
    let record = direct_record(&envelope);
    state.database.insert_direct_message(&record)?;
    let view = direct_message_view(&state, record)?;
    enqueue_and_publish_direct(&app, &state, envelope)?;
    Ok(view)
}

#[tauri::command]
fn get_network_status(state: State<'_, NodeState>) -> Result<NetworkStatusView, String> {
    let snapshot = state
        .snapshot
        .lock()
        .map_err(|_| "estado do node indisponível")?
        .clone();
    let relays = snapshot
        .relay_addresses
        .iter()
        .map(|address| RelayStatus {
            peer_id: address.parse::<Multiaddr>().ok().and_then(|multiaddr| {
                multiaddr.iter().find_map(|protocol| match protocol {
                    Protocol::P2p(peer_id) => Some(peer_id.to_string()),
                    _ => None,
                })
            }),
            state: if snapshot.relay_connected {
                "connected"
            } else if snapshot.is_running {
                "connecting"
            } else {
                "offline"
            }
            .into(),
            address: address.clone(),
            last_seen: snapshot.relay_connected.then_some(now_millis()),
        })
        .collect();
    Ok(NetworkStatusView {
        snapshot,
        relays,
        peers: get_known_peers(state)?,
    })
}

#[tauri::command]
fn test_peer_connection(
    address: String,
    app: AppHandle,
    state: State<'_, NodeState>,
) -> Result<ConnectionDiagnostic, String> {
    let address = address
        .parse::<Multiaddr>()
        .map_err(|error| format!("multiaddress inválido: {error}"))?;
    let peer_id = address
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::P2p(peer_id) => Some(peer_id.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "desconhecido".into());
    let sender = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
        .ok_or("node ainda não iniciou")?;
    sender
        .send(NodeCommand::Dial { address })
        .map_err(|_| "node não está aceitando conexões".to_string())?;
    let diagnostic = ConnectionDiagnostic {
        peer_id,
        state: "dialing".into(),
        detail: "tentativa de conexão enviada ao node".into(),
        checked_at: now_millis(),
    };
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "connection-diagnostic".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::to_value(&diagnostic).map_err(|error| error.to_string())?),
        },
    );
    Ok(diagnostic)
}

#[tauri::command]
fn run_network_diagnostics(state: State<'_, NodeState>) -> Result<NetworkDiagnostics, String> {
    let snapshot = state
        .snapshot
        .lock()
        .map_err(|_| "estado do node indisponível")?
        .clone();
    let mut checks = vec![
        NetworkDiagnosticCheck {
            name: "node".into(),
            status: if snapshot.is_running { "ok" } else { "error" }.into(),
            detail: if snapshot.is_running {
                "node iniciado e respondendo"
            } else {
                "node ainda não foi iniciado"
            }
            .into(),
        },
        NetworkDiagnosticCheck {
            name: "listener".into(),
            status: if !snapshot.listen_addresses.is_empty() {
                "ok"
            } else {
                "waiting"
            }
            .into(),
            detail: if snapshot.listen_addresses.is_empty() {
                "aguardando endereço de escuta".into()
            } else {
                format!(
                    "{} endereço(s) anunciado(s)",
                    snapshot.listen_addresses.len()
                )
            },
        },
        NetworkDiagnosticCheck {
            name: "peers".into(),
            status: if snapshot.connected_peers > 0 {
                "ok"
            } else {
                "waiting"
            }
            .into(),
            detail: if snapshot.connected_peers > 0 {
                format!(
                    "{} peer(s) conectado(s); Ping libp2p ativo",
                    snapshot.connected_peers
                )
            } else {
                "nenhum peer conectado — use um endereço ou bootstrap".into()
            },
        },
    ];
    let recovery_configured =
        !snapshot.bootstrap_addresses.is_empty() || !snapshot.relay_addresses.is_empty();
    checks.push(NetworkDiagnosticCheck {
        name: "recovery".into(),
        status: if recovery_configured { "ok" } else { "info" }.into(),
        detail: if recovery_configured {
            format!(
                "{} bootstrap(s) e {} relay(s) configurado(s)",
                snapshot.bootstrap_addresses.len(),
                snapshot.relay_addresses.len()
            )
        } else {
            "sem bootstrap/relay; conexão direta ainda funciona na LAN".into()
        },
    });
    if snapshot.is_running {
        let sync_probe = state
            .command_tx
            .lock()
            .map_err(|_| "controle do node indisponível")?
            .as_ref()
            .map(|sender| sender.send(NodeCommand::RequestSync).is_ok())
            .unwrap_or(false);
        checks.push(NetworkDiagnosticCheck {
            name: "sync-probe".into(),
            status: if sync_probe { "ok" } else { "error" }.into(),
            detail: if sync_probe {
                "sonda de sincronização enviada aos peers".into()
            } else {
                "node não aceitou a sonda de sincronização".into()
            },
        });
    }
    let status = if !snapshot.is_running {
        "offline"
    } else if snapshot.connected_peers > 0 {
        "healthy"
    } else {
        "waiting"
    };
    let summary = match status {
        "healthy" => "node saudável e conectado a peers",
        "waiting" => "node ativo, aguardando conexão com outro node",
        _ => "node offline",
    };
    Ok(NetworkDiagnostics {
        status: status.into(),
        summary: summary.into(),
        checks,
        connected_peers: snapshot.connected_peers,
        checked_at: now_millis(),
    })
}

#[tauri::command]
fn get_network_config(state: State<'_, NodeState>) -> Result<NetworkConfig, String> {
    state
        .network_config
        .lock()
        .map(|config| config.clone())
        .map_err(|_| "configuração de rede bloqueada".into())
}

#[tauri::command]
fn set_network_config(
    relay_addresses: Vec<String>,
    bootstrap_addresses: Vec<String>,
    state: State<'_, NodeState>,
) -> Result<NetworkConfig, String> {
    let previous = state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")?
        .clone();
    let normalized_relays = normalize_multiaddr_list(relay_addresses, "relay")?;
    let normalized_bootstrap = normalize_multiaddr_list(bootstrap_addresses, "bootstrap")?;
    let config = NetworkConfig {
        relay_addresses: normalized_relays,
        bootstrap_addresses: normalized_bootstrap,
    };
    save_network_config(&state.data_dir, &config)?;
    *state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")? = config.clone();
    if let Ok(mut snapshot) = state.snapshot.lock() {
        snapshot.relay_addresses = config.relay_addresses.clone();
        snapshot.bootstrap_addresses = config.bootstrap_addresses.clone();
    }
    if let Ok(sender) = state.command_tx.lock() {
        if let Some(sender) = sender.clone() {
            for address in &config.relay_addresses {
                if previous.relay_addresses.iter().any(|item| item == address) {
                    continue;
                }
                if let Ok(address) = address.parse::<Multiaddr>() {
                    let _ = sender.send(NodeCommand::ConfigureRelay { address });
                }
            }
            for address in &config.bootstrap_addresses {
                if previous
                    .bootstrap_addresses
                    .iter()
                    .any(|item| item == address)
                {
                    continue;
                }
                if let Ok(address) = address.parse::<Multiaddr>() {
                    let _ = sender.send(NodeCommand::Dial { address });
                }
            }
        }
    }
    Ok(config)
}

fn normalize_multiaddr_list(addresses: Vec<String>, kind: &str) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for address in addresses {
        let trimmed = address.trim();
        if trimmed.is_empty() {
            continue;
        }
        trimmed
            .parse::<Multiaddr>()
            .map_err(|error| format!("{kind} inválido: {error}"))?;
        if !normalized.iter().any(|item: &String| item == trimmed) {
            normalized.push(trimmed.to_string());
        }
    }
    Ok(normalized)
}

#[tauri::command]
fn get_media_config(state: State<'_, NodeState>) -> Result<MediaConfig, String> {
    state
        .media_config
        .lock()
        .map(|config| config.clone())
        .map_err(|_| "configuração de mídia bloqueada".into())
}

#[tauri::command]
fn set_media_config(
    ice_servers: Vec<IceServerConfig>,
    state: State<'_, NodeState>,
) -> Result<MediaConfig, String> {
    let config = MediaConfig {
        ice_servers: ice_servers
            .into_iter()
            .map(validate_ice_server)
            .collect::<Result<Vec<_>, _>>()?,
    };
    save_media_config(&config)?;
    *state
        .media_config
        .lock()
        .map_err(|_| "configuração de mídia bloqueada")? = config.clone();
    Ok(config)
}

#[tauri::command]
fn get_groups(state: State<'_, NodeState>) -> Result<Vec<GroupView>, String> {
    let local_peer_id = ensure_keypair(&state)?.public().to_peer_id().to_string();
    state
        .database
        .list_groups()?
        .into_iter()
        .filter(|group| {
            state
                .database
                .get_member(&group.id, &local_peer_id)
                .ok()
                .flatten()
                .is_some_and(|member| member.status == "active")
        })
        .map(|group| group_view(&state.database, group))
        .collect()
}

#[tauri::command]
fn create_group(name: String, state: State<'_, NodeState>) -> Result<GroupCreateResult, String> {
    let name = validate_group_name(name)?;
    let keypair = ensure_keypair(&state)?;
    let group_id = format!("group-{}", Uuid::new_v4());
    let group = GroupRecord {
        id: group_id.clone(),
        initials: initials_for(&name),
        name: name.clone(),
        color: "#8276ff".into(),
        owner_peer_id: keypair.public().to_peer_id().to_string(),
        created_at: now_millis(),
        current_key_epoch: 1,
    };
    let group_key = new_group_key()?;
    let (_, agreement_public) = ensure_agreement_keypair()?;
    state.database.insert_group(&group)?;
    state.database.insert_default_channels(&group.id)?;
    store_group_key_epoch(&group.id, 1, &group_key)?;
    state.database.insert_key_epoch(
        &group.id,
        1,
        &group.owner_peer_id,
        group.created_at,
        &hash_key(&group_key),
    )?;
    state.database.insert_member(&MemberRecord {
        group_id: group.id.clone(),
        peer_id: group.owner_peer_id.clone(),
        display_name: "Dono".into(),
        role: Role::Owner,
        public_key: keypair.public().encode_protobuf(),
        x25519_public_key: agreement_public.to_vec(),
        status: "active".into(),
        joined_at: group.created_at,
        timeout_until: None,
        updated_at: group.created_at,
    })?;
    subscribe_group_if_running(&state, &group.id);
    let invite = crypto::create_invite_with_contacts(
        &keypair,
        &group.id,
        &group.name,
        &group.initials,
        &group.color,
        &group_key,
        group.current_key_epoch,
        &BASE64.encode(agreement_public),
        &contact_addresses(&state),
        now_seconds() + 30 * 24 * 60 * 60,
    )?;
    Ok(GroupCreateResult {
        group: group_view(&state.database, group)?,
        invite,
    })
}

#[tauri::command]
fn create_invite(group_id: String, state: State<'_, NodeState>) -> Result<String, String> {
    let group = state
        .database
        .get_group(&group_id)?
        .ok_or("grupo não encontrado")?;
    let group_key = load_group_key_epoch(&group.id, group.current_key_epoch)?;
    let keypair = ensure_keypair(&state)?;
    let actor_peer_id = keypair.public().to_peer_id().to_string();
    let actor = state
        .database
        .get_member(&group.id, &actor_peer_id)?
        .ok_or("este node não é membro do grupo")?;
    if !has_permission(&actor.role, &Permission::ManageInvites) {
        return Err("membro sem permissão para criar convites".into());
    }
    let expires_at = now_seconds() + 30 * 24 * 60 * 60;
    if actor_peer_id == group.owner_peer_id {
        let (_, agreement_public) = ensure_agreement_keypair()?;
        crypto::create_invite_with_contacts(
            &keypair,
            &group.id,
            &group.name,
            &group.initials,
            &group.color,
            &group_key,
            group.current_key_epoch,
            &BASE64.encode(agreement_public),
            &contact_addresses(&state),
            expires_at,
        )
    } else {
        let owner = state
            .database
            .get_member(&group.id, &group.owner_peer_id)?
            .ok_or("identidade do Owner ainda não sincronizada")?;
        if owner.x25519_public_key.len() != X25519_KEY_BYTES {
            return Err("chave de acordo do Owner ainda não sincronizada".into());
        }
        crypto::create_delegated_invite_with_contacts(
            &keypair,
            &group.owner_peer_id,
            &BASE64.encode(owner.public_key),
            &group.id,
            &group.name,
            &group.initials,
            &group.color,
            &group_key,
            group.current_key_epoch,
            &BASE64.encode(owner.x25519_public_key.as_slice()),
            &contact_addresses(&state),
            expires_at,
        )
    }
}

#[tauri::command]
fn join_group(
    app: AppHandle,
    invite: String,
    state: State<'_, NodeState>,
) -> Result<GroupView, String> {
    let payload = crypto::decode_invite(&invite)?;
    let group_key = validate_invite(&payload, now_seconds())?;
    let owner_public_key = BASE64
        .decode(&payload.owner_public_key)
        .map_err(|error| format!("chave pública do Owner inválida: {error}"))?;
    let owner_x25519_public_key = payload
        .owner_x25519_public_key
        .as_deref()
        .map(|encoded| {
            BASE64
                .decode(encoded)
                .map_err(|error| format!("chave de acordo do Owner inválida: {error}"))
        })
        .transpose()?
        .unwrap_or_default();
    if !owner_x25519_public_key.is_empty() && owner_x25519_public_key.len() != X25519_KEY_BYTES {
        return Err("chave de acordo do Owner inválida".into());
    }
    let keypair = ensure_keypair(&state)?;
    let local_peer_id = keypair.public().to_peer_id().to_string();
    if let Some(existing_member) = state
        .database
        .get_member(&payload.group_id, &local_peer_id)?
    {
        if existing_member.status == "banned" {
            return Err("este node está banido deste grupo".into());
        }
        if existing_member.status == "active" {
            return Err("este node já é membro ativo deste grupo".into());
        }
    }
    if let Some(existing_group) = state.database.get_group(&payload.group_id)? {
        if payload.key_epoch < existing_group.current_key_epoch {
            return Err("o convite usa uma época antiga do grupo".into());
        }
    }
    let group = GroupRecord {
        id: payload.group_id,
        name: payload.group_name,
        initials: payload.initials,
        color: payload.color,
        owner_peer_id: payload.owner_peer_id,
        created_at: now_millis(),
        current_key_epoch: payload.key_epoch.max(1),
    };
    state.database.insert_group(&group)?;
    state
        .database
        .set_current_key_epoch(&group.id, group.current_key_epoch)?;
    state.database.insert_default_channels(&group.id)?;
    store_group_key_epoch(&group.id, group.current_key_epoch, &group_key)?;
    state.database.insert_key_epoch(
        &group.id,
        group.current_key_epoch,
        &group.owner_peer_id,
        group.created_at,
        &hash_key(&group_key),
    )?;
    if group.owner_peer_id != local_peer_id {
        state.database.insert_member(&MemberRecord {
            group_id: group.id.clone(),
            peer_id: group.owner_peer_id.clone(),
            display_name: "Dono".into(),
            role: Role::Owner,
            public_key: owner_public_key,
            x25519_public_key: owner_x25519_public_key,
            status: "active".into(),
            joined_at: group.created_at,
            timeout_until: None,
            updated_at: group.created_at,
        })?;
    }
    let (_, agreement_public) = ensure_agreement_keypair()?;
    state.database.insert_member(&MemberRecord {
        group_id: group.id.clone(),
        peer_id: keypair.public().to_peer_id().to_string(),
        display_name: "Membro".into(),
        role: Role::Member,
        public_key: keypair.public().encode_protobuf(),
        x25519_public_key: agreement_public.to_vec(),
        status: "active".into(),
        joined_at: group.created_at,
        timeout_until: None,
        updated_at: group.created_at,
    })?;
    let mut event = new_control_event(
        &state,
        &group.id,
        CONTROL_MEMBER_JOINED,
        Some(keypair.public().to_peer_id().to_string()),
        serde_json::json!({
            "display_name": "Membro",
            "public_key": BASE64.encode(keypair.public().encode_protobuf()),
            "x25519_public_key": BASE64.encode(agreement_public),
        }),
    )?;
    attach_join_proof(&state, &mut event, invite.trim())?;
    if state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .is_some()
    {
        submit_control_event(&app, &state, event)?;
    }
    subscribe_group_if_running(&state, &group.id);
    for raw_address in &payload.contact_addresses {
        let Ok(address) = raw_address.parse::<Multiaddr>() else {
            continue;
        };
        if let Some(peer_id) = address.iter().find_map(|protocol| match protocol {
            Protocol::P2p(peer_id) => Some(peer_id),
            _ => None,
        }) {
            let _ = state.database.remember_peer_address(
                &peer_id.to_string(),
                raw_address,
                "invite",
                now_millis(),
            );
        }
        if let Ok(sender) = state.command_tx.lock() {
            if let Some(sender) = sender.clone() {
                let _ = sender.send(NodeCommand::Dial { address });
            }
        }
    }
    group_view(&state.database, group)
}

#[tauri::command]
fn list_group_members(
    group_id: String,
    state: State<'_, NodeState>,
) -> Result<Vec<MemberRecord>, String> {
    state.database.list_members(&group_id)
}

#[tauri::command]
fn get_group_roles() -> Result<Vec<RoleView>, String> {
    Ok([Role::Owner, Role::Admin, Role::Mod, Role::Member]
        .into_iter()
        .map(|role| RoleView {
            permissions: access::permissions_for(&role),
            role,
        })
        .collect())
}

#[tauri::command]
fn get_channel_permissions(
    group_id: String,
    channel_id: String,
    state: State<'_, NodeState>,
) -> Result<Vec<ChannelPermissionRecord>, String> {
    let channel = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal não encontrado")?;
    let stored = state
        .database
        .list_channel_permissions(&group_id, &channel_id)?;
    [Role::Owner, Role::Admin, Role::Mod, Role::Member]
        .into_iter()
        .map(|role| {
            if let Some(permission) = stored.iter().find(|item| item.role == role) {
                return Ok(permission.clone());
            }
            Ok(ChannelPermissionRecord {
                group_id: group_id.clone(),
                channel_id: channel_id.clone(),
                role: role.clone(),
                can_join_voice: channel.kind == "voice"
                    && has_permission(&role, &Permission::JoinVoice),
                can_share_screen: channel.kind == "voice"
                    && has_permission(&role, &Permission::ShareScreen),
            })
        })
        .collect()
}

#[tauri::command]
fn set_channel_permission(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    role: String,
    can_join_voice: bool,
    can_share_screen: bool,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ManageChannels) {
        return Err("membro sem permissão para alterar permissões do canal".into());
    }
    let role = Role::parse(&role)?;
    let channel = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal não encontrado")?;
    let permission = ChannelPermissionRecord {
        group_id: group_id.clone(),
        channel_id: channel_id.clone(),
        role,
        can_join_voice: channel.kind == "voice" && can_join_voice,
        can_share_screen: channel.kind == "voice" && can_share_screen,
    };
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_CHANNEL_PERMISSION_UPDATED,
        None,
        serde_json::to_value(&permission).map_err(|error| error.to_string())?,
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn get_audit_log(
    group_id: String,
    limit: Option<u32>,
    state: State<'_, NodeState>,
) -> Result<Vec<AuditEventRecord>, String> {
    state
        .database
        .list_audit_events(&group_id, limit.unwrap_or(200))
}

#[tauri::command]
fn update_member_role(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    role: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let role = Role::parse(&role)?;
    let actor = local_member(&state, &group_id)?;
    let target = state
        .database
        .get_member(&group_id, &peer_id)?
        .ok_or("membro não encontrado")?;
    if target.status != "active"
        || peer_id == actor.peer_id
        || !role_change_is_authorized(&state, &group_id, &actor, &target, &role)?
    {
        return Err("membro sem autoridade para alterar este cargo".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_ROLE_CHANGED,
        Some(peer_id.clone()),
        serde_json::json!({ "role": role.as_str() }),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn transfer_group_ownership(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let group = state
        .database
        .get_group(&group_id)?
        .ok_or("grupo não encontrado")?;
    let actor = local_member(&state, &group_id)?;
    if actor.peer_id != group.owner_peer_id || actor.role != Role::Owner {
        return Err("somente o Owner pode transferir a propriedade".into());
    }
    let target = state
        .database
        .get_member(&group_id, &peer_id)?
        .ok_or("membro alvo não encontrado")?;
    if target.status != "active" || peer_id == actor.peer_id {
        return Err("o novo Owner precisa ser outro membro ativo".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_GROUP_OWNER_TRANSFERRED,
        Some(peer_id),
        serde_json::json!({}),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn delete_group(
    app: AppHandle,
    group_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let group = state
        .database
        .get_group(&group_id)?
        .ok_or("grupo não encontrado")?;
    let actor = local_member(&state, &group_id)?;
    if actor.peer_id != group.owner_peer_id || actor.role != Role::Owner {
        return Err("somente o Owner pode excluir o grupo".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_GROUP_DELETED,
        None,
        serde_json::json!({}),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn kick_member(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    moderate_member(&app, &state, &group_id, &peer_id, CONTROL_MEMBER_KICKED)
}

#[tauri::command]
fn ban_member(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    moderate_member(&app, &state, &group_id, &peer_id, CONTROL_MEMBER_BANNED)
}

#[tauri::command]
fn unban_member(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ManageMembers) {
        return Err("membro sem permissão para alterar membros".into());
    }
    let target = state
        .database
        .get_member(&group_id, &peer_id)?
        .ok_or("membro não encontrado")?;
    if target.status != "banned" {
        return Err("somente um membro banido pode ser desbanido".into());
    }
    if target.x25519_public_key.len() != X25519_KEY_BYTES {
        return Err("membro não possui chave de acordo para receber a época atual".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_MEMBER_UNBANNED,
        Some(peer_id.clone()),
        serde_json::json!({}),
    )?;
    submit_control_event(&app, &state, event)?;
    share_current_group_key(&app, &state, &group_id, &peer_id)
}

#[tauri::command]
fn timeout_member(
    app: AppHandle,
    group_id: String,
    peer_id: String,
    duration_seconds: i64,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ModerateMessages) {
        return Err("membro sem permissão para aplicar timeout".into());
    }
    let target = state
        .database
        .get_member(&group_id, &peer_id)?
        .ok_or("membro não encontrado")?;
    if target.status != "active" || !can_manage_target(&actor.role, &target.role) {
        return Err("não é possível aplicar timeout neste cargo".into());
    }
    let until = now_seconds() + duration_seconds.clamp(1, 7 * 24 * 60 * 60);
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_MEMBER_TIMEOUT,
        Some(peer_id),
        serde_json::json!({ "until": until }),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn leave_group(
    app: AppHandle,
    group_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if actor.role == Role::Owner {
        return Err("o Owner precisa transferir a propriedade antes de sair".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_MEMBER_LEFT,
        Some(actor.peer_id),
        serde_json::json!({}),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn create_channel(
    app: AppHandle,
    group_id: String,
    name: String,
    kind: String,
    state: State<'_, NodeState>,
) -> Result<ChannelRecord, String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ManageChannels) {
        return Err("membro sem permissão para criar canais".into());
    }
    let channel = ChannelRecord {
        id: format!("{group_id}:{}", Uuid::new_v4()),
        group_id: group_id.clone(),
        name: validate_channel_name(name)?,
        kind: validate_channel_kind(&kind)?,
        position: state.database.list_channels(&group_id)?.len() as i32,
    };
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_CHANNEL_CREATED,
        None,
        serde_json::to_value(&channel).map_err(|error| error.to_string())?,
    )?;
    submit_control_event(&app, &state, event)?;
    Ok(channel)
}

#[tauri::command]
fn update_channel(
    app: AppHandle,
    channel: ChannelRecord,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &channel.group_id)?;
    if !has_permission(&actor.role, &Permission::ManageChannels) {
        return Err("membro sem permissão para editar canais".into());
    }
    let mut normalized = channel;
    normalized.name = validate_channel_name(normalized.name)?;
    normalized.kind = validate_channel_kind(&normalized.kind)?;
    let group_id = normalized.group_id.clone();
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_CHANNEL_UPDATED,
        None,
        serde_json::to_value(normalized).map_err(|error| error.to_string())?,
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn delete_channel(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ManageChannels) {
        return Err("membro sem permissão para excluir canais".into());
    }
    let channels = state.database.list_channels(&group_id)?;
    if channels
        .iter()
        .any(|channel| channel.id == channel_id && channel.name == "geral")
    {
        return Err("o canal geral não pode ser excluído".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_CHANNEL_DELETED,
        None,
        serde_json::json!({ "channel_id": channel_id }),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn reorder_channels(
    app: AppHandle,
    group_id: String,
    channel_ids: Vec<String>,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ManageChannels) {
        return Err("membro sem permissão para reorganizar canais".into());
    }
    let existing = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .map(|channel| channel.id)
        .collect::<HashSet<_>>();
    let requested = channel_ids.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != channel_ids.len() || requested != existing {
        return Err("a ordenação precisa conter exatamente todos os canais do grupo".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_CHANNEL_REORDERED,
        None,
        serde_json::json!({ "channel_ids": channel_ids }),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn join_call(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    call_id: Option<String>,
    display_name: Option<String>,
    state: State<'_, NodeState>,
) -> Result<CallState, String> {
    let member = local_member(&state, &group_id)?;
    let call_display_name = match display_name {
        Some(value) => validate_display_name(value)?,
        None => member.display_name.clone(),
    };
    if member.status != "active"
        || timeout_is_active(member.timeout_until, now_seconds())
        || !has_permission(&member.role, &Permission::JoinVoice)
    {
        return Err("membro sem permissão para entrar em call".into());
    }
    let channel = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal de call não encontrado")?;
    if channel.kind == "text" {
        return Err("canal de texto não aceita call".into());
    }
    if !effective_channel_permission(&state, &group_id, &channel_id, &member.role)?.can_join_voice {
        return Err("seu cargo não pode entrar nesta call".into());
    }
    let call_id = call_id.unwrap_or_else(|| format!("{group_id}:{channel_id}"));
    let call_key = format!("{group_id}:{channel_id}:{call_id}");
    let signal = new_call_signal(
        &state,
        &group_id,
        &channel_id,
        &call_id,
        None,
        "join",
        serde_json::json!({ "display_name": call_display_name.clone() }),
    )?;
    let call_state = {
        let mut states = state
            .call_states
            .lock()
            .map_err(|_| "estado de call bloqueado")?;
        let current = states.entry(call_key.clone()).or_insert_with(|| CallState {
            group_id: group_id.clone(),
            channel_id: channel_id.clone(),
            call_id: call_id.clone(),
            participants: Vec::new(),
        });
        if !current
            .participants
            .iter()
            .any(|participant| participant.peer_id == member.peer_id)
        {
            if current.participants.len() >= MAX_CALL_PARTICIPANTS {
                return Err("esta call já atingiu o limite de 8 participantes".into());
            }
            current.participants.push(CallParticipant {
                peer_id: member.peer_id.clone(),
                display_name: call_display_name.clone(),
                role: member.role.clone(),
                muted: false,
                sharing_screen: false,
                connection_state: "connected".into(),
            });
        }
        current.clone()
    };
    if let Err(error) = publish_call_signal(&app, &state, signal) {
        let mut states = state
            .call_states
            .lock()
            .map_err(|_| "estado de call bloqueado")?;
        if let Some(current) = states.get_mut(&call_key) {
            current
                .participants
                .retain(|participant| participant.peer_id != member.peer_id);
            if current.participants.is_empty() {
                states.remove(&call_key);
            }
        }
        return Err(error);
    }
    emit_call_state(&app, &state, &call_state)?;
    request_presence_broadcast(&state);
    Ok(call_state)
}

#[tauri::command]
fn leave_call(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    call_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let member = local_member(&state, &group_id)?;
    let signal = new_call_signal(
        &state,
        &group_id,
        &channel_id,
        &call_id,
        None,
        "leave",
        serde_json::json!({}),
    )?;
    let call_key = format!("{group_id}:{channel_id}:{call_id}");
    publish_call_signal(&app, &state, signal)?;
    let remaining = {
        let mut states = state
            .call_states
            .lock()
            .map_err(|_| "estado de call bloqueado")?;
        let Some(current) = states.get_mut(&call_key) else {
            return Ok(());
        };
        current
            .participants
            .retain(|participant| participant.peer_id != member.peer_id);
        let snapshot = current.clone();
        if current.participants.is_empty() {
            states.remove(&call_key);
        }
        snapshot
    };
    emit_call_state(&app, &state, &remaining)?;
    request_presence_broadcast(&state);
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn send_call_signal(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    call_id: String,
    to_peer_id: Option<String>,
    kind: String,
    payload: serde_json::Value,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let member = local_member(&state, &group_id)?;
    if member.status != "active" || timeout_is_active(member.timeout_until, now_seconds()) {
        return Err("membro não pode enviar sinal de call".into());
    }
    let channel = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal de call não encontrado")?;
    if channel.kind == "text" {
        return Err("canal de texto não aceita sinal de call".into());
    }
    let channel_permission =
        effective_channel_permission(&state, &group_id, &channel_id, &member.role)?;
    let call_key = format!("{group_id}:{channel_id}:{call_id}");
    let local_in_call = state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?
        .get(&call_key)
        .is_some_and(|current| {
            current
                .participants
                .iter()
                .any(|participant| participant.peer_id == member.peer_id)
        });
    if !local_in_call {
        return Err("membro não está nesta call".into());
    }
    if let Some(target_peer_id) = to_peer_id.as_deref() {
        let target = state
            .database
            .get_member(&group_id, target_peer_id)?
            .ok_or("destinatário de sinal não encontrado")?;
        if target.status != "active" {
            return Err("destinatário de sinal não está ativo".into());
        }
    }
    if kind == "screen-start" {
        if !channel_permission.can_share_screen {
            return Err("membro sem permissão para compartilhar tela".into());
        }
        if state
            .call_states
            .lock()
            .map_err(|_| "estado de call bloqueado")?
            .get(&call_key)
            .is_some_and(|current| {
                current.participants.iter().any(|participant| {
                    participant.peer_id != member.peer_id && participant.sharing_screen
                })
            })
        {
            return Err("já existe outra tela compartilhada nesta call".into());
        }
    }
    let signal = new_call_signal(
        &state,
        &group_id,
        &channel_id,
        &call_id,
        to_peer_id,
        &kind,
        payload,
    )?;
    publish_call_signal(&app, &state, signal)
}

#[tauri::command]
fn request_call_mute(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    call_id: String,
    peer_id: String,
    muted: bool,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if !has_permission(&actor.role, &Permission::ModerateMessages) {
        return Err("membro sem permissão para silenciar participantes".into());
    }
    let target = state
        .database
        .get_member(&group_id, &peer_id)?
        .ok_or("membro alvo não encontrado")?;
    if target.status != "active" || !can_manage_target(&actor.role, &target.role) {
        return Err("não é possível silenciar este cargo".into());
    }
    let call_key = format!("{group_id}:{channel_id}:{call_id}");
    let in_call = state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?
        .get(&call_key)
        .is_some_and(|current| {
            current
                .participants
                .iter()
                .any(|participant| participant.peer_id == actor.peer_id)
                && current
                    .participants
                    .iter()
                    .any(|participant| participant.peer_id == peer_id)
        });
    if !in_call {
        return Err("o membro alvo não está nesta call".into());
    }
    let signal = new_call_signal(
        &state,
        &group_id,
        &channel_id,
        &call_id,
        Some(peer_id),
        "mute-request",
        serde_json::json!({ "muted": muted }),
    )?;
    publish_call_signal(&app, &state, signal)
}

#[tauri::command]
fn get_call_state(
    group_id: String,
    channel_id: String,
    call_id: String,
    state: State<'_, NodeState>,
) -> Result<CallState, String> {
    let key = format!("{group_id}:{channel_id}:{call_id}");
    Ok(state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?
        .get(&key)
        .cloned()
        .unwrap_or(CallState {
            group_id,
            channel_id,
            call_id,
            participants: Vec::new(),
        }))
}

#[tauri::command]
fn get_messages(
    group_id: String,
    channel_id: String,
    limit: Option<u32>,
    state: State<'_, NodeState>,
) -> Result<Vec<ChatMessage>, String> {
    let mut messages = Vec::new();
    for envelope in
        state
            .database
            .list_messages(&group_id, &channel_id, limit.unwrap_or(200).min(500))?
    {
        if verify_author(
            &envelope.author_public_key,
            &envelope.author_peer_id,
            &signing_bytes(&envelope)?,
            &envelope.signature,
        )
        .is_err()
        {
            continue;
        }
        if let Ok(group_key) = load_group_key_epoch(&group_id, envelope.key_epoch) {
            if let Ok(message) = envelope_to_chat(&envelope, &group_key) {
                messages.push(message);
            }
        }
    }
    Ok(messages)
}

#[tauri::command]
fn search_messages(
    group_id: String,
    query: String,
    limit: Option<u32>,
    state: State<'_, NodeState>,
) -> Result<Vec<SearchResult>, String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let max = limit.unwrap_or(50).clamp(1, 100) as usize;
    let mut results = Vec::new();
    for channel in state.database.list_channels(&group_id)? {
        if channel.name.to_lowercase().contains(&query) {
            results.push(SearchResult {
                kind: "channel".into(),
                id: channel.id.clone(),
                title: format!(
                    "{}{}",
                    if channel.kind == "text" { "#" } else { "🔊 " },
                    channel.name
                ),
                subtitle: if channel.kind == "voice" {
                    "voz + chat + tela".into()
                } else {
                    "canal de texto".into()
                },
                group_id: group_id.clone(),
                channel_id: Some(channel.id.clone()),
                created_at: None,
            });
        }
        for envelope in state.database.list_messages(&group_id, &channel.id, 500)? {
            if results.len() >= max {
                break;
            }
            let Ok(group_key) = load_group_key_epoch(&group_id, envelope.key_epoch) else {
                continue;
            };
            let Ok(signed) = signing_bytes(&envelope) else {
                continue;
            };
            if verify_author(
                &envelope.author_public_key,
                &envelope.author_peer_id,
                &signed,
                &envelope.signature,
            )
            .is_err()
            {
                continue;
            }
            let Ok(message) = envelope_to_chat(&envelope, &group_key) else {
                continue;
            };
            if message.content.to_lowercase().contains(&query)
                || message.author.to_lowercase().contains(&query)
            {
                results.push(SearchResult {
                    kind: "message".into(),
                    id: message.id,
                    title: format!("{} · #{}", message.author, channel.name),
                    subtitle: message.content,
                    group_id: group_id.clone(),
                    channel_id: Some(channel.id.clone()),
                    created_at: Some(message.created_at),
                });
            }
        }
    }
    for member in state.database.list_members(&group_id)? {
        if results.len() >= max || !member.display_name.to_lowercase().contains(&query) {
            continue;
        }
        results.push(SearchResult {
            kind: "member".into(),
            id: member.peer_id.clone(),
            title: member.display_name,
            subtitle: format!("{} · {}", member.role.as_str(), member.peer_id),
            group_id: group_id.clone(),
            channel_id: None,
            created_at: None,
        });
    }
    Ok(results.into_iter().take(max).collect())
}

#[tauri::command]
fn send_message(
    group_id: String,
    channel_id: String,
    author_name: String,
    content: String,
    state: State<'_, NodeState>,
) -> Result<ChatMessage, String> {
    let content = content.trim().to_string();
    if content.is_empty() || content.len() > 4000 {
        return Err("mensagem deve ter entre 1 e 4000 caracteres".into());
    }
    let member = local_member(&state, &group_id)?;
    if member.status != "active"
        || timeout_is_active(member.timeout_until, now_seconds())
        || !has_permission(&member.role, &Permission::SendMessages)
    {
        return Err("este membro não pode enviar mensagens neste momento".into());
    }
    let channel = state
        .database
        .list_channels(&group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal não encontrado")?;
    if channel.kind != "text" && channel.kind != "voice" {
        return Err("este tipo de canal não aceita mensagens de texto".into());
    }
    let keypair = ensure_keypair(&state)?;
    let group_epoch = current_group_epoch(&state.database, &group_id)?;
    let group_key = load_group_key_epoch(&group_id, group_epoch)?;
    let effective_author = if author_name.trim().is_empty() {
        "Você".to_string()
    } else {
        author_name.trim().to_string()
    };
    let local_peer_id = keypair.public().to_peer_id().to_string();
    let mut envelope = MessageEnvelope {
        event_id: Uuid::new_v4().to_string(),
        group_id,
        channel_id,
        author_peer_id: local_peer_id.clone(),
        author_public_key: keypair.public().encode_protobuf(),
        author_name: effective_author.clone(),
        initials: initials_for(&effective_author),
        color: "#8276ff".into(),
        created_at: now_millis(),
        key_epoch: group_epoch,
        nonce: Vec::new(),
        ciphertext: Vec::new(),
        signature: Vec::new(),
    };
    let (nonce, ciphertext) = encrypt_message(&group_key, &content, &aad_for_envelope(&envelope)?)?;
    envelope.nonce = nonce;
    envelope.ciphertext = ciphertext;
    envelope.signature = keypair
        .sign(&signing_bytes(&envelope)?)
        .map_err(|error| format!("não foi possível assinar mensagem: {error}"))?;
    let sender = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
        .ok_or("node ainda não iniciou")?;
    if !state.database.insert_message(&envelope)? {
        return Err("evento de mensagem duplicado".into());
    }
    state.database.enqueue_outbox(
        &envelope.event_id,
        "message",
        &serde_json::to_string(&envelope)
            .map_err(|error| format!("não foi possível preparar mensagem: {error}"))?,
        envelope.created_at,
    )?;
    let _ = sender.send(NodeCommand::Publish {
        envelope: Box::new(envelope.clone()),
    });
    let mut message = envelope_to_chat(&envelope, &group_key)?;
    message.mine = message.author_peer_id == local_peer_id;
    Ok(message)
}

#[tauri::command]
fn delete_message(
    app: AppHandle,
    group_id: String,
    channel_id: String,
    message_id: String,
    state: State<'_, NodeState>,
) -> Result<(), String> {
    let actor = local_member(&state, &group_id)?;
    if actor.status != "active" {
        return Err("membro não está ativo neste grupo".into());
    }
    let message = state
        .database
        .get_message(&message_id)?
        .ok_or("mensagem não encontrada")?;
    if message.group_id != group_id
        || message.channel_id != channel_id
        || (message.author_peer_id != actor.peer_id
            && !has_permission(&actor.role, &Permission::ModerateMessages))
    {
        return Err("membro sem permissão para apagar esta mensagem".into());
    }
    let event = new_control_event(
        &state,
        &group_id,
        CONTROL_MESSAGE_DELETED,
        Some(message.author_peer_id),
        serde_json::json!({
            "message_id": message_id,
            "channel_id": channel_id,
        }),
    )?;
    submit_control_event(&app, &state, event)
}

#[tauri::command]
fn connect_peer(address: String, state: State<'_, NodeState>) -> Result<(), String> {
    let address = address
        .parse::<Multiaddr>()
        .map_err(|error| format!("multiaddress inválido: {error}"))?;
    let sender = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
        .ok_or("node ainda não iniciou")?;
    sender
        .send(NodeCommand::Dial { address })
        .map_err(|_| "node não está aceitando conexões".into())
}

fn local_member(state: &NodeState, group_id: &str) -> Result<MemberRecord, String> {
    let peer_id = ensure_keypair(state)?.public().to_peer_id().to_string();
    state
        .database
        .get_member(group_id, &peer_id)?
        .ok_or_else(|| "este node não é membro do grupo".into())
}

fn effective_channel_permission(
    state: &NodeState,
    group_id: &str,
    channel_id: &str,
    role: &Role,
) -> Result<ChannelPermissionRecord, String> {
    let channel = state
        .database
        .list_channels(group_id)?
        .into_iter()
        .find(|channel| channel.id == channel_id)
        .ok_or("canal não encontrado")?;
    if let Some(permission) = state.database.get_channel_permission(channel_id, role)? {
        return Ok(permission);
    }
    Ok(ChannelPermissionRecord {
        group_id: group_id.into(),
        channel_id: channel_id.into(),
        role: role.clone(),
        can_join_voice: channel.kind == "voice" && has_permission(role, &Permission::JoinVoice),
        can_share_screen: channel.kind == "voice" && has_permission(role, &Permission::ShareScreen),
    })
}

fn ensure_legacy_owner_memberships(state: &NodeState, keypair: &Keypair) -> Result<(), String> {
    let peer_id = keypair.public().to_peer_id().to_string();
    let (_, agreement_public) = ensure_agreement_keypair()?;
    for group in state.database.list_groups()? {
        if group.owner_peer_id != peer_id
            || state.database.get_member(&group.id, &peer_id)?.is_some()
        {
            continue;
        }
        state.database.insert_member(&MemberRecord {
            group_id: group.id,
            peer_id: peer_id.clone(),
            display_name: "Dono".into(),
            role: Role::Owner,
            public_key: keypair.public().encode_protobuf(),
            x25519_public_key: agreement_public.to_vec(),
            status: "active".into(),
            joined_at: group.created_at,
            timeout_until: None,
            updated_at: now_millis(),
        })?;
    }
    Ok(())
}

fn can_manage_target(actor: &Role, target: &Role) -> bool {
    match actor {
        Role::Owner => *target != Role::Owner,
        Role::Admin => matches!(target, Role::Mod | Role::Member),
        Role::Mod => *target == Role::Member,
        Role::Member => false,
    }
}

fn can_update_role(actor: &Role, target: &Role, next: &Role) -> bool {
    match actor {
        Role::Owner => *target != Role::Owner && *next != Role::Owner,
        Role::Admin => {
            matches!(target, Role::Mod | Role::Member) && matches!(next, Role::Mod | Role::Member)
        }
        Role::Mod | Role::Member => false,
    }
}

fn role_change_is_authorized(
    state: &NodeState,
    group_id: &str,
    actor: &MemberRecord,
    target: &MemberRecord,
    next: &Role,
) -> Result<bool, String> {
    if !has_permission(&actor.role, &Permission::ManageRoles)
        || !can_update_role(&actor.role, &target.role, next)
    {
        return Ok(false);
    }
    if actor.role == Role::Owner {
        return Ok(true);
    }
    let group = state
        .database
        .get_group(group_id)?
        .ok_or("grupo não encontrado")?;
    state
        .database
        .owner_authorized_admin(group_id, &group.owner_peer_id, &actor.peer_id)
}

fn timeout_is_active(timeout_until: Option<i64>, now_seconds: i64) -> bool {
    timeout_until.is_some_and(|until| until > now_seconds)
}

fn join_proof_aad(event: &ControlEvent) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        "teamscord-join-proof-v1",
        &event.event_id,
        &event.group_id,
        event.key_epoch,
        &event.target_peer_id,
    ))
    .map_err(|error| format!("AAD de convite de entrada inválido: {error}"))
}

fn attach_join_proof(
    state: &NodeState,
    event: &mut ControlEvent,
    invite: &str,
) -> Result<(), String> {
    if event.kind != CONTROL_MEMBER_JOINED {
        return Err("prova de entrada só pode ser anexada a um ingresso".into());
    }
    let group_key = load_group_key_epoch(&event.group_id, event.key_epoch)?;
    let (nonce, ciphertext) = encrypt_message(&group_key, invite, &join_proof_aad(event)?)?;
    let payload = event
        .payload
        .as_object_mut()
        .ok_or("payload de entrada inválido")?;
    payload.insert(
        "invite_nonce".into(),
        serde_json::json!(BASE64.encode(nonce)),
    );
    payload.insert(
        "invite_ciphertext".into(),
        serde_json::json!(BASE64.encode(ciphertext)),
    );
    let keypair = ensure_keypair(state)?;
    sign_control_event(event, &keypair)
}

fn validate_join_event(state: &NodeState, event: &ControlEvent) -> Result<(), String> {
    if event.kind != CONTROL_MEMBER_JOINED
        || event.target_peer_id.as_deref() != Some(event.issuer_peer_id.as_str())
    {
        return Err("evento de entrada não é autoassinado pelo novo membro".into());
    }
    let target_peer_id = event
        .target_peer_id
        .as_deref()
        .ok_or("membro alvo ausente")?;
    let public_key_encoded = event
        .payload
        .get("public_key")
        .and_then(serde_json::Value::as_str)
        .ok_or("chave pública de membro ausente")?;
    let public_key = BASE64
        .decode(public_key_encoded)
        .map_err(|error| format!("chave pública de membro inválida: {error}"))?;
    let public_key_value = libp2p::identity::PublicKey::try_decode_protobuf(&public_key)
        .map_err(|error| format!("chave pública de membro inválida: {error}"))?;
    if public_key_value.to_peer_id().to_string() != target_peer_id
        || public_key != event.issuer_public_key
    {
        return Err("chave pública de entrada não corresponde ao emissor".into());
    }
    let x25519_public_key = BASE64
        .decode(
            event
                .payload
                .get("x25519_public_key")
                .and_then(serde_json::Value::as_str)
                .ok_or("chave de acordo de membro ausente")?,
        )
        .map_err(|error| format!("chave de acordo de membro inválida: {error}"))?;
    if x25519_public_key.len() != X25519_KEY_BYTES {
        return Err("chave de acordo de membro inválida".into());
    }
    let nonce = BASE64
        .decode(
            event
                .payload
                .get("invite_nonce")
                .and_then(serde_json::Value::as_str)
                .ok_or("prova de convite ausente")?,
        )
        .map_err(|error| format!("nonce da prova de convite inválido: {error}"))?;
    let ciphertext = BASE64
        .decode(
            event
                .payload
                .get("invite_ciphertext")
                .and_then(serde_json::Value::as_str)
                .ok_or("cifra da prova de convite ausente")?,
        )
        .map_err(|error| format!("cifra da prova de convite inválida: {error}"))?;
    let group = state
        .database
        .get_group(&event.group_id)?
        .ok_or("grupo da entrada não encontrado")?;
    if let Some(existing_member) = state.database.get_member(&event.group_id, target_peer_id)? {
        if existing_member.status == "banned" {
            return Err("membro banido não pode voltar por convite".into());
        }
        if existing_member.status == "active" {
            return Err("membro já está ativo neste grupo".into());
        }
    }
    let group_key = load_group_key_epoch(&event.group_id, event.key_epoch)?;
    let invite_text = decrypt_message(&group_key, &nonce, &ciphertext, &join_proof_aad(event)?)?;
    let invite = crypto::decode_invite(&invite_text)?;
    let invite_key = validate_invite(&invite, now_seconds())?;
    if invite.group_id != event.group_id
        || invite.owner_peer_id != group.owner_peer_id
        || invite.key_epoch != event.key_epoch
        || invite_key != group_key
    {
        return Err("prova de convite não pertence à época atual do grupo".into());
    }
    let owner = state
        .database
        .get_member(&event.group_id, &group.owner_peer_id)?
        .ok_or("identidade do Owner não sincronizada")?;
    let invite_owner_public_key = BASE64
        .decode(&invite.owner_public_key)
        .map_err(|error| format!("chave do Owner no convite inválida: {error}"))?;
    if invite_owner_public_key != owner.public_key {
        return Err("convite foi emitido para outro Owner".into());
    }
    if let Some(owner_x25519) = invite.owner_x25519_public_key.as_deref() {
        let owner_x25519 = BASE64
            .decode(owner_x25519)
            .map_err(|error| format!("chave de acordo do Owner inválida: {error}"))?;
        if owner_x25519.len() != X25519_KEY_BYTES
            || (owner.x25519_public_key.len() == X25519_KEY_BYTES
                && owner_x25519 != owner.x25519_public_key)
        {
            return Err("convite contém chave de acordo de outro Owner".into());
        }
    }
    if matches!(invite.version, 2 | 4 | 6 | 8) {
        let issuer_peer_id = invite
            .issuer_peer_id
            .as_deref()
            .ok_or("emissor do convite ausente")?;
        let issuer_public_key = BASE64
            .decode(
                invite
                    .issuer_public_key
                    .as_deref()
                    .ok_or("chave do emissor ausente")?,
            )
            .map_err(|error| format!("chave do emissor inválida: {error}"))?;
        let issuer = state
            .database
            .get_member(&event.group_id, issuer_peer_id)?
            .ok_or("emissor do convite não é membro conhecido")?;
        if issuer.status != "active"
            || issuer.public_key != issuer_public_key
            || !has_permission(&issuer.role, &Permission::ManageInvites)
        {
            return Err("emissor do convite não possui permissão vigente".into());
        }
    } else if !matches!(invite.version, 1 | 3 | 5 | 7) {
        return Err("tipo de convite de entrada inválido".into());
    }
    Ok(())
}

fn control_record(event: &ControlEvent) -> AuditEventRecord {
    AuditEventRecord {
        event_id: event.event_id.clone(),
        group_id: event.group_id.clone(),
        key_epoch: event.key_epoch,
        issuer_peer_id: event.issuer_peer_id.clone(),
        kind: event.kind.clone(),
        target_peer_id: event.target_peer_id.clone(),
        payload: event.payload.clone(),
        logical_timestamp: event.logical_timestamp,
        signature: event.signature.clone(),
    }
}

struct KeySharePayload {
    epoch: i64,
    sender_public_key: Vec<u8>,
    recipient_public_key: Vec<u8>,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn decode_key_share_payload(event: &ControlEvent) -> Result<KeySharePayload, String> {
    let decode = |name: &str| -> Result<Vec<u8>, String> {
        BASE64
            .decode(
                event
                    .payload
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("{name} ausente"))?,
            )
            .map_err(|error| format!("{name} inválido: {error}"))
    };
    let epoch = event
        .payload
        .get("epoch")
        .and_then(serde_json::Value::as_i64)
        .ok_or("época de chave ausente")?;
    let sender_public_key = decode("sender_public_key")?;
    let recipient_public_key = decode("recipient_public_key")?;
    let nonce = decode("nonce")?;
    let ciphertext = decode("ciphertext")?;
    if event.target_peer_id.is_none()
        || event.key_epoch != epoch
        || sender_public_key.len() != X25519_KEY_BYTES
        || recipient_public_key.len() != X25519_KEY_BYTES
        || nonce.is_empty()
        || ciphertext.is_empty()
    {
        return Err("compartilhamento de chave inválido".into());
    }
    Ok(KeySharePayload {
        epoch,
        sender_public_key,
        recipient_public_key,
        nonce,
        ciphertext,
    })
}

fn key_share_is_ready(
    state: &NodeState,
    event: &ControlEvent,
    payload: &KeySharePayload,
) -> Result<bool, String> {
    let target_peer_id = event
        .target_peer_id
        .as_deref()
        .ok_or("destinatário da chave ausente")?;
    let target = state
        .database
        .get_member(&event.group_id, target_peer_id)?
        .ok_or("destinatário da chave ainda não sincronizado")?;
    if target.x25519_public_key != payload.recipient_public_key {
        return Err("chave de acordo do destinatário não corresponde ao evento".into());
    }
    let issuer = state
        .database
        .get_member(&event.group_id, &event.issuer_peer_id)?
        .ok_or("emissor da chave ainda não sincronizado")?;
    if issuer.x25519_public_key != payload.sender_public_key {
        return Err("chave de acordo do emissor não corresponde ao evento".into());
    }
    if target.status != "active" {
        return Ok(false);
    }
    let current_epoch = current_group_epoch(&state.database, &event.group_id)?;
    if payload.epoch < current_epoch || payload.epoch > current_epoch + 1 {
        return Err("época de chave fora da sequência esperada".into());
    }
    let rotation_exists = state.database.has_audit_event_before(
        &event.group_id,
        CONTROL_KEY_ROTATED,
        payload.epoch,
        None,
        event.logical_timestamp,
    )?;
    let unban_exists = state.database.has_audit_event_before(
        &event.group_id,
        CONTROL_MEMBER_UNBANNED,
        payload.epoch,
        Some(target_peer_id),
        event.logical_timestamp,
    )?;
    Ok(rotation_exists || unban_exists)
}

fn member_event_is_stale(state: &NodeState, event: &ControlEvent) -> Result<bool, String> {
    if !matches!(
        event.kind.as_str(),
        CONTROL_MEMBER_JOINED
            | CONTROL_ROLE_CHANGED
            | CONTROL_MEMBER_KICKED
            | CONTROL_MEMBER_BANNED
            | CONTROL_MEMBER_UNBANNED
            | CONTROL_MEMBER_TIMEOUT
            | CONTROL_MEMBER_LEFT
    ) {
        return Ok(false);
    }
    let Some(target_peer_id) = event.target_peer_id.as_deref() else {
        return Ok(false);
    };
    let Some((timestamp, event_id)) = state
        .database
        .latest_member_control_cursor(&event.group_id, target_peer_id)?
    else {
        return Ok(false);
    };
    Ok(timestamp > event.logical_timestamp
        || (timestamp == event.logical_timestamp && event_id > event.event_id))
}

fn moderate_member(
    app: &AppHandle,
    state: &NodeState,
    group_id: &str,
    peer_id: &str,
    kind: &str,
) -> Result<(), String> {
    let actor = local_member(state, group_id)?;
    if !has_permission(&actor.role, &Permission::ManageMembers)
        && !has_permission(&actor.role, &Permission::ModerateMessages)
    {
        return Err("membro sem permissão para gerenciar membros".into());
    }
    let target = state
        .database
        .get_member(group_id, peer_id)?
        .ok_or("membro não encontrado")?;
    if target.status != "active" {
        return Err("este membro não está ativo para esta ação".into());
    }
    if !can_manage_target(&actor.role, &target.role) {
        return Err("não é possível moderar este cargo".into());
    }
    if matches!(kind, CONTROL_MEMBER_KICKED | CONTROL_MEMBER_BANNED) {
        ensure_key_rotation_recipients(state, group_id, Some(peer_id))?;
    }
    let event = new_control_event(
        state,
        group_id,
        kind,
        Some(peer_id.to_string()),
        serde_json::json!({}),
    )?;
    submit_control_event(app, state, event)?;
    if matches!(kind, CONTROL_MEMBER_KICKED | CONTROL_MEMBER_BANNED) {
        rotate_group_key(app, state, group_id)?;
    }
    Ok(())
}

fn new_control_event(
    state: &NodeState,
    group_id: &str,
    kind: &str,
    target_peer_id: Option<String>,
    payload: serde_json::Value,
) -> Result<ControlEvent, String> {
    let keypair = ensure_keypair(state)?;
    let mut event = ControlEvent {
        event_id: Uuid::new_v4().to_string(),
        group_id: group_id.into(),
        key_epoch: current_group_epoch(&state.database, group_id)?,
        issuer_peer_id: keypair.public().to_peer_id().to_string(),
        issuer_public_key: keypair.public().encode_protobuf(),
        kind: kind.into(),
        target_peer_id,
        payload,
        logical_timestamp: now_millis(),
        signature: Vec::new(),
    };
    sign_control_event(&mut event, &keypair)?;
    Ok(event)
}

fn submit_control_event(
    app: &AppHandle,
    state: &NodeState,
    event: ControlEvent,
) -> Result<(), String> {
    verify_control_event(&event)?;
    let sender = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
        .ok_or("node ainda não iniciou")?;
    let issuer = state
        .database
        .get_member(&event.group_id, &event.issuer_peer_id)?
        .ok_or("autoridade administrativa desconhecida")?;
    if issuer.status != "active" {
        return Err("membro administrativo não está ativo".into());
    }
    if event.kind == CONTROL_KEY_SHARE {
        let payload = decode_key_share_payload(&event)?;
        if !key_share_is_ready(state, &event, &payload)? {
            return Err(
                "compartilhamento de chave ainda não está autorizado pela época atual".into(),
            );
        }
    }
    if member_event_is_stale(state, &event)? {
        let _ = state.database.insert_audit_event(&control_record(&event))?;
        return Ok(());
    }
    if !state.database.insert_audit_event(&control_record(&event))? {
        return Err("evento administrativo duplicado".into());
    }
    state.database.enqueue_outbox(
        &event.event_id,
        "control",
        &serde_json::to_string(&event)
            .map_err(|error| format!("não foi possível preparar evento administrativo: {error}"))?,
        event.logical_timestamp,
    )?;
    apply_control_event(state, &event)?;
    if matches!(
        event.kind.as_str(),
        CONTROL_MEMBER_KICKED | CONTROL_MEMBER_BANNED | CONTROL_MEMBER_LEFT | CONTROL_GROUP_DELETED
    ) {
        for call_state in remove_call_participants(
            state,
            &event.group_id,
            if event.kind == CONTROL_GROUP_DELETED {
                None
            } else {
                event.target_peer_id.as_deref()
            },
        )? {
            emit_call_state(app, state, &call_state)?;
        }
    }
    flush_pending_call_signals(app, state, &event.group_id);
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "group-control".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::to_value(&event).map_err(|error| error.to_string())?),
        },
    );
    let _ = sender.send(NodeCommand::PublishControl {
        event: Box::new(event),
    });
    Ok(())
}

fn rotate_group_key(app: &AppHandle, state: &NodeState, group_id: &str) -> Result<(), String> {
    let actor = local_member(state, group_id)?;
    if !has_permission(&actor.role, &Permission::ManageMembers)
        && !has_permission(&actor.role, &Permission::ModerateMessages)
    {
        return Err("membro sem permissão para rotacionar chave".into());
    }
    let recipients = ensure_key_rotation_recipients(state, group_id, None)?;
    let current_epoch = current_group_epoch(&state.database, group_id)?;
    let next_epoch = current_epoch + 1;
    let new_key = new_group_key()?;
    let (_, sender_public) = ensure_agreement_keypair()?;
    state.database.insert_key_epoch(
        group_id,
        next_epoch,
        &actor.peer_id,
        now_millis(),
        &hash_key(&new_key),
    )?;
    store_group_key_epoch(group_id, next_epoch, &new_key)?;
    let mut rotation = new_control_event(
        state,
        group_id,
        CONTROL_KEY_ROTATED,
        None,
        serde_json::json!({ "epoch": next_epoch }),
    )?;
    rotation.key_epoch = next_epoch;
    sign_control_event(&mut rotation, &ensure_keypair(state)?)?;
    submit_control_event(app, state, rotation)?;
    let aad = |peer_id: &str| format!("teamscord-key-share-v1:{group_id}:{next_epoch}:{peer_id}");
    for member in recipients {
        let (nonce, ciphertext) = encrypt_for_recipient(
            &ensure_agreement_keypair()?.0,
            &member.x25519_public_key,
            &new_key,
            aad(&member.peer_id).as_bytes(),
        )?;
        let event = new_control_event(
            state,
            group_id,
            CONTROL_KEY_SHARE,
            Some(member.peer_id.clone()),
            serde_json::json!({
                "epoch": next_epoch,
                "sender_public_key": BASE64.encode(sender_public),
                "nonce": BASE64.encode(nonce),
                "ciphertext": BASE64.encode(ciphertext),
                "recipient_public_key": BASE64.encode(&member.x25519_public_key),
            }),
        )?;
        submit_control_event(app, state, event)?;
    }
    Ok(())
}

fn ensure_key_rotation_recipients(
    state: &NodeState,
    group_id: &str,
    excluded_peer_id: Option<&str>,
) -> Result<Vec<MemberRecord>, String> {
    let recipients = state
        .database
        .list_members(group_id)?
        .into_iter()
        .filter(|member| {
            member.status == "active" && Some(member.peer_id.as_str()) != excluded_peer_id
        })
        .collect::<Vec<_>>();
    if recipients
        .iter()
        .any(|member| member.x25519_public_key.len() != X25519_KEY_BYTES)
    {
        return Err("um membro ativo ainda não possui chave de acordo; peça um novo convite antes de rotacionar".into());
    }
    Ok(recipients)
}

fn share_current_group_key(
    app: &AppHandle,
    state: &NodeState,
    group_id: &str,
    recipient_peer_id: &str,
) -> Result<(), String> {
    let actor = local_member(state, group_id)?;
    if !has_permission(&actor.role, &Permission::ManageMembers) {
        return Err("membro sem permissão para compartilhar chave de grupo".into());
    }
    let recipient = state
        .database
        .get_member(group_id, recipient_peer_id)?
        .ok_or("membro destinatário não encontrado")?;
    if recipient.status != "active" || recipient.x25519_public_key.len() != X25519_KEY_BYTES {
        return Err("membro destinatário não está pronto para receber a chave".into());
    }
    let group = state
        .database
        .get_group(group_id)?
        .ok_or("grupo não encontrado")?;
    let epoch = group.current_key_epoch;
    let group_key = load_group_key_epoch(group_id, epoch)?;
    let (sender_secret, sender_public) = ensure_agreement_keypair()?;
    let aad = format!("teamscord-key-share-v1:{group_id}:{epoch}:{recipient_peer_id}");
    let (nonce, ciphertext) = encrypt_for_recipient(
        &sender_secret,
        &recipient.x25519_public_key,
        &group_key,
        aad.as_bytes(),
    )?;
    let event = new_control_event(
        state,
        group_id,
        CONTROL_KEY_SHARE,
        Some(recipient_peer_id.into()),
        serde_json::json!({
            "epoch": epoch,
            "sender_public_key": BASE64.encode(sender_public),
            "recipient_public_key": BASE64.encode(&recipient.x25519_public_key),
            "nonce": BASE64.encode(nonce),
            "ciphertext": BASE64.encode(ciphertext),
        }),
    )?;
    submit_control_event(app, state, event)
}

fn new_call_signal(
    state: &NodeState,
    group_id: &str,
    channel_id: &str,
    call_id: &str,
    to_peer_id: Option<String>,
    kind: &str,
    payload: serde_json::Value,
) -> Result<CallSignal, String> {
    let keypair = ensure_keypair(state)?;
    let epoch = current_group_epoch(&state.database, group_id)?;
    let mut signal = CallSignal {
        event_id: Uuid::new_v4().to_string(),
        group_id: group_id.into(),
        channel_id: channel_id.into(),
        call_id: call_id.into(),
        from_peer_id: keypair.public().to_peer_id().to_string(),
        from_public_key: keypair.public().encode_protobuf(),
        key_epoch: epoch,
        created_at: now_millis(),
        nonce: Vec::new(),
        ciphertext: Vec::new(),
        signature: Vec::new(),
    };
    let body = serde_json::to_string(&CallSignalBody {
        to_peer_id,
        kind: kind.into(),
        payload,
    })
    .map_err(|error| format!("sinal de call inválido: {error}"))?;
    let group_key = load_group_key_epoch(group_id, epoch)?;
    let (nonce, ciphertext) =
        encrypt_message(&group_key, &body, &protocol::call_signal_aad(&signal)?)?;
    signal.nonce = nonce;
    signal.ciphertext = ciphertext;
    signal.signature = keypair
        .sign(&protocol::call_signal_signing_bytes(&signal)?)
        .map_err(|error| format!("não foi possível assinar sinal de call: {error}"))?;
    Ok(signal)
}

fn publish_call_signal(
    _app: &AppHandle,
    state: &NodeState,
    signal: CallSignal,
) -> Result<(), String> {
    let sender = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
        .ok_or("node ainda não iniciou")?;
    sender
        .send(NodeCommand::PublishCall {
            signal: Box::new(signal),
        })
        .map_err(|_| "node não está aceitando sinal de call".to_string())
}

fn emit_call_state(
    app: &AppHandle,
    _state: &NodeState,
    call_state: &CallState,
) -> Result<(), String> {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "call-state".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::to_value(call_state).map_err(|error| error.to_string())?),
        },
    );
    Ok(())
}

fn remove_call_participants(
    state: &NodeState,
    group_id: &str,
    peer_id: Option<&str>,
) -> Result<Vec<CallState>, String> {
    let mut states = state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?;
    let mut changed = Vec::new();
    for call_state in states.values_mut().filter(|call| call.group_id == group_id) {
        let before = call_state.participants.len();
        if let Some(peer_id) = peer_id {
            call_state
                .participants
                .retain(|participant| participant.peer_id != peer_id);
        } else {
            call_state.participants.clear();
        }
        if call_state.participants.len() != before {
            changed.push(call_state.clone());
        }
    }
    Ok(changed)
}

fn update_call_state(
    state: &NodeState,
    signal: &CallSignal,
    body: &CallSignalBody,
) -> Result<Option<CallState>, String> {
    let key = format!(
        "{}:{}:{}",
        signal.group_id, signal.channel_id, signal.call_id
    );
    let mut states = state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?;
    if !states.contains_key(&key) {
        if body.kind != "join" {
            return Ok(None);
        }
        states.insert(
            key.clone(),
            CallState {
                group_id: signal.group_id.clone(),
                channel_id: signal.channel_id.clone(),
                call_id: signal.call_id.clone(),
                participants: Vec::new(),
            },
        );
    }
    let Some(call_state) = states.get_mut(&key) else {
        return Ok(None);
    };
    let member = state
        .database
        .get_member(&signal.group_id, &signal.from_peer_id)?;
    match body.kind.as_str() {
        "join" => {
            if call_state.participants.len() >= MAX_CALL_PARTICIPANTS
                && !call_state
                    .participants
                    .iter()
                    .any(|participant| participant.peer_id == signal.from_peer_id)
            {
                return Ok(Some(call_state.clone()));
            }
            let display_name = body
                .payload
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .or_else(|| member.as_ref().map(|value| value.display_name.as_str()))
                .unwrap_or("Membro")
                .to_string();
            let role = member
                .as_ref()
                .map(|value| value.role.clone())
                .unwrap_or(Role::Member);
            if let Some(existing) = call_state
                .participants
                .iter_mut()
                .find(|existing| existing.peer_id == signal.from_peer_id)
            {
                existing.display_name = display_name;
                existing.role = role;
                existing.connection_state = "connecting".into();
            } else {
                call_state.participants.push(CallParticipant {
                    peer_id: signal.from_peer_id.clone(),
                    display_name,
                    role,
                    muted: false,
                    sharing_screen: false,
                    connection_state: "connecting".into(),
                });
            }
        }
        "leave" => {
            call_state
                .participants
                .retain(|participant| participant.peer_id != signal.from_peer_id);
            let snapshot = call_state.clone();
            if snapshot.participants.is_empty() {
                states.remove(&key);
            }
            return Ok(Some(snapshot));
        }
        "mute" => {
            if let Some(participant) = call_state
                .participants
                .iter_mut()
                .find(|participant| participant.peer_id == signal.from_peer_id)
            {
                participant.muted = body
                    .payload
                    .get("muted")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
            }
        }
        "screen-start" => {
            if call_state.participants.iter().any(|participant| {
                participant.peer_id != signal.from_peer_id && participant.sharing_screen
            }) {
                return Err("já existe outra tela compartilhada nesta call".into());
            }
            if let Some(participant) = call_state
                .participants
                .iter_mut()
                .find(|participant| participant.peer_id == signal.from_peer_id)
            {
                participant.sharing_screen = true;
            }
        }
        "screen-stop" => {
            if let Some(participant) = call_state
                .participants
                .iter_mut()
                .find(|participant| participant.peer_id == signal.from_peer_id)
            {
                participant.sharing_screen = false;
            }
        }
        "peer-state" => {
            if let Some(participant) = call_state
                .participants
                .iter_mut()
                .find(|participant| participant.peer_id == signal.from_peer_id)
            {
                participant.connection_state = normalized_connection_state(
                    body.payload
                        .get("connection_state")
                        .and_then(serde_json::Value::as_str),
                )
                .into();
            }
        }
        _ => {}
    }
    Ok(Some(call_state.clone()))
}

fn normalized_connection_state(value: Option<&str>) -> &'static str {
    match value {
        Some("new") => "new",
        Some("connecting") => "connecting",
        Some("connected") => "connected",
        Some("disconnected") => "disconnected",
        Some("failed") => "failed",
        Some("closed") => "closed",
        _ => "connecting",
    }
}

async fn run_node(
    app: AppHandle,
    state: NodeState,
    keypair: Keypair,
    mut command_rx: mpsc::UnboundedReceiver<NodeCommand>,
) -> Result<(), String> {
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(std::time::Duration::from_secs(10))
        .validation_mode(gossipsub::ValidationMode::Strict)
        .build()
        .map_err(|error| format!("configuração GossipSub inválida: {error}"))?;
    let gossipsub = gossipsub::Behaviour::new(
        MessageAuthenticity::Signed(keypair.clone()),
        gossipsub_config,
    )
    .map_err(|error| format!("GossipSub indisponível: {error}"))?;
    let identify = identify::Behaviour::new(identify::Config::new(
        "/teamscord/1.0.0".into(),
        keypair.public(),
    ));
    let ping = ping::Behaviour::new(ping::Config::new());
    let sync = request_response::json::Behaviour::new(
        [(StreamProtocol::new(SYNC_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default()
            .with_request_timeout(std::time::Duration::from_secs(15)),
    );
    let call_signal = request_response::json::Behaviour::new(
        [(
            StreamProtocol::new(CALL_SIGNAL_PROTOCOL),
            ProtocolSupport::Full,
        )],
        request_response::Config::default()
            .with_request_timeout(std::time::Duration::from_secs(10)),
    );
    let presence = request_response::json::Behaviour::new(
        [(
            StreamProtocol::new(PRESENCE_PROTOCOL),
            ProtocolSupport::Full,
        )],
        request_response::Config::default()
            .with_request_timeout(std::time::Duration::from_secs(10)),
    );
    let direct = request_response::json::Behaviour::new(
        [(StreamProtocol::new(DIRECT_PROTOCOL), ProtocolSupport::Full)],
        request_response::Config::default()
            .with_request_timeout(std::time::Duration::from_secs(15)),
    );
    let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), keypair.public().to_peer_id())
        .map_err(|error| format!("mDNS indisponível: {error}"))?;
    let mut swarm: Swarm<Behaviour> = SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(|error| format!("transporte TCP indisponível: {error}"))?
        .with_quic()
        .with_relay_client(noise::Config::new, yamux::Config::default)
        .map_err(|error| format!("relay client indisponível: {error}"))?
        .with_behaviour(|_, relay| Behaviour {
            gossipsub,
            identify,
            mdns,
            ping,
            relay,
            sync,
            call_signal,
            presence,
            direct,
        })
        .expect("comportamento P2P deveria ser construído sem erro")
        .build();
    listen_on(&mut swarm, "/ip4/0.0.0.0/tcp/0")?;
    listen_on(&mut swarm, "/ip4/0.0.0.0/udp/0/quic-v1")?;
    let mut subscribed = HashSet::new();
    let mut known_peer_addresses: HashMap<PeerId, Vec<Multiaddr>> = HashMap::new();
    for record in state.database.list_peer_addresses()? {
        let Ok(peer_id) = record.peer_id.parse::<PeerId>() else {
            continue;
        };
        let Ok(address) = record.address.parse::<Multiaddr>() else {
            continue;
        };
        remember_peer_address(&mut known_peer_addresses, &address);
        known_peer_addresses
            .entry(peer_id)
            .or_insert_with(|| vec![address]);
    }
    for group in state.database.list_groups()? {
        subscribe_group(&mut swarm, &state.database, &group.id, &mut subscribed)?;
    }
    for address in state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")?
        .relay_addresses
        .clone()
    {
        if let Ok(address) = address.parse::<Multiaddr>() {
            if let Err(error) = configure_relay(&mut swarm, address) {
                emit_error(&app, error);
            }
        }
    }
    for address in state
        .network_config
        .lock()
        .map_err(|_| "configuração de rede bloqueada")?
        .bootstrap_addresses
        .clone()
    {
        if let Ok(address) = address.parse::<Multiaddr>() {
            remember_peer_address(&mut known_peer_addresses, &address);
            if let Err(error) = swarm.dial(address) {
                emit_error(
                    &app,
                    format!("não foi possível conectar ao bootstrap: {error}"),
                );
            }
        }
    }
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "ready".into(),
            message: None,
            snapshot: None,
            error: None,
            data: None,
        },
    );
    let mut presence_heartbeat = tokio::time::interval(std::time::Duration::from_secs(15));
    presence_heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outbox_retry = tokio::time::interval(std::time::Duration::from_secs(5));
    outbox_retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = presence_heartbeat.tick() => {
                if let Ok(announcement) = local_presence_announcement(&state) {
                    for peer_id in swarm.connected_peers().copied().collect::<Vec<_>>() {
                        swarm.behaviour_mut().presence.send_request(&peer_id, announcement.clone());
                    }
                }
            }
            _ = outbox_retry.tick() => {
                if let Err(error) = flush_outbox(&mut swarm, &state, &app) {
                    emit_error(&app, error);
                }
            }
            Some(command) = command_rx.recv() => match command {
                NodeCommand::Dial { address } => {
                    remember_peer_address(&mut known_peer_addresses, &address);
                    if let Err(error) = swarm.dial(address) { emit_error(&app, format!("não foi possível conectar: {error}")); }
                }
                NodeCommand::ConfigureRelay { address } => {
                    if let Err(error) = configure_relay(&mut swarm, address) { emit_error(&app, error); }
                }
                NodeCommand::SubscribeGroup { group_id } => {
                    if let Err(error) = subscribe_group(&mut swarm, &state.database, &group_id, &mut subscribed) { emit_error(&app, error); }
                }
                NodeCommand::RequestSync => {
                    if let Err(error) = request_sync_all(&mut swarm, &state.database) { emit_error(&app, error); }
                    if let Err(error) = flush_outbox(&mut swarm, &state, &app) { emit_error(&app, error); }
                }
                NodeCommand::BroadcastPresence => {
                    if let Ok(announcement) = local_presence_announcement(&state) {
                        for peer_id in swarm.connected_peers().copied().collect::<Vec<_>>() {
                            swarm.behaviour_mut().presence.send_request(&peer_id, announcement.clone());
                        }
                    }
                }
                NodeCommand::Publish { envelope } => {
                    let envelope = *envelope;
                    let topic = IdentTopic::new(topic_for(&envelope.channel_id));
                    match serde_json::to_vec(&envelope) {
                        Ok(payload) => match swarm.behaviour_mut().gossipsub.publish(topic, payload) {
                            Ok(_) => { let _ = state.database.remove_outbox(&envelope.event_id); }
                            Err(error) => {
                                let _ = state.database.mark_outbox_attempt(&envelope.event_id, now_millis());
                                if is_waiting_publish_error(&error) { emit_sync_waiting(&app); }
                                else { emit_error(&app, format!("não foi possível publicar: {error}")); }
                            }
                        },
                        Err(error) => emit_error(&app, format!("mensagem inválida: {error}")),
                    }
                }
                NodeCommand::PublishControl { event } => {
                    let event = *event;
                    let topic = IdentTopic::new(topic_for_control(&event.group_id));
                    match serde_json::to_vec(&event) {
                        Ok(payload) => match swarm.behaviour_mut().gossipsub.publish(topic, payload) {
                            Ok(_) => { let _ = state.database.remove_outbox(&event.event_id); }
                            Err(error) => {
                                let _ = state.database.mark_outbox_attempt(&event.event_id, now_millis());
                                if is_waiting_publish_error(&error) { emit_sync_waiting(&app); }
                                else { emit_error(&app, format!("não foi possível publicar evento administrativo: {error}")); }
                            }
                        },
                        Err(error) => emit_error(&app, format!("evento administrativo inválido: {error}")),
                    }
                }
                NodeCommand::PublishCall { signal } => {
                    let signal = *signal;
                    let peers = swarm.connected_peers().copied().collect::<Vec<_>>();
                    for peer_id in peers {
                        swarm
                            .behaviour_mut()
                            .call_signal
                            .send_request(&peer_id, signal.clone());
                    }
                }
                NodeCommand::PublishDirect { envelope } => {
                    let envelope = *envelope;
                    if let Ok(peer_id) = envelope.to_peer_id.parse::<PeerId>() {
                        if swarm.connected_peers().any(|connected| connected == &peer_id) {
                            swarm.behaviour_mut().direct.send_request(&peer_id, envelope.clone());
                            let _ = state.database.remove_outbox(&envelope.event_id);
                        } else {
                            let _ = state.database.mark_outbox_attempt(&envelope.event_id, now_millis());
                        }
                    }
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    if let Some(address) = announced_address(address, swarm.local_peer_id()) {
                        if let Ok(mut snapshot) = state.snapshot.lock() {
                            if !snapshot.listen_addresses.contains(&address.to_string()) { snapshot.listen_addresses.push(address.to_string()); }
                            emit_snapshot(&app, &snapshot);
                        }
                    }
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    if let Ok(mut snapshot) = state.snapshot.lock() {
                        snapshot.connected_peers += 1;
                        if snapshot.relay_addresses.iter().any(|address| relay_peer_id(address).as_ref() == Some(&peer_id)) {
                            snapshot.relay_connected = true;
                            emit_network_event(&app, "relay-state", serde_json::json!({ "peer_id": peer_id.to_string(), "state": "connected" }));
                        }
                        emit_snapshot(&app, &snapshot);
                    }
                    let _ = state.database.remember_peer(&peer_id.to_string(), now_millis());
                    let _ = app.emit(
                        EVENT_NAME,
                        NodeEvent {
                            kind: "peer-updated".into(),
                            message: None,
                            snapshot: None,
                            error: None,
                            data: Some(serde_json::json!({ "peer_id": peer_id.to_string(), "state": "connected" })),
                        },
                    );
                    emit_peer_presence(&app, &peer_id, "connecting", "connection");
                    emit_network_event(
                        &app,
                        "connection-diagnostic",
                        serde_json::json!({
                            "peer_id": peer_id.to_string(),
                            "state": "connected",
                            "detail": "transporte libp2p conectado",
                            "checked_at": now_millis(),
                        }),
                    );
                    let _ = request_sync(&mut swarm, &state.database, &peer_id);
                    if let Err(error) = flush_outbox(&mut swarm, &state, &app) { emit_error(&app, error); }
                    if let Ok(announcement) = local_presence_announcement(&state) {
                        swarm.behaviour_mut().presence.send_request(&peer_id, announcement);
                    }
                }
                SwarmEvent::ConnectionClosed { peer_id, num_established, .. } => {
                    if let Ok(mut snapshot) = state.snapshot.lock() {
                        snapshot.connected_peers = snapshot.connected_peers.saturating_sub(1);
                        if snapshot.relay_addresses.iter().any(|address| relay_peer_id(address).as_ref() == Some(&peer_id)) && num_established == 0 {
                            snapshot.relay_connected = false;
                            emit_network_event(&app, "relay-state", serde_json::json!({ "peer_id": peer_id.to_string(), "state": "offline" }));
                        }
                        emit_snapshot(&app, &snapshot);
                    }
                    emit_peer_presence(&app, &peer_id, "offline", "connection");
                    emit_network_event(
                        &app,
                        "connection-diagnostic",
                        serde_json::json!({
                            "peer_id": peer_id.to_string(),
                            "state": "reconnecting",
                            "detail": "conexão encerrada; endereços conhecidos serão tentados novamente",
                            "checked_at": now_millis(),
                        }),
                    );
                    let _ = app.emit(
                        EVENT_NAME,
                        NodeEvent {
                            kind: "peer-updated".into(),
                            message: None,
                            snapshot: None,
                            error: None,
                            data: Some(serde_json::json!({ "peer_id": peer_id.to_string(), "state": "disconnected" })),
                        },
                    );
                    if num_established == 0 {
                        if let Some(addresses) = known_peer_addresses.get(&peer_id).cloned() {
                            for address in addresses {
                                let dial_address = peer_dial_address(address, &peer_id);
                                if let Err(error) = swarm.dial(dial_address) {
                                    emit_error(&app, format!("não foi possível reconectar ao peer: {error}"));
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                    let addresses = info
                        .listen_addrs
                        .into_iter()
                        .filter(|address| !address.iter().any(|protocol| matches!(protocol, Protocol::P2pCircuit)))
                        .filter_map(|address| announced_address(address, &peer_id))
                        .collect::<Vec<_>>();
                    if !addresses.is_empty() {
                        for address in &addresses {
                            remember_peer_address(&mut known_peer_addresses, address);
                            let _ = state.database.remember_peer_address(&peer_id.to_string(), &address.to_string(), "identify", now_millis());
                        }
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(peers))) => {
                    for (peer_id, address) in peers {
                        if peer_id == *swarm.local_peer_id() {
                            continue;
                        }
                        let address = peer_dial_address(address, &peer_id);
                        remember_peer_address(&mut known_peer_addresses, &address);
                        let _ = state.database.remember_peer_address(&peer_id.to_string(), &address.to_string(), "mdns", now_millis());
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        emit_peer_presence(&app, &peer_id, "connecting", "mdns");
                        if let Err(error) = swarm.dial(address) {
                            emit_error(&app, format!("não foi possível conectar ao peer descoberto na LAN: {error}"));
                        }
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic })) => {
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    if topic.as_str().starts_with("teamscord/") {
                        if let Err(error) = flush_outbox(&mut swarm, &state, &app) {
                            emit_error(&app, error);
                        }
                        emit_network_event(
                            &app,
                            "gossip-subscription",
                            serde_json::json!({
                                "peer_id": peer_id.to_string(),
                                "topic": topic.as_str(),
                                "state": "ready",
                            }),
                        );
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                    let topic = message.topic.as_str();
                    if topic.starts_with("teamscord/channel/") {
                        if let Ok(envelope) = serde_json::from_slice::<MessageEnvelope>(&message.data) { process_incoming(&app, &state, envelope); }
                    } else if topic.starts_with("teamscord/control/") {
                        if let Ok(event) = serde_json::from_slice::<ControlEvent>(&message.data) { process_control_event(&app, &state, event); }
                    } else if topic.starts_with("teamscord/call/") {
                        if let Ok(signal) = serde_json::from_slice::<CallSignal>(&message.data) { process_call_signal(&app, &state, signal); }
                    }
                }
                SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::Message { peer, message, .. })) => match message {
                    request_response::Message::Request { request, channel, .. } => {
                        let response = build_sync_response(&state.database, &request, &peer).unwrap_or(SyncResponse { messages: Vec::new(), controls: Vec::new(), next: None });
                        let _ = swarm.behaviour_mut().sync.send_response(channel, response);
                    }
                    request_response::Message::Response { response, .. } => {
                        for event in &response.controls { process_control_event(&app, &state, event.clone()); }
                        for envelope in &response.messages { process_incoming(&app, &state, envelope.clone()); }
                        match advance_sync_cursor(&state.database, &peer, &response) {
                            Ok(Some(next)) => {
                                if let Err(error) = request_sync_page(&mut swarm, &peer, next) {
                                    emit_error(&app, error);
                                }
                            }
                            Ok(None) => {}
                            Err(error) => emit_error(&app, error),
                        }
                        let _ = app.emit(
                            EVENT_NAME,
                            NodeEvent {
                                kind: "sync-state".into(),
                                message: None,
                                snapshot: None,
                                error: None,
                                data: Some(serde_json::json!({ "state": "synced" })),
                            },
                        );
                        if let Ok(mut snapshot) = state.snapshot.lock() {
                            snapshot.last_sync_at = Some(now_millis());
                            emit_snapshot(&app, &snapshot);
                        }
                        emit_network_event(
                            &app,
                            "sync-progress",
                            serde_json::json!({
                                "peer_id": peer.to_string(),
                                "state": "synced",
                                "messages": response.messages.len(),
                                "controls": response.controls.len(),
                            }),
                        );
                    }
                },
                SwarmEvent::Behaviour(BehaviourEvent::Sync(request_response::Event::OutboundFailure { error, .. })) => emit_error(&app, format!("sincronização falhou: {error}")),
                SwarmEvent::Behaviour(BehaviourEvent::CallSignal(request_response::Event::Message { message, .. })) => match message {
                    request_response::Message::Request { request, channel, .. } => {
                        let response = call_signal_join_response(&state, &request).unwrap_or_else(|| request.clone());
                        process_call_signal(&app, &state, request.clone());
                        let _ = swarm
                            .behaviour_mut()
                            .call_signal
                            .send_response(channel, response);
                    }
                    request_response::Message::Response { response, .. } => {
                        process_call_signal(&app, &state, response);
                    }
                },
                SwarmEvent::Behaviour(BehaviourEvent::CallSignal(request_response::Event::OutboundFailure { error, .. })) => emit_media_error(&app, format!("sinal de call não entregue: {error}")),
                SwarmEvent::Behaviour(BehaviourEvent::Presence(request_response::Event::Message { peer, message, .. })) => match message {
                    request_response::Message::Request { request, channel, .. } => {
                        if let Err(error) = process_presence(&app, &state, request) {
                            emit_error(&app, error);
                        }
                        if let Ok(announcement) = local_presence_announcement(&state) {
                            let _ = swarm.behaviour_mut().presence.send_response(channel, announcement);
                        }
                        let _ = peer;
                    }
                    request_response::Message::Response { response, .. } => {
                        if let Err(error) = process_presence(&app, &state, response) {
                            emit_error(&app, error);
                        }
                    }
                },
                SwarmEvent::Behaviour(BehaviourEvent::Presence(request_response::Event::OutboundFailure { peer, error, .. })) => emit_presence_transport_issue(&app, &peer, error),
                SwarmEvent::Behaviour(BehaviourEvent::Direct(request_response::Event::Message { message, .. })) => match message {
                    request_response::Message::Request { request, channel, .. } => {
                        if let Err(error) = process_direct_envelope(&app, &state, request.clone()) {
                            emit_error(&app, error);
                        }
                        let _ = swarm.behaviour_mut().direct.send_response(channel, request);
                    }
                    request_response::Message::Response { response, .. } => {
                        if let Err(error) = process_direct_envelope(&app, &state, response) {
                            emit_error(&app, error);
                        }
                    }
                },
                SwarmEvent::Behaviour(BehaviourEvent::Direct(request_response::Event::OutboundFailure { error, .. })) => {
                    emit_network_event(&app, "friend-updated", serde_json::json!({ "state": "reconnecting", "detail": error.to_string() }));
                }
                _ => {}
            }
        }
    }
}

fn peer_dial_address(address: Multiaddr, peer_id: &PeerId) -> Multiaddr {
    if address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2p(_)))
    {
        address
    } else {
        address.with(Protocol::P2p(*peer_id))
    }
}

fn relay_peer_id(address: &str) -> Option<PeerId> {
    address
        .parse::<Multiaddr>()
        .ok()?
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::P2p(peer_id) => Some(peer_id),
            _ => None,
        })
}

fn emit_network_event(app: &AppHandle, kind: &str, data: serde_json::Value) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: kind.into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(data),
        },
    );
}

fn emit_presence_transport_issue(
    app: &AppHandle,
    peer_id: &PeerId,
    error: request_response::OutboundFailure,
) {
    let technical_details = error.to_string();
    emit_network_event(
        app,
        "peer-presence",
        serde_json::json!({
            "peer_id": peer_id.to_string(),
            "state": "reconnecting",
            "last_seen": now_millis(),
            "source": "presence",
            "retryable": true,
            "technical_details": technical_details,
        }),
    );
    emit_network_event(
        app,
        "connection-diagnostic",
        serde_json::json!({
            "peer_id": peer_id.to_string(),
            "state": "reconnecting",
            "detail": "a entrega de presença falhou; o heartbeat tentará novamente",
            "technical_details": technical_details,
            "retryable": true,
            "checked_at": now_millis(),
        }),
    );
}

fn contact_addresses(state: &NodeState) -> Vec<String> {
    let local_peer_id = ensure_keypair(state)
        .ok()
        .map(|keypair| keypair.public().to_peer_id());
    let mut addresses = HashSet::new();
    if let Ok(snapshot) = state.snapshot.lock() {
        addresses.extend(snapshot.listen_addresses.iter().cloned());
    }
    if let Ok(config) = state.network_config.lock() {
        for raw in &config.relay_addresses {
            let Ok(address) = raw.parse::<Multiaddr>() else {
                continue;
            };
            let Some(peer_id) = local_peer_id else {
                continue;
            };
            if address.iter().any(|protocol| match protocol {
                Protocol::Ip4(ip) => ip.is_unspecified(),
                Protocol::Ip6(ip) => ip.is_unspecified(),
                _ => false,
            }) {
                continue;
            }
            let address = if address
                .iter()
                .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
            {
                address
            } else {
                address.with(Protocol::P2pCircuit)
            }
            .with(Protocol::P2p(peer_id));
            addresses.insert(address.to_string());
        }
        for raw in &config.bootstrap_addresses {
            if let Ok(address) = raw.parse::<Multiaddr>() {
                if !address.iter().any(|protocol| match protocol {
                    Protocol::Ip4(ip) => ip.is_unspecified(),
                    Protocol::Ip6(ip) => ip.is_unspecified(),
                    _ => false,
                }) && address
                    .iter()
                    .any(|protocol| matches!(protocol, Protocol::P2p(_)))
                {
                    addresses.insert(address.to_string());
                }
            }
        }
    }
    addresses.into_iter().collect()
}

fn local_contact_card(state: &NodeState, display_name: &str) -> Result<ContactCard, String> {
    let keypair = ensure_keypair(state)?;
    let (secret, public) = ensure_agreement_keypair()?;
    let peer_id = keypair.public().to_peer_id().to_string();
    let safe_name = display_name.trim().chars().take(64).collect::<String>();
    let mut card = ContactCard {
        version: 1,
        peer_id,
        display_name: if safe_name.is_empty() {
            "Você".into()
        } else {
            safe_name
        },
        public_key: keypair.public().encode_protobuf(),
        x25519_public_key: public.to_vec(),
        addresses: contact_addresses(state),
        created_at: now_millis(),
        signature: Vec::new(),
    };
    let _ = secret;
    card.signature = keypair
        .sign(&contact_card_signing_bytes(&card)?)
        .map_err(|error| format!("não foi possível assinar cartão de contato: {error}"))?;
    Ok(card)
}

fn encode_contact_card(card: &ContactCard) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(card).map_err(|error| format!("cartão de contato inválido: {error}"))?;
    Ok(BASE64.encode(bytes))
}

fn decode_contact_card(value: &str) -> Result<ContactCard, String> {
    let trimmed = value.trim();
    let encoded = trimmed
        .strip_prefix("teamscord://contact/v1/")
        .unwrap_or(trimmed);
    let bytes = BASE64
        .decode(encoded)
        .map_err(|error| format!("código de contato inválido: {error}"))?;
    let card: ContactCard = serde_json::from_slice(&bytes)
        .map_err(|error| format!("cartão de contato inválido: {error}"))?;
    validate_contact_card(&card)?;
    Ok(card)
}

fn validate_contact_card(card: &ContactCard) -> Result<(), String> {
    if card.version != 1 || card.x25519_public_key.len() != X25519_KEY_BYTES {
        return Err("cartão de contato incompatível".into());
    }
    let public_key = libp2p::identity::PublicKey::try_decode_protobuf(&card.public_key)
        .map_err(|error| format!("chave pública de contato inválida: {error}"))?;
    if public_key.to_peer_id().to_string() != card.peer_id {
        return Err("cartão de contato não corresponde ao PeerId".into());
    }
    if !public_key.verify(&contact_card_signing_bytes(card)?, &card.signature) {
        return Err("assinatura do cartão de contato inválida".into());
    }
    if now_millis().saturating_sub(card.created_at).abs() > 90 * 24 * 60 * 60 * 1_000 {
        return Err("cartão de contato expirado".into());
    }
    Ok(())
}

fn contact_to_card(contact: &ContactRecord) -> ContactCard {
    ContactCard {
        version: 1,
        peer_id: contact.peer_id.clone(),
        display_name: contact.display_name.clone(),
        public_key: contact.public_key.clone(),
        x25519_public_key: contact.x25519_public_key.clone(),
        addresses: Vec::new(),
        created_at: contact.updated_at,
        signature: Vec::new(),
    }
}

fn conversation_id_for(left: &str, right: &str) -> String {
    let mut peers = [left, right];
    peers.sort_unstable();
    format!("dm:{}:{}", peers[0], peers[1])
}

fn new_direct_envelope(
    state: &NodeState,
    recipient: &ContactCard,
    kind: &str,
    body: DirectBody,
) -> Result<DirectEnvelope, String> {
    let keypair = ensure_keypair(state)?;
    let (secret, public) = ensure_agreement_keypair()?;
    let mut envelope = DirectEnvelope {
        event_id: Uuid::new_v4().to_string(),
        kind: kind.into(),
        from_peer_id: keypair.public().to_peer_id().to_string(),
        from_public_key: keypair.public().encode_protobuf(),
        from_x25519_public_key: public.to_vec(),
        to_peer_id: recipient.peer_id.clone(),
        created_at: now_millis(),
        nonce: Vec::new(),
        ciphertext: Vec::new(),
        signature: Vec::new(),
    };
    let plaintext =
        serde_json::to_vec(&body).map_err(|error| format!("mensagem direta inválida: {error}"))?;
    let (nonce, ciphertext) = encrypt_for_recipient(
        &secret,
        &recipient.x25519_public_key,
        &plaintext,
        &direct_aad(&envelope)?,
    )?;
    envelope.nonce = nonce;
    envelope.ciphertext = ciphertext;
    envelope.signature = keypair
        .sign(&direct_signing_bytes(&envelope)?)
        .map_err(|error| format!("não foi possível assinar mensagem direta: {error}"))?;
    Ok(envelope)
}

fn direct_record(envelope: &DirectEnvelope) -> DirectMessageRecord {
    DirectMessageRecord {
        event_id: envelope.event_id.clone(),
        conversation_id: conversation_id_for(&envelope.from_peer_id, &envelope.to_peer_id),
        from_peer_id: envelope.from_peer_id.clone(),
        to_peer_id: envelope.to_peer_id.clone(),
        from_public_key: envelope.from_public_key.clone(),
        from_x25519_public_key: envelope.from_x25519_public_key.clone(),
        created_at: envelope.created_at,
        nonce: envelope.nonce.clone(),
        ciphertext: envelope.ciphertext.clone(),
        signature: envelope.signature.clone(),
    }
}

fn decrypt_direct_body(
    _state: &NodeState,
    envelope: &DirectEnvelope,
) -> Result<DirectBody, String> {
    let (secret, _) = ensure_agreement_keypair()?;
    let plaintext = decrypt_from_sender(
        &secret,
        &envelope.from_x25519_public_key,
        &envelope.nonce,
        &envelope.ciphertext,
        &direct_aad(envelope)?,
    )?;
    serde_json::from_slice(&plaintext).map_err(|error| format!("mensagem direta inválida: {error}"))
}

fn friend_view(_state: &NodeState, contact: ContactRecord) -> FriendView {
    FriendView {
        peer_id: contact.peer_id,
        display_name: contact.display_name,
        status: contact.status,
        requested_by: contact.requested_by,
        online: now_millis().saturating_sub(contact.last_seen) < 45_000,
        updated_at: contact.updated_at,
    }
}

fn direct_message_view(
    state: &NodeState,
    message: DirectMessageRecord,
) -> Result<DirectMessageView, String> {
    let envelope = DirectEnvelope {
        event_id: message.event_id.clone(),
        kind: "direct_message".into(),
        from_peer_id: message.from_peer_id.clone(),
        from_public_key: message.from_public_key.clone(),
        from_x25519_public_key: message.from_x25519_public_key.clone(),
        to_peer_id: message.to_peer_id.clone(),
        created_at: message.created_at,
        nonce: message.nonce.clone(),
        ciphertext: message.ciphertext.clone(),
        signature: message.signature.clone(),
    };
    let body = decrypt_direct_body(state, &envelope)?;
    let local_peer_id = ensure_keypair(state)?.public().to_peer_id().to_string();
    let author = if message.from_peer_id == local_peer_id {
        "Você".into()
    } else {
        state
            .database
            .get_contact(&message.from_peer_id)?
            .map(|contact| contact.display_name)
            .unwrap_or_else(|| message.from_peer_id.chars().take(12).collect())
    };
    Ok(DirectMessageView {
        id: message.event_id,
        conversation_id: message.conversation_id,
        from_peer_id: message.from_peer_id.clone(),
        to_peer_id: message.to_peer_id,
        author,
        content: body.content.unwrap_or_default(),
        created_at: message.created_at,
        mine: message.from_peer_id == local_peer_id,
    })
}

fn enqueue_and_publish_direct(
    _app: &AppHandle,
    state: &NodeState,
    envelope: DirectEnvelope,
) -> Result<(), String> {
    state.database.enqueue_outbox(
        &envelope.event_id,
        "direct",
        &serde_json::to_string(&envelope)
            .map_err(|error| format!("envelope direto inválido: {error}"))?,
        envelope.created_at,
    )?;
    if let Some(sender) = state
        .command_tx
        .lock()
        .map_err(|_| "controle do node indisponível")?
        .clone()
    {
        sender
            .send(NodeCommand::PublishDirect {
                envelope: Box::new(envelope),
            })
            .map_err(|_| "node não está aceitando mensagens privadas".to_string())?;
    }
    Ok(())
}

fn process_direct_envelope(
    app: &AppHandle,
    state: &NodeState,
    envelope: DirectEnvelope,
) -> Result<(), String> {
    let local_peer_id = ensure_keypair(state)?.public().to_peer_id().to_string();
    if envelope.to_peer_id != local_peer_id {
        return Ok(());
    }
    verify_author(
        &envelope.from_public_key,
        &envelope.from_peer_id,
        &direct_signing_bytes(&envelope)?,
        &envelope.signature,
    )?;
    let body = decrypt_direct_body(state, &envelope)?;
    let now = now_millis();
    match envelope.kind.as_str() {
        "friend_request" | "friend_accept" | "friend_reject" | "friend_block" => {
            let card = body.contact_card.ok_or("cartão de contato ausente")?;
            validate_contact_card(&card)?;
            if card.peer_id != envelope.from_peer_id {
                return Err("cartão de contato não corresponde ao remetente".into());
            }
            for address in &card.addresses {
                if let Ok(address) = address.parse::<Multiaddr>() {
                    state.database.remember_peer_address(
                        &card.peer_id,
                        &address.to_string(),
                        "contact",
                        now,
                    )?;
                }
            }
            let existing = state.database.get_contact(&card.peer_id)?;
            if existing
                .as_ref()
                .is_some_and(|contact| contact.status == "blocked")
                && envelope.kind != "friend_block"
            {
                return Ok(());
            }
            let status = match envelope.kind.as_str() {
                "friend_request" => {
                    if existing
                        .as_ref()
                        .is_some_and(|contact| contact.status == "accepted")
                    {
                        "accepted"
                    } else {
                        "pending"
                    }
                }
                "friend_accept" => "accepted",
                "friend_reject" => "rejected",
                _ => "blocked",
            };
            state.database.upsert_contact(&ContactRecord {
                peer_id: card.peer_id.clone(),
                display_name: card.display_name.clone(),
                public_key: card.public_key.clone(),
                x25519_public_key: card.x25519_public_key.clone(),
                status: status.into(),
                requested_by: if envelope.kind == "friend_request" {
                    "remote"
                } else {
                    existing
                        .as_ref()
                        .map(|contact| contact.requested_by.as_str())
                        .unwrap_or("remote")
                }
                .into(),
                created_at: existing
                    .as_ref()
                    .map(|contact| contact.created_at)
                    .unwrap_or(now),
                updated_at: now,
                last_seen: now,
            })?;
            if envelope.kind == "friend_request" {
                for address in &card.addresses {
                    if let Ok(address) = address.parse::<Multiaddr>() {
                        if let Ok(peer_id) = card.peer_id.parse::<PeerId>() {
                            if let Ok(sender) = state.command_tx.lock() {
                                if let Some(sender) = sender.clone() {
                                    let _ = sender.send(NodeCommand::Dial {
                                        address: peer_dial_address(address, &peer_id),
                                    });
                                }
                            }
                        }
                    }
                }
            }
            if let Some(contact) = state.database.get_contact(&card.peer_id)? {
                let _ = app.emit(
                    EVENT_NAME,
                    NodeEvent {
                        kind: "friend-updated".into(),
                        message: None,
                        snapshot: None,
                        error: None,
                        data: Some(
                            serde_json::to_value(friend_view(state, contact))
                                .map_err(|error| error.to_string())?,
                        ),
                    },
                );
            }
        }
        "direct_message" => {
            let contact = state
                .database
                .get_contact(&envelope.from_peer_id)?
                .ok_or("remetente não é conhecido")?;
            if contact.status != "accepted" {
                return Err("mensagem privada recebida de contato não aceito".into());
            }
            let record = direct_record(&envelope);
            if state.database.insert_direct_message(&record)? {
                let view = direct_message_view(state, record)?;
                let _ = app.emit(
                    EVENT_NAME,
                    NodeEvent {
                        kind: "direct-message".into(),
                        message: None,
                        snapshot: None,
                        error: None,
                        data: Some(serde_json::to_value(view).map_err(|error| error.to_string())?),
                    },
                );
            }
        }
        _ => return Err("tipo de mensagem direta desconhecido".into()),
    }
    Ok(())
}

fn announced_address(address: Multiaddr, peer_id: &PeerId) -> Option<Multiaddr> {
    if address.iter().any(|protocol| match protocol {
        Protocol::Ip4(ip) => ip.is_unspecified(),
        Protocol::Ip6(ip) => ip.is_unspecified(),
        _ => false,
    }) {
        None
    } else {
        Some(peer_dial_address(address, peer_id))
    }
}

fn emit_peer_presence(app: &AppHandle, peer_id: &PeerId, state: &str, source: &str) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "peer-presence".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::json!({
                "peer_id": peer_id.to_string(),
                "state": state,
                "last_seen": now_millis(),
                "source": source,
            })),
        },
    );
}

fn local_presence_announcement(state: &NodeState) -> Result<PresenceAnnouncement, String> {
    let keypair = ensure_keypair(state)?;
    let peer_id = keypair.public().to_peer_id().to_string();
    let active_calls = state
        .call_states
        .lock()
        .map_err(|_| "estado de call bloqueado")?
        .values()
        .filter_map(|call| {
            call.participants
                .iter()
                .find(|participant| participant.peer_id == peer_id)
                .map(|participant| PresenceCall {
                    group_id: call.group_id.clone(),
                    channel_id: call.channel_id.clone(),
                    call_id: call.call_id.clone(),
                    display_name: participant.display_name.clone(),
                })
        })
        .collect();
    let mut announcement = PresenceAnnouncement {
        event_id: Uuid::new_v4().to_string(),
        peer_id,
        public_key: keypair.public().encode_protobuf(),
        state: "online".into(),
        active_calls,
        created_at: now_millis(),
        signature: Vec::new(),
    };
    announcement.signature = keypair
        .sign(&protocol::presence_signing_bytes(&announcement)?)
        .map_err(|error| format!("não foi possível assinar presença: {error}"))?;
    Ok(announcement)
}

fn request_presence_broadcast(state: &NodeState) {
    if let Ok(sender) = state.command_tx.lock() {
        if let Some(sender) = sender.clone() {
            let _ = sender.send(NodeCommand::BroadcastPresence);
        }
    }
}

fn verify_presence(announcement: &PresenceAnnouncement) -> Result<PeerId, String> {
    let public_key = libp2p::identity::PublicKey::try_decode_protobuf(&announcement.public_key)
        .map_err(|error| format!("chave pública de presença inválida: {error}"))?;
    let peer_id = public_key.to_peer_id();
    if peer_id.to_string() != announcement.peer_id {
        return Err("presença não corresponde ao PeerId anunciado".into());
    }
    if now_millis().saturating_sub(announcement.created_at).abs() > 120_000 {
        return Err("presença expirada".into());
    }
    if !public_key.verify(
        &protocol::presence_signing_bytes(announcement)?,
        &announcement.signature,
    ) {
        return Err("assinatura de presença inválida".into());
    }
    Ok(peer_id)
}

fn process_presence(
    app: &AppHandle,
    state: &NodeState,
    announcement: PresenceAnnouncement,
) -> Result<(), String> {
    let peer_id = verify_presence(&announcement)?;
    let now = now_millis();
    state.database.remember_peer(&announcement.peer_id, now)?;
    let active_call_keys = announcement
        .active_calls
        .iter()
        .map(|call| format!("{}:{}:{}", call.group_id, call.channel_id, call.call_id))
        .collect::<HashSet<_>>();
    let changed_states = {
        let mut states = state
            .call_states
            .lock()
            .map_err(|_| "estado de call bloqueado")?;
        for call in &announcement.active_calls {
            let key = format!("{}:{}:{}", call.group_id, call.channel_id, call.call_id);
            let Some(member) = state
                .database
                .get_member(&call.group_id, &announcement.peer_id)?
            else {
                continue;
            };
            let current = states.entry(key).or_insert_with(|| CallState {
                group_id: call.group_id.clone(),
                channel_id: call.channel_id.clone(),
                call_id: call.call_id.clone(),
                participants: Vec::new(),
            });
            if !current
                .participants
                .iter()
                .any(|participant| participant.peer_id == announcement.peer_id)
            {
                if current.participants.len() < MAX_CALL_PARTICIPANTS {
                    current.participants.push(CallParticipant {
                        peer_id: announcement.peer_id.clone(),
                        display_name: call.display_name.clone(),
                        role: member.role,
                        muted: false,
                        sharing_screen: false,
                        connection_state: "connected".into(),
                    });
                }
            } else if let Some(participant) = current
                .participants
                .iter_mut()
                .find(|participant| participant.peer_id == announcement.peer_id)
            {
                participant.display_name = call.display_name.clone();
                participant.connection_state = "connected".into();
            }
        }
        let keys = states.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let Some(current) = states.get_mut(&key) else {
                continue;
            };
            if !active_call_keys.contains(&key) {
                current
                    .participants
                    .retain(|participant| participant.peer_id != announcement.peer_id);
            }
        }
        let empty = states
            .iter()
            .filter_map(|(key, current)| current.participants.is_empty().then_some(key.clone()))
            .collect::<Vec<_>>();
        for key in empty {
            states.remove(&key);
        }
        states.values().cloned().collect::<Vec<_>>()
    };
    for call_state in changed_states {
        emit_call_state(app, state, &call_state)?;
    }
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "peer-presence".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::json!({
                "peer_id": announcement.peer_id,
                "state": announcement.state,
                "last_seen": now,
                "source": "presence",
                "active_calls": announcement.active_calls,
            })),
        },
    );
    let _ = peer_id;
    Ok(())
}

fn remember_peer_address(
    known_peer_addresses: &mut HashMap<PeerId, Vec<Multiaddr>>,
    address: &Multiaddr,
) {
    let Some(peer_id) = address.iter().find_map(|protocol| match protocol {
        Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    }) else {
        return;
    };
    let addresses = known_peer_addresses.entry(peer_id).or_default();
    if !addresses.iter().any(|known| known == address) {
        addresses.push(address.clone());
    }
}

fn listen_on(swarm: &mut Swarm<Behaviour>, address: &str) -> Result<(), String> {
    swarm
        .listen_on(
            address
                .parse()
                .map_err(|error| format!("endereço inválido: {error}"))?,
        )
        .map_err(|error| format!("não foi possível escutar em {address}: {error}"))?;
    Ok(())
}

fn configure_relay(swarm: &mut Swarm<Behaviour>, address: Multiaddr) -> Result<(), String> {
    if !address
        .iter()
        .any(|protocol| matches!(protocol, Protocol::P2pCircuit))
    {
        swarm
            .listen_on(address.clone().with(Protocol::P2pCircuit))
            .map_err(|error| format!("não foi possível reservar relay: {error}"))?;
    }
    swarm
        .dial(address)
        .map_err(|error| format!("não foi possível conectar ao relay: {error}"))?;
    Ok(())
}

fn subscribe_group(
    swarm: &mut Swarm<Behaviour>,
    database: &Database,
    group_id: &str,
    subscribed: &mut HashSet<String>,
) -> Result<(), String> {
    let control_topic = topic_for_control(group_id);
    if subscribed.insert(control_topic.clone()) {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&IdentTopic::new(control_topic))
            .map_err(|error| format!("não foi possível assinar controles do grupo: {error}"))?;
    }
    for channel in database.list_channels(group_id)? {
        let topic_name = topic_for(&channel.id);
        if subscribed.insert(topic_name.clone()) {
            swarm
                .behaviour_mut()
                .gossipsub
                .subscribe(&IdentTopic::new(topic_name))
                .map_err(|error| format!("não foi possível assinar canal: {error}"))?;
        }
        let call_topic = topic_for_call(&channel.id);
        if subscribed.insert(call_topic.clone()) {
            swarm
                .behaviour_mut()
                .gossipsub
                .subscribe(&IdentTopic::new(call_topic))
                .map_err(|error| format!("não foi possível assinar call: {error}"))?;
        }
    }
    Ok(())
}

fn flush_outbox(
    swarm: &mut Swarm<Behaviour>,
    state: &NodeState,
    app: &AppHandle,
) -> Result<(), String> {
    for record in state.database.list_outbox(500)? {
        match record.kind.as_str() {
            "message" => {
                let envelope: MessageEnvelope = serde_json::from_str(&record.payload)
                    .map_err(|error| format!("mensagem pendente inválida: {error}"))?;
                let topic = IdentTopic::new(topic_for(&envelope.channel_id));
                let payload = serde_json::to_vec(&envelope)
                    .map_err(|error| format!("mensagem pendente inválida: {error}"))?;
                match swarm.behaviour_mut().gossipsub.publish(topic, payload) {
                    Ok(_) => {
                        state.database.remove_outbox(&record.event_id)?;
                    }
                    Err(error) => {
                        state
                            .database
                            .mark_outbox_attempt(&record.event_id, now_millis())?;
                        if is_waiting_publish_error(&error) {
                            emit_sync_waiting(app);
                        }
                    }
                }
            }
            "control" => {
                let event: ControlEvent = serde_json::from_str(&record.payload)
                    .map_err(|error| format!("evento administrativo pendente inválido: {error}"))?;
                let topic = IdentTopic::new(topic_for_control(&event.group_id));
                let payload = serde_json::to_vec(&event)
                    .map_err(|error| format!("evento administrativo pendente inválido: {error}"))?;
                match swarm.behaviour_mut().gossipsub.publish(topic, payload) {
                    Ok(_) => {
                        state.database.remove_outbox(&record.event_id)?;
                    }
                    Err(error) => {
                        state
                            .database
                            .mark_outbox_attempt(&record.event_id, now_millis())?;
                        if is_waiting_publish_error(&error) {
                            emit_sync_waiting(app);
                        }
                    }
                }
            }
            "direct" => {
                let envelope: DirectEnvelope = serde_json::from_str(&record.payload)
                    .map_err(|error| format!("envelope direto pendente inválido: {error}"))?;
                let Ok(peer_id) = envelope.to_peer_id.parse::<PeerId>() else {
                    state
                        .database
                        .mark_outbox_attempt(&record.event_id, now_millis())?;
                    continue;
                };
                if swarm
                    .connected_peers()
                    .any(|connected| connected == &peer_id)
                {
                    swarm
                        .behaviour_mut()
                        .direct
                        .send_request(&peer_id, envelope);
                    state.database.remove_outbox(&record.event_id)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn request_sync(
    swarm: &mut Swarm<Behaviour>,
    database: &Database,
    peer_id: &PeerId,
) -> Result<(), String> {
    let remote_peer_id = peer_id.to_string();
    for group in database.list_groups()? {
        let (after_control_timestamp, after_control_event_id) =
            initial_sync_cursor(database, &remote_peer_id, &group.id, "")?;
        swarm.behaviour_mut().sync.send_request(
            peer_id,
            SyncRequest {
                group_id: group.id.clone(),
                channel_id: None,
                after_created_at: 0,
                after_event_id: String::new(),
                after_control_timestamp,
                after_control_event_id,
                limit: 500,
            },
        );
        for channel in database
            .list_channels(&group.id)?
            .into_iter()
            .filter(|channel| channel.kind == "text" || channel.kind == "voice")
        {
            let (after_created_at, after_event_id) =
                initial_sync_cursor(database, &remote_peer_id, &group.id, &channel.id)?;
            swarm.behaviour_mut().sync.send_request(
                peer_id,
                SyncRequest {
                    group_id: group.id.clone(),
                    channel_id: Some(channel.id),
                    after_created_at,
                    after_event_id,
                    after_control_timestamp: 0,
                    after_control_event_id: String::new(),
                    limit: 500,
                },
            );
        }
    }
    Ok(())
}

fn initial_sync_cursor(
    database: &Database,
    remote_peer_id: &str,
    group_id: &str,
    channel_id: &str,
) -> Result<(i64, String), String> {
    Ok(database
        .get_sync_cursor(remote_peer_id, group_id, channel_id)?
        .unwrap_or((0, String::new())))
}

fn request_sync_page(
    swarm: &mut Swarm<Behaviour>,
    peer_id: &PeerId,
    request: SyncRequest,
) -> Result<(), String> {
    swarm.behaviour_mut().sync.send_request(peer_id, request);
    Ok(())
}

fn advance_sync_cursor(
    database: &Database,
    peer_id: &PeerId,
    response: &SyncResponse,
) -> Result<Option<SyncRequest>, String> {
    let remote_peer_id = peer_id.to_string();
    let response_channel_id = response
        .next
        .as_ref()
        .and_then(|request| request.channel_id.clone())
        .or_else(|| {
            response
                .messages
                .first()
                .map(|message| message.channel_id.clone())
        });
    if let Some(channel_id) = response_channel_id.as_deref() {
        let mut persisted_prefix = None;
        for message in &response.messages {
            if database.has_message_event(&message.event_id)? {
                persisted_prefix = Some((message.created_at, message.event_id.as_str()));
            } else {
                break;
            }
        }
        if let Some((created_at, event_id)) = persisted_prefix {
            let group_id = response
                .messages
                .first()
                .map(|message| message.group_id.as_str())
                .ok_or("resposta de sincronização sem grupo")?;
            database.set_sync_cursor(
                &remote_peer_id,
                group_id,
                channel_id,
                created_at,
                event_id,
            )?;
            let all_messages_persisted = response
                .messages
                .last()
                .is_some_and(|message| message.event_id == event_id);
            if !all_messages_persisted {
                return Ok(None);
            }
        } else if !response.messages.is_empty() {
            return Ok(None);
        }
    } else if !response.controls.is_empty() {
        let mut persisted_prefix = None;
        for event in &response.controls {
            if database.has_audit_event(&event.event_id)? {
                persisted_prefix = Some((event.logical_timestamp, event.event_id.as_str()));
            } else {
                break;
            }
        }
        if let Some((logical_timestamp, event_id)) = persisted_prefix {
            let group_id = response
                .controls
                .first()
                .map(|event| event.group_id.as_str())
                .ok_or("resposta administrativa sem grupo")?;
            database.set_sync_cursor(&remote_peer_id, group_id, "", logical_timestamp, event_id)?;
            let all_controls_persisted = response
                .controls
                .last()
                .is_some_and(|event| event.event_id == event_id);
            if !all_controls_persisted {
                return Ok(None);
            }
        } else {
            return Ok(None);
        }
    }
    let Some(next) = response.next.clone() else {
        return Ok(None);
    };
    if let Some(channel_id) = next.channel_id.as_deref() {
        let cursor = database
            .get_sync_cursor(&remote_peer_id, &next.group_id, channel_id)?
            .unwrap_or((0, String::new()));
        return Ok(Some(SyncRequest {
            after_created_at: cursor.0,
            after_event_id: cursor.1,
            ..next
        }));
    }
    let cursor = database
        .get_sync_cursor(&remote_peer_id, &next.group_id, "")?
        .unwrap_or((0, String::new()));
    Ok(Some(SyncRequest {
        after_control_timestamp: cursor.0,
        after_control_event_id: cursor.1,
        ..next
    }))
}

fn request_sync_all(swarm: &mut Swarm<Behaviour>, database: &Database) -> Result<(), String> {
    let peers = swarm.connected_peers().copied().collect::<Vec<_>>();
    for peer_id in peers {
        request_sync(swarm, database, &peer_id)?;
    }
    Ok(())
}

fn build_sync_response(
    database: &Database,
    request: &SyncRequest,
    requester_peer_id: &PeerId,
) -> Result<SyncResponse, String> {
    let requester = database
        .get_member(&request.group_id, &requester_peer_id.to_string())?
        .ok_or("peer não é membro deste grupo")?;
    if requester.status != "active" {
        return Err("peer não está ativo neste grupo".into());
    }
    let limit = request.limit.min(500);
    let messages = if let Some(channel_id) = request.channel_id.as_deref() {
        database.list_messages_after(
            &request.group_id,
            channel_id,
            request.after_created_at,
            &request.after_event_id,
            limit,
        )?
    } else {
        Vec::new()
    };
    let controls = if request.channel_id.is_none() {
        database
            .list_audit_events_after(
                &request.group_id,
                request.after_control_timestamp,
                &request.after_control_event_id,
                limit,
            )?
            .into_iter()
            .filter_map(|event| {
                let public_key = database
                    .get_member(&event.group_id, &event.issuer_peer_id)
                    .ok()
                    .flatten()
                    .map(|member| member.public_key)?;
                Some(ControlEvent {
                    event_id: event.event_id,
                    group_id: event.group_id,
                    key_epoch: event.key_epoch,
                    issuer_peer_id: event.issuer_peer_id,
                    issuer_public_key: public_key,
                    kind: event.kind,
                    target_peer_id: event.target_peer_id,
                    payload: event.payload,
                    logical_timestamp: event.logical_timestamp,
                    signature: event.signature,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    let next = if request.channel_id.is_some() && messages.len() >= limit as usize {
        messages.last().map(|message| SyncRequest {
            after_created_at: message.created_at,
            after_event_id: message.event_id.clone(),
            ..request.clone()
        })
    } else if request.channel_id.is_none() && controls.len() >= limit as usize {
        controls.last().map(|event| SyncRequest {
            after_control_timestamp: event.logical_timestamp,
            after_control_event_id: event.event_id.clone(),
            ..request.clone()
        })
    } else {
        None
    };
    Ok(SyncResponse {
        messages,
        controls,
        next,
    })
}

fn process_incoming(app: &AppHandle, state: &NodeState, envelope: MessageEnvelope) {
    let result = (|| {
        if !state.database.has_group(&envelope.group_id)? {
            return Ok(());
        }
        let signed = signing_bytes(&envelope)?;
        verify_author(
            &envelope.author_public_key,
            &envelope.author_peer_id,
            &signed,
            &envelope.signature,
        )?;
        let Some(member) = state
            .database
            .get_member(&envelope.group_id, &envelope.author_peer_id)?
        else {
            queue_pending_message(state, envelope);
            return Ok(());
        };
        if member.public_key != envelope.author_public_key {
            return Err("mensagem de membro não autorizado".into());
        }
        if !state.database.member_is_active_at(
            &envelope.group_id,
            &envelope.author_peer_id,
            envelope.created_at,
        )? {
            return Err("mensagem de membro não autorizado".into());
        }
        if state
            .database
            .timeout_until_at(
                &envelope.group_id,
                &envelope.author_peer_id,
                envelope.created_at,
            )?
            .is_some_and(|until| until > envelope.created_at / 1000)
        {
            return Err("mensagem de membro em timeout".into());
        }
        let group_key = match load_group_key_epoch(&envelope.group_id, envelope.key_epoch) {
            Ok(group_key) => group_key,
            Err(_error)
                if envelope.key_epoch
                    >= current_group_epoch(&state.database, &envelope.group_id)? =>
            {
                queue_pending_message(state, envelope);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let message = envelope_to_chat(&envelope, &group_key)?;
        if state.database.insert_message(&envelope)?
            && !state.database.is_message_deleted(&envelope.event_id)?
        {
            let _ = app.emit(
                EVENT_NAME,
                NodeEvent {
                    kind: "message".into(),
                    message: Some(message),
                    snapshot: None,
                    error: None,
                    data: None,
                },
            );
        }
        flush_pending_controls(app, state, &envelope.group_id);
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        emit_error(app, error);
    }
}

fn queue_pending_message(state: &NodeState, envelope: MessageEnvelope) {
    if let Ok(mut pending) = state.pending_messages.lock() {
        if pending.len() < 2048
            && pending
                .iter()
                .all(|queued| queued.event_id != envelope.event_id)
        {
            pending.push_back(envelope);
        }
    }
}

fn flush_pending_messages(app: &AppHandle, state: &NodeState, group_id: &str) {
    let pending = match state.pending_messages.lock() {
        Ok(mut queue) => {
            let mut matching = Vec::new();
            let mut remaining = VecDeque::new();
            while let Some(message) = queue.pop_front() {
                if message.group_id == group_id {
                    matching.push(message);
                } else {
                    remaining.push_back(message);
                }
            }
            *queue = remaining;
            matching
        }
        Err(_) => Vec::new(),
    };
    for message in pending {
        process_incoming(app, state, message);
    }
}

fn queue_pending_control(state: &NodeState, event: ControlEvent) {
    if let Ok(mut queue) = state.pending_controls.lock() {
        if queue.len() < 1024
            && queue
                .iter()
                .all(|pending| pending.event_id != event.event_id)
        {
            queue.push_back(event);
        }
    }
}

fn flush_pending_controls(app: &AppHandle, state: &NodeState, group_id: &str) {
    let pending = match state.pending_controls.lock() {
        Ok(mut queue) => {
            let mut matching = Vec::new();
            let mut remaining = VecDeque::new();
            while let Some(event) = queue.pop_front() {
                if event.group_id == group_id {
                    matching.push(event);
                } else {
                    remaining.push_back(event);
                }
            }
            *queue = remaining;
            matching
        }
        Err(_) => Vec::new(),
    };
    for event in pending {
        process_control_event(app, state, event);
    }
}

fn queue_pending_call_signal(state: &NodeState, signal: CallSignal) {
    if let Ok(mut queue) = state.pending_call_signals.lock() {
        if queue.len() < 2048
            && queue
                .iter()
                .all(|pending| pending.event_id != signal.event_id)
        {
            queue.push_back(signal);
        }
    }
}

fn flush_pending_call_signals(app: &AppHandle, state: &NodeState, group_id: &str) {
    let pending = match state.pending_call_signals.lock() {
        Ok(mut queue) => {
            let mut matching = Vec::new();
            let mut remaining = VecDeque::new();
            while let Some(signal) = queue.pop_front() {
                if signal.group_id == group_id {
                    matching.push(signal);
                } else {
                    remaining.push_back(signal);
                }
            }
            *queue = remaining;
            matching
        }
        Err(_) => Vec::new(),
    };
    for signal in pending {
        process_call_signal(app, state, signal);
    }
}

fn process_control_event(app: &AppHandle, state: &NodeState, event: ControlEvent) {
    let result = (|| {
        verify_control_event(&event)?;
        if !state.database.has_group(&event.group_id)? {
            return Ok(());
        }
        let self_join = event.kind == CONTROL_MEMBER_JOINED
            && event.target_peer_id.as_deref() == Some(event.issuer_peer_id.as_str());
        if self_join {
            validate_join_event(state, &event)?;
        }
        let issuer = state
            .database
            .get_member(&event.group_id, &event.issuer_peer_id)?;
        if !self_join && issuer.is_none() {
            queue_pending_control(state, event.clone());
            return Ok(());
        }
        let missing_target = event
            .target_peer_id
            .as_deref()
            .map(|target_peer_id| {
                state
                    .database
                    .get_member(&event.group_id, target_peer_id)
                    .map(|member| member.is_none())
            })
            .transpose()?
            .unwrap_or(false);
        if missing_target
            && matches!(
                event.kind.as_str(),
                CONTROL_ROLE_CHANGED
                    | CONTROL_MEMBER_KICKED
                    | CONTROL_MEMBER_BANNED
                    | CONTROL_MEMBER_UNBANNED
                    | CONTROL_MEMBER_TIMEOUT
                    | CONTROL_GROUP_OWNER_TRANSFERRED
                    | CONTROL_KEY_SHARE
            )
        {
            queue_pending_control(state, event.clone());
            return Ok(());
        }
        if event.kind == CONTROL_KEY_SHARE {
            let payload = decode_key_share_payload(&event)?;
            if !key_share_is_ready(state, &event, &payload)? {
                queue_pending_control(state, event.clone());
                return Ok(());
            }
        }
        let authorized = if self_join {
            true
        } else {
            let issuer = issuer
                .as_ref()
                .ok_or("autoridade administrativa desconhecida")?;
            match event.kind.as_str() {
                CONTROL_ROLE_CHANGED => {
                    let Some(target_peer_id) = event.target_peer_id.as_deref() else {
                        return Err("alvo do cargo ausente".into());
                    };
                    let target = state
                        .database
                        .get_member(&event.group_id, target_peer_id)?
                        .ok_or("membro alvo desconhecido")?;
                    let next_role_value = event
                        .payload
                        .get("role")
                        .and_then(serde_json::Value::as_str)
                        .ok_or("cargo ausente")?;
                    let next_role = Role::parse(next_role_value)?;
                    role_change_is_authorized(state, &event.group_id, issuer, &target, &next_role)?
                }
                CONTROL_CHANNEL_CREATED
                | CONTROL_CHANNEL_UPDATED
                | CONTROL_CHANNEL_DELETED
                | CONTROL_CHANNEL_REORDERED
                | CONTROL_CHANNEL_PERMISSION_UPDATED => {
                    has_permission(&issuer.role, &Permission::ManageChannels)
                }
                CONTROL_MESSAGE_DELETED => {
                    let message_id = event
                        .payload
                        .get("message_id")
                        .and_then(serde_json::Value::as_str)
                        .ok_or("mensagem alvo ausente")?;
                    let channel_id = event
                        .payload
                        .get("channel_id")
                        .and_then(serde_json::Value::as_str)
                        .ok_or("canal da mensagem ausente")?;
                    match state.database.get_message(message_id)? {
                        Some(message) => {
                            message.group_id == event.group_id
                                && message.channel_id == channel_id
                                && (message.author_peer_id == issuer.peer_id
                                    || has_permission(&issuer.role, &Permission::ModerateMessages))
                        }
                        None if has_permission(&issuer.role, &Permission::ModerateMessages)
                            || event.target_peer_id.as_deref() == Some(issuer.peer_id.as_str()) =>
                        {
                            true
                        }
                        None => {
                            queue_pending_control(state, event.clone());
                            return Ok(());
                        }
                    }
                }
                CONTROL_MEMBER_JOINED => false,
                CONTROL_MEMBER_KICKED | CONTROL_MEMBER_BANNED | CONTROL_MEMBER_TIMEOUT => {
                    let Some(target_peer_id) = event.target_peer_id.as_deref() else {
                        return Err("membro alvo ausente".into());
                    };
                    let target = state
                        .database
                        .get_member(&event.group_id, target_peer_id)?
                        .ok_or("membro alvo desconhecido")?;
                    target.status == "active"
                        && can_manage_target(&issuer.role, &target.role)
                        && (has_permission(&issuer.role, &Permission::ManageMembers)
                            || has_permission(&issuer.role, &Permission::ModerateMessages))
                }
                CONTROL_MEMBER_LEFT => {
                    event.target_peer_id.as_deref() == Some(event.issuer_peer_id.as_str())
                }
                CONTROL_MEMBER_UNBANNED => {
                    let Some(target_peer_id) = event.target_peer_id.as_deref() else {
                        return Err("membro alvo ausente".into());
                    };
                    let target = state
                        .database
                        .get_member(&event.group_id, target_peer_id)?
                        .ok_or("membro alvo desconhecido")?;
                    target.status == "banned"
                        && can_manage_target(&issuer.role, &target.role)
                        && has_permission(&issuer.role, &Permission::ManageMembers)
                }
                CONTROL_KEY_ROTATED => {
                    has_permission(&issuer.role, &Permission::ManageMembers)
                        || has_permission(&issuer.role, &Permission::ModerateMessages)
                }
                CONTROL_KEY_SHARE => {
                    has_permission(&issuer.role, &Permission::ManageMembers)
                        || has_permission(&issuer.role, &Permission::ModerateMessages)
                }
                CONTROL_GROUP_OWNER_TRANSFERRED | CONTROL_GROUP_DELETED => {
                    let group = state
                        .database
                        .get_group(&event.group_id)?
                        .ok_or("grupo administrativo desconhecido")?;
                    issuer.peer_id == group.owner_peer_id && issuer.role == Role::Owner
                }
                _ => return Err("tipo de evento administrativo desconhecido".into()),
            }
        };
        if issuer
            .as_ref()
            .is_some_and(|member| member.status != "active")
            || !authorized
        {
            return Err("membro não possui permissão para este evento".into());
        }
        if member_event_is_stale(state, &event)? {
            let _ = state.database.insert_audit_event(&control_record(&event))?;
            return Ok(());
        }
        if !state.database.insert_audit_event(&control_record(&event))? {
            return Ok(());
        }
        apply_control_event(state, &event)?;
        if event.kind == CONTROL_MEMBER_JOINED {
            subscribe_group_if_running(state, &event.group_id);
        }
        if matches!(
            event.kind.as_str(),
            CONTROL_MEMBER_KICKED
                | CONTROL_MEMBER_BANNED
                | CONTROL_MEMBER_LEFT
                | CONTROL_GROUP_DELETED
        ) {
            for call_state in remove_call_participants(
                state,
                &event.group_id,
                if event.kind == CONTROL_GROUP_DELETED {
                    None
                } else {
                    event.target_peer_id.as_deref()
                },
            )? {
                emit_call_state(app, state, &call_state)?;
            }
        }
        flush_pending_call_signals(app, state, &event.group_id);
        flush_pending_messages(app, state, &event.group_id);
        flush_pending_controls(app, state, &event.group_id);
        let _ = app.emit(
            EVENT_NAME,
            NodeEvent {
                kind: "group-control".into(),
                message: None,
                snapshot: None,
                error: None,
                data: Some(serde_json::to_value(&event).map_err(|error| error.to_string())?),
            },
        );
        let refresh_kind =
            if event.kind.starts_with("member_") || event.kind == CONTROL_ROLE_CHANGED {
                Some("member-updated")
            } else if event.kind.starts_with("channel_") {
                Some("channel-updated")
            } else if event.kind == CONTROL_KEY_ROTATED || event.kind == CONTROL_KEY_SHARE {
                Some("key-epoch-changed")
            } else {
                None
            };
        if let Some(kind) = refresh_kind {
            let _ = app.emit(
                EVENT_NAME,
                NodeEvent {
                    kind: kind.into(),
                    message: None,
                    snapshot: None,
                    error: None,
                    data: Some(serde_json::to_value(&event).map_err(|error| error.to_string())?),
                },
            );
        }
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        emit_error(app, error);
    }
}

fn apply_control_event(state: &NodeState, event: &ControlEvent) -> Result<(), String> {
    if event.kind == CONTROL_GROUP_DELETED {
        state.database.delete_group(&event.group_id)?;
        return Ok(());
    }
    if event.kind == CONTROL_GROUP_OWNER_TRANSFERRED {
        let target_peer_id = event
            .target_peer_id
            .as_deref()
            .ok_or("novo Owner ausente")?;
        let group = state
            .database
            .get_group(&event.group_id)?
            .ok_or("grupo não encontrado")?;
        if group.owner_peer_id != event.issuer_peer_id {
            return Err("transferência emitida por autoridade antiga inválida".into());
        }
        let mut old_owner = state
            .database
            .get_member(&event.group_id, &event.issuer_peer_id)?
            .ok_or("Owner atual não encontrado")?;
        let mut new_owner = state
            .database
            .get_member(&event.group_id, target_peer_id)?
            .ok_or("novo Owner não encontrado")?;
        if new_owner.status != "active" {
            return Err("novo Owner precisa estar ativo".into());
        }
        old_owner.role = Role::Admin;
        old_owner.updated_at = event.logical_timestamp;
        new_owner.role = Role::Owner;
        new_owner.updated_at = event.logical_timestamp;
        state.database.insert_member(&old_owner)?;
        state.database.insert_member(&new_owner)?;
        state
            .database
            .set_owner_peer_id(&event.group_id, target_peer_id)?;
        return Ok(());
    }
    if event.kind == CONTROL_CHANNEL_CREATED || event.kind == CONTROL_CHANNEL_UPDATED {
        let mut channel: ChannelRecord = serde_json::from_value(event.payload.clone())
            .map_err(|error| format!("canal administrativo inválido: {error}"))?;
        if channel.group_id != event.group_id {
            return Err("canal administrativo pertence a outro grupo".into());
        }
        channel.name = validate_channel_name(channel.name)?;
        channel.kind = validate_channel_kind(&channel.kind)?;
        if event.kind == CONTROL_CHANNEL_CREATED {
            state.database.insert_channel(&channel)?;
        } else {
            state.database.update_channel(&channel)?;
        }
        return Ok(());
    }
    if event.kind == CONTROL_CHANNEL_DELETED {
        let channel_id = event
            .payload
            .get("channel_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("canal ausente")?;
        if state
            .database
            .list_channels(&event.group_id)?
            .iter()
            .any(|channel| channel.id == channel_id && channel.name == "geral")
        {
            return Err("o canal geral não pode ser excluído".into());
        }
        state.database.delete_channel(&event.group_id, channel_id)?;
        return Ok(());
    }
    if event.kind == CONTROL_CHANNEL_REORDERED {
        let channel_ids: Vec<String> = serde_json::from_value(
            event
                .payload
                .get("channel_ids")
                .cloned()
                .ok_or("ordenação de canais ausente")?,
        )
        .map_err(|error| format!("ordenação de canais inválida: {error}"))?;
        let existing = state
            .database
            .list_channels(&event.group_id)?
            .into_iter()
            .map(|channel| channel.id)
            .collect::<HashSet<_>>();
        let requested = channel_ids.iter().cloned().collect::<HashSet<_>>();
        if requested.len() != channel_ids.len() || requested != existing {
            return Err("ordenação administrativa contém canais inválidos".into());
        }
        state
            .database
            .reorder_channels(&event.group_id, &channel_ids)?;
        return Ok(());
    }
    if event.kind == CONTROL_CHANNEL_PERMISSION_UPDATED {
        let permission: ChannelPermissionRecord = serde_json::from_value(event.payload.clone())
            .map_err(|error| format!("permissão de canal inválida: {error}"))?;
        if permission.group_id != event.group_id {
            return Err("permissão de canal pertence a outro grupo".into());
        }
        let channel = state
            .database
            .list_channels(&event.group_id)?
            .into_iter()
            .find(|channel| channel.id == permission.channel_id)
            .ok_or("canal da permissão não encontrado")?;
        let normalized = ChannelPermissionRecord {
            can_join_voice: channel.kind == "voice" && permission.can_join_voice,
            can_share_screen: channel.kind == "voice" && permission.can_share_screen,
            ..permission
        };
        state.database.set_channel_permission(&normalized)?;
        return Ok(());
    }
    if event.kind == CONTROL_MESSAGE_DELETED {
        let message_id = event
            .payload
            .get("message_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("mensagem alvo ausente")?;
        let channel_id = event
            .payload
            .get("channel_id")
            .and_then(serde_json::Value::as_str)
            .ok_or("canal da mensagem ausente")?;
        if let Some(message) = state.database.get_message(message_id)? {
            if message.group_id != event.group_id || message.channel_id != channel_id {
                return Err("exclusão de mensagem pertence a outro canal".into());
            }
        }
        let _ = state.database.delete_message(
            message_id,
            &event.group_id,
            channel_id,
            &event.issuer_peer_id,
            event.logical_timestamp,
        )?;
        return Ok(());
    }
    if event.kind == CONTROL_KEY_ROTATED {
        let epoch = event
            .payload
            .get("epoch")
            .and_then(serde_json::Value::as_i64)
            .ok_or("época de rotação ausente")?;
        let current = current_group_epoch(&state.database, &event.group_id)?;
        if epoch != event.key_epoch || epoch != current + 1 {
            return Err("rotação de chave fora da sequência esperada".into());
        }
        state
            .database
            .set_current_key_epoch(&event.group_id, epoch)?;
        return Ok(());
    }
    if event.kind == CONTROL_MEMBER_JOINED {
        let Some(peer_id) = event.target_peer_id.as_deref() else {
            return Ok(());
        };
        let public_key_encoded = event
            .payload
            .get("public_key")
            .and_then(serde_json::Value::as_str)
            .ok_or("chave pública de membro ausente")?;
        let public_key = BASE64
            .decode(public_key_encoded)
            .map_err(|error| format!("chave pública de membro inválida: {error}"))?;
        if peer_id == event.issuer_peer_id && public_key != event.issuer_public_key {
            return Err("chave pública de entrada não corresponde ao emissor".into());
        }
        let x25519_public_key = BASE64
            .decode(
                event
                    .payload
                    .get("x25519_public_key")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("chave de acordo de membro ausente")?,
            )
            .map_err(|error| format!("chave de acordo de membro inválida: {error}"))?;
        state.database.insert_member(&MemberRecord {
            group_id: event.group_id.clone(),
            peer_id: peer_id.into(),
            display_name: event
                .payload
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("Membro")
                .into(),
            role: Role::Member,
            public_key,
            x25519_public_key,
            status: "active".into(),
            joined_at: event.logical_timestamp,
            timeout_until: None,
            updated_at: event.logical_timestamp,
        })?;
        return Ok(());
    }
    if event.kind == CONTROL_KEY_SHARE {
        let local_peer_id = ensure_keypair(state)?.public().to_peer_id().to_string();
        if event.target_peer_id.as_deref() != Some(local_peer_id.as_str()) {
            return Ok(());
        }
        let payload = decode_key_share_payload(event)?;
        let secret = ensure_agreement_keypair()?.0;
        let aad = format!(
            "teamscord-key-share-v1:{}:{}:{}",
            event.group_id, payload.epoch, local_peer_id
        );
        let key: [u8; GROUP_KEY_BYTES] = decrypt_from_sender(
            &secret,
            &payload.sender_public_key,
            &payload.nonce,
            &payload.ciphertext,
            aad.as_bytes(),
        )?
        .try_into()
        .map_err(|_| "chave de grupo recebida inválida".to_string())?;
        if let Ok(existing) = load_group_key_epoch(&event.group_id, payload.epoch) {
            if existing != key {
                return Err("conflito entre chaves da mesma época".into());
            }
            return Ok(());
        }
        store_group_key_epoch(&event.group_id, payload.epoch, &key)?;
        state.database.insert_key_epoch(
            &event.group_id,
            payload.epoch,
            &event.issuer_peer_id,
            event.logical_timestamp,
            &hash_key(&key),
        )?;
        let current = current_group_epoch(&state.database, &event.group_id)?;
        if payload.epoch > current {
            state
                .database
                .set_current_key_epoch(&event.group_id, payload.epoch)?;
        }
        return Ok(());
    }
    let Some(target_peer_id) = event.target_peer_id.as_deref() else {
        return Ok(());
    };
    let Some(mut member) = state.database.get_member(&event.group_id, target_peer_id)? else {
        return Ok(());
    };
    match event.kind.as_str() {
        CONTROL_ROLE_CHANGED => {
            let role = event
                .payload
                .get("role")
                .and_then(serde_json::Value::as_str)
                .ok_or("cargo ausente")?;
            member.role = Role::parse(role)?;
        }
        CONTROL_MEMBER_KICKED => member.status = "kicked".into(),
        CONTROL_MEMBER_BANNED => member.status = "banned".into(),
        CONTROL_MEMBER_LEFT => member.status = "left".into(),
        CONTROL_MEMBER_UNBANNED => member.status = "active".into(),
        CONTROL_MEMBER_TIMEOUT => {
            member.timeout_until = event
                .payload
                .get("until")
                .and_then(serde_json::Value::as_i64);
        }
        _ => return Ok(()),
    }
    member.updated_at = event.logical_timestamp;
    state.database.insert_member(&member)
}

fn decode_call_signal_body(signal: &CallSignal) -> Result<CallSignalBody, String> {
    let group_key = load_group_key_epoch(&signal.group_id, signal.key_epoch)?;
    let body_bytes = decrypt_message(
        &group_key,
        &signal.nonce,
        &signal.ciphertext,
        &protocol::call_signal_aad(signal)?,
    )?;
    serde_json::from_str(&body_bytes).map_err(|error| format!("payload de call inválido: {error}"))
}

/// A request-response request is also the delivery mechanism for call signals.
/// When a member joins, the peer that is already in the call must return its own
/// signed join signal so the newcomer can create the matching RTCPeerConnection.
/// This response is deliberately emitted only by an active local participant;
/// merely observing a remote call must not make a node join it implicitly.
fn call_signal_join_response(state: &NodeState, request: &CallSignal) -> Option<CallSignal> {
    let body = decode_call_signal_body(request).ok()?;
    if body.kind != "join" {
        return None;
    }
    let keypair = ensure_keypair(state).ok()?;
    let local_peer_id = keypair.public().to_peer_id().to_string();
    if body
        .to_peer_id
        .as_deref()
        .is_some_and(|peer_id| peer_id != local_peer_id)
        || request.from_peer_id == local_peer_id
    {
        return None;
    }
    let local_member = state
        .database
        .get_member(&request.group_id, &local_peer_id)
        .ok()??;
    if local_member.status != "active"
        || timeout_is_active(local_member.timeout_until, now_seconds())
    {
        return None;
    }
    let remote_member = state
        .database
        .get_member(&request.group_id, &request.from_peer_id)
        .ok()??;
    if remote_member.status != "active" || remote_member.public_key != request.from_public_key {
        return None;
    }
    let channel = state
        .database
        .list_channels(&request.group_id)
        .ok()?
        .into_iter()
        .find(|channel| channel.id == request.channel_id)?;
    if channel.kind == "text"
        || request.key_epoch != current_group_epoch(&state.database, &request.group_id).ok()?
    {
        return None;
    }
    let call_key = format!(
        "{}:{}:{}",
        request.group_id, request.channel_id, request.call_id
    );
    let local_in_call = state
        .call_states
        .lock()
        .ok()?
        .get(&call_key)
        .is_some_and(|current| {
            current
                .participants
                .iter()
                .any(|participant| participant.peer_id == local_peer_id)
        });
    if !local_in_call {
        return None;
    }
    new_call_signal(
        state,
        &request.group_id,
        &request.channel_id,
        &request.call_id,
        Some(request.from_peer_id.clone()),
        "join",
        serde_json::json!({ "display_name": local_member.display_name }),
    )
    .ok()
}

fn process_call_signal(app: &AppHandle, state: &NodeState, signal: CallSignal) {
    let result = (|| {
        if signal.created_at < now_millis() - 5 * 60 * 1000 {
            return Err("sinal de call expirado".into());
        }
        if signal.created_at > now_millis() + 60 * 1000 {
            return Err("sinal de call adiantado demais".into());
        }
        verify_author(
            &signal.from_public_key,
            &signal.from_peer_id,
            &protocol::call_signal_signing_bytes(&signal)?,
            &signal.signature,
        )?;
        if !state.database.has_group(&signal.group_id)? {
            return Ok(());
        }
        let channel = state
            .database
            .list_channels(&signal.group_id)?
            .into_iter()
            .find(|channel| channel.id == signal.channel_id)
            .ok_or("sinal de call usa um canal inexistente")?;
        if channel.kind == "text" || signal.call_id.trim().is_empty() || signal.call_id.len() > 160
        {
            return Err("sinal de call usa parâmetros inválidos".into());
        }
        if signal.key_epoch != current_group_epoch(&state.database, &signal.group_id)? {
            return Err("sinal de call usa uma época de chave antiga".into());
        }
        let member = match state
            .database
            .get_member(&signal.group_id, &signal.from_peer_id)?
        {
            Some(member) => member,
            None => {
                queue_pending_call_signal(state, signal);
                return Ok(());
            }
        };
        if member.status != "active"
            || member.public_key != signal.from_public_key
            || timeout_is_active(member.timeout_until, now_seconds())
        {
            return Err("sinal de call de membro não autorizado".into());
        }
        let local_peer_id = ensure_keypair(state)?.public().to_peer_id().to_string();
        let body = decode_call_signal_body(&signal)?;
        if body
            .to_peer_id
            .as_deref()
            .is_some_and(|peer_id| peer_id != local_peer_id)
        {
            return Ok(());
        }
        if !matches!(
            body.kind.as_str(),
            "join"
                | "leave"
                | "mute"
                | "mute-request"
                | "screen-start"
                | "screen-stop"
                | "offer"
                | "answer"
                | "ice"
                | "peer-state"
        ) {
            return Err("tipo de sinal de call desconhecido".into());
        }
        let channel_permission = effective_channel_permission(
            state,
            &signal.group_id,
            &signal.channel_id,
            &member.role,
        )?;
        if body.kind == "join" && !channel_permission.can_join_voice {
            return Err("membro sem permissão para entrar nesta call".into());
        }
        if body.kind == "screen-start" && !channel_permission.can_share_screen {
            return Err("membro sem permissão para compartilhar tela neste canal".into());
        }
        if body.kind == "mute-request" {
            let target_peer_id = body
                .to_peer_id
                .as_deref()
                .ok_or("destinatário do mute ausente")?;
            let target = state
                .database
                .get_member(&signal.group_id, target_peer_id)?
                .ok_or("destinatário do mute não encontrado")?;
            if !has_permission(&member.role, &Permission::ModerateMessages)
                || target.status != "active"
                || !can_manage_target(&member.role, &target.role)
            {
                return Err("membro não possui autoridade para este mute".into());
            }
            let in_call = state
                .call_states
                .lock()
                .map_err(|_| "estado de call bloqueado")?
                .get(&format!(
                    "{}:{}:{}",
                    signal.group_id, signal.channel_id, signal.call_id
                ))
                .is_some_and(|current| {
                    current
                        .participants
                        .iter()
                        .any(|participant| participant.peer_id == signal.from_peer_id)
                        && current
                            .participants
                            .iter()
                            .any(|participant| participant.peer_id == target_peer_id)
                });
            if !in_call {
                return Err("o destinatário não está nesta call".into());
            }
        }
        {
            let mut handled = state
                .handled_call_signals
                .lock()
                .map_err(|_| "estado de sinais de call bloqueado")?;
            if !handled.insert(signal.event_id.clone()) {
                return Ok(());
            }
            if handled.len() > 8192 {
                handled.clear();
                handled.insert(signal.event_id.clone());
            }
        }
        if let Some(call_state) = update_call_state(state, &signal, &body)? {
            emit_call_state(app, state, &call_state)?;
        }
        let data = serde_json::json!({ "signal": signal, "body": body });
        let _ = app.emit(
            EVENT_NAME,
            NodeEvent {
                kind: "call-signal".into(),
                message: None,
                snapshot: None,
                error: None,
                data: Some(data),
            },
        );
        Ok::<(), String>(())
    })();
    if let Err(error) = result {
        emit_media_error(app, error);
    }
}

fn envelope_to_chat(
    envelope: &MessageEnvelope,
    group_key: &[u8; GROUP_KEY_BYTES],
) -> Result<ChatMessage, String> {
    let content = decrypt_message(
        group_key,
        &envelope.nonce,
        &envelope.ciphertext,
        &aad_for_envelope(envelope)?,
    )?;
    Ok(ChatMessage {
        id: envelope.event_id.clone(),
        group_id: envelope.group_id.clone(),
        channel_id: envelope.channel_id.clone(),
        author_peer_id: envelope.author_peer_id.clone(),
        author: envelope.author_name.clone(),
        initials: envelope.initials.clone(),
        color: envelope.color.clone(),
        content,
        timestamp: timestamp_from_millis(envelope.created_at),
        created_at: envelope.created_at,
        mine: false,
    })
}

fn group_view(database: &Database, group: GroupRecord) -> Result<GroupView, String> {
    Ok(GroupView {
        id: group.id.clone(),
        name: group.name.clone(),
        initials: group.initials.clone(),
        color: group.color.clone(),
        owner_peer_id: group.owner_peer_id.clone(),
        created_at: group.created_at,
        channels: database.list_channels(&group.id)?,
    })
}

fn current_group_epoch(database: &Database, group_id: &str) -> Result<i64, String> {
    database
        .get_group(group_id)?
        .map(|group| group.current_key_epoch)
        .ok_or_else(|| "grupo não encontrado".to_string())
}

fn subscribe_group_if_running(state: &NodeState, group_id: &str) {
    if let Ok(sender) = state.command_tx.lock() {
        if let Some(sender) = sender.clone() {
            let _ = sender.send(NodeCommand::SubscribeGroup {
                group_id: group_id.into(),
            });
            let _ = sender.send(NodeCommand::RequestSync);
        }
    }
}

fn ensure_keypair(state: &NodeState) -> Result<Keypair, String> {
    if let Some(keypair) = state
        .keypair
        .lock()
        .map_err(|_| "identidade bloqueada")?
        .clone()
    {
        return Ok(keypair);
    }
    let keypair = load_or_create_identity()?;
    *state.keypair.lock().map_err(|_| "identidade bloqueada")? = Some(keypair.clone());
    Ok(keypair)
}

fn load_or_create_identity() -> Result<Keypair, String> {
    let entry = Entry::new(IDENTITY_SERVICE, IDENTITY_USERNAME)
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    if let Ok(encoded) = entry.get_password() {
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| format!("identidade salva inválida: {error}"))?;
        return Keypair::from_protobuf_encoding(&bytes)
            .map_err(|error| format!("identidade salva inválida: {error}"));
    }
    let keypair = Keypair::generate_ed25519();
    let encoded = BASE64.encode(
        keypair
            .to_protobuf_encoding()
            .map_err(|error| format!("não foi possível serializar identidade: {error}"))?,
    );
    entry.set_password(&encoded).map_err(|error| {
        format!("não foi possível salvar identidade no Credential Manager: {error}")
    })?;
    Ok(keypair)
}

fn store_group_key_epoch(
    group_id: &str,
    epoch: i64,
    key: &[u8; GROUP_KEY_BYTES],
) -> Result<(), String> {
    let entry = Entry::new(GROUP_KEY_SERVICE, &format!("{group_id}:{epoch}"))
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    entry
        .set_password(&BASE64.encode(key))
        .map_err(|error| format!("não foi possível salvar chave do grupo: {error}"))
}

fn load_group_key_epoch(group_id: &str, epoch: i64) -> Result<[u8; GROUP_KEY_BYTES], String> {
    let epoch_entry = Entry::new(GROUP_KEY_SERVICE, &format!("{group_id}:{epoch}"))
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    let encoded = match epoch_entry.get_password() {
        Ok(encoded) => encoded,
        Err(_) if epoch == 1 => Entry::new(GROUP_KEY_SERVICE, group_id)
            .map_err(|error| format!("Credential Manager indisponível: {error}"))?
            .get_password()
            .map_err(|_| "chave local do grupo não encontrada".to_string())?,
        Err(_) => return Err("chave local da época do grupo não encontrada".into()),
    };
    let bytes = BASE64
        .decode(encoded)
        .map_err(|error| format!("chave local do grupo inválida: {error}"))?;
    bytes
        .try_into()
        .map_err(|_| "chave local do grupo inválida".to_string())
}

fn ensure_agreement_keypair() -> Result<([u8; X25519_KEY_BYTES], [u8; X25519_KEY_BYTES]), String> {
    let entry = Entry::new(AGREEMENT_SERVICE, AGREEMENT_USERNAME)
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    if let Ok(encoded) = entry.get_password() {
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| format!("chave de acordo salva inválida: {error}"))?;
        if bytes.len() != X25519_KEY_BYTES * 2 {
            return Err("chave de acordo salva inválida".into());
        }
        let secret: [u8; X25519_KEY_BYTES] = bytes[..X25519_KEY_BYTES]
            .try_into()
            .map_err(|_| "chave de acordo salva inválida".to_string())?;
        let public: [u8; X25519_KEY_BYTES] = bytes[X25519_KEY_BYTES..]
            .try_into()
            .map_err(|_| "chave de acordo salva inválida".to_string())?;
        return Ok((secret, public));
    }
    let (secret, public) = new_x25519_keypair()?;
    let mut bytes = Vec::with_capacity(X25519_KEY_BYTES * 2);
    bytes.extend_from_slice(&secret);
    bytes.extend_from_slice(&public);
    entry
        .set_password(&BASE64.encode(bytes))
        .map_err(|error| format!("não foi possível salvar chave de acordo: {error}"))?;
    Ok((secret, public))
}

fn hash_key(key: &[u8; GROUP_KEY_BYTES]) -> Vec<u8> {
    Sha256::digest(key).to_vec()
}

fn load_network_config(data_dir: &Path) -> Result<NetworkConfig, String> {
    let path = data_dir.join("network.json");
    if !path.exists() {
        return Ok(NetworkConfig::default());
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("não foi possível ler configuração de rede: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("configuração de rede inválida: {error}"))
}

fn save_network_config(data_dir: &Path, config: &NetworkConfig) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("não foi possível serializar configuração: {error}"))?;
    fs::write(data_dir.join("network.json"), bytes)
        .map_err(|error| format!("não foi possível salvar configuração de rede: {error}"))
}

fn validate_ice_server(mut server: IceServerConfig) -> Result<IceServerConfig, String> {
    server.urls.retain(|url| !url.trim().is_empty());
    if server.urls.is_empty()
        || server.urls.iter().any(|url| {
            !(url.starts_with("stun:")
                || url.starts_with("stuns:")
                || url.starts_with("turn:")
                || url.starts_with("turns:"))
        })
    {
        return Err("servidor ICE deve usar URL stun:, stuns:, turn: ou turns:".into());
    }
    Ok(server)
}

fn load_media_config() -> Result<MediaConfig, String> {
    let entry = Entry::new(MEDIA_CONFIG_SERVICE, MEDIA_CONFIG_USERNAME)
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    let Ok(encoded) = entry.get_password() else {
        return Ok(MediaConfig::default());
    };
    let bytes = BASE64
        .decode(encoded)
        .map_err(|error| format!("configuração de mídia inválida: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("configuração de mídia inválida: {error}"))
}

fn save_media_config(config: &MediaConfig) -> Result<(), String> {
    let entry = Entry::new(MEDIA_CONFIG_SERVICE, MEDIA_CONFIG_USERNAME)
        .map_err(|error| format!("Credential Manager indisponível: {error}"))?;
    let bytes = serde_json::to_vec(config)
        .map_err(|error| format!("não foi possível serializar configuração de mídia: {error}"))?;
    entry
        .set_password(&BASE64.encode(bytes))
        .map_err(|error| format!("não foi possível salvar configuração de mídia: {error}"))
}

fn validate_group_name(name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.len() > 80 {
        return Err("nome do grupo deve ter entre 1 e 80 caracteres".into());
    }
    Ok(name)
}

fn validate_display_name(name: String) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("nome exibido deve ter entre 1 e 40 caracteres".into());
    }
    Ok(name)
}

fn validate_channel_name(name: String) -> Result<String, String> {
    let name = name.trim().replace(' ', "-").to_lowercase();
    if name.is_empty()
        || name.len() > 40
        || !name
            .chars()
            .all(|character| character.is_alphanumeric() || character == '-' || character == '_')
    {
        return Err("nome de canal deve ter entre 1 e 40 caracteres simples".into());
    }
    Ok(name)
}

fn validate_channel_kind(kind: &str) -> Result<String, String> {
    match kind {
        "text" | "voice" => Ok(kind.into()),
        _ => Err("tipo de canal inválido".into()),
    }
}

fn initials_for(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn topic_for(channel_id: &str) -> String {
    format!("teamscord/channel/{channel_id}")
}

fn topic_for_control(group_id: &str) -> String {
    format!("teamscord/control/{group_id}")
}

fn topic_for_call(channel_id: &str) -> String {
    format!("teamscord/call/{channel_id}")
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn now_seconds() -> i64 {
    now_millis() / 1000
}

fn timestamp_from_millis(millis: i64) -> String {
    let seconds = (millis / 1000).max(0);
    format!("{:02}:{:02}", (seconds / 3600) % 24, (seconds / 60) % 60)
}

fn emit_snapshot(app: &AppHandle, snapshot: &NodeSnapshot) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "snapshot".into(),
            message: None,
            snapshot: Some(snapshot.clone()),
            error: None,
            data: None,
        },
    );
}

fn is_waiting_publish_error(error: &gossipsub::PublishError) -> bool {
    matches!(
        error,
        gossipsub::PublishError::NoPeersSubscribedToTopic
            | gossipsub::PublishError::AllQueuesFull(_)
    )
}

fn emit_sync_waiting(app: &AppHandle) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "sync-state".into(),
            message: None,
            snapshot: None,
            error: None,
            data: Some(serde_json::json!({ "state": "waiting" })),
        },
    );
}

fn emit_error(app: &AppHandle, error: String) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "error".into(),
            message: None,
            snapshot: None,
            error: Some(error),
            data: None,
        },
    );
}

fn emit_media_error(app: &AppHandle, error: String) {
    let _ = app.emit(
        EVENT_NAME,
        NodeEvent {
            kind: "media-error".into(),
            message: None,
            snapshot: None,
            error: Some(error),
            data: None,
        },
    );
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    const RELEASE_PREFIX: &str = "https://github.com/PessoasBoas/Teamscord/releases/";
    if !url.starts_with(RELEASE_PREFIX)
        || url
            .chars()
            .any(|character| character == '\r' || character == '\n')
    {
        return Err("URL externa não autorizada".into());
    }

    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(&url).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(&url).spawn();

    result
        .map(|_| ())
        .map_err(|error| format!("não foi possível abrir o navegador: {error}"))
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("não foi possível localizar dados do app: {error}"))?;
            let state = NodeState::open(data_dir).map_err(|error| {
                Box::new(std::io::Error::other(error)) as Box<dyn std::error::Error>
            })?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_node,
            get_node_snapshot,
            run_network_diagnostics,
            get_network_config,
            set_network_config,
            get_media_config,
            set_media_config,
            get_groups,
            create_group,
            create_invite,
            join_group,
            list_group_members,
            get_group_roles,
            get_channel_permissions,
            set_channel_permission,
            get_audit_log,
            update_member_role,
            transfer_group_ownership,
            delete_group,
            kick_member,
            ban_member,
            unban_member,
            timeout_member,
            create_channel,
            update_channel,
            delete_channel,
            reorder_channels,
            join_call,
            leave_call,
            leave_group,
            send_call_signal,
            request_call_mute,
            get_call_state,
            get_messages,
            search_messages,
            send_message,
            delete_message,
            connect_peer,
            get_network_status,
            get_known_peers,
            get_contact_card,
            list_friends,
            create_friend_request,
            respond_friend_request,
            get_direct_messages,
            send_direct_message,
            test_peer_connection,
            open_external_url
        ])
        .run(tauri::generate_context!())
        .expect("erro ao executar Teamscord");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(NetworkBehaviour)]
    struct IntegrationBehaviour {
        gossipsub: gossipsub::Behaviour,
        sync: request_response::json::Behaviour<SyncRequest, SyncResponse>,
        call_signal: request_response::json::Behaviour<CallSignal, CallSignal>,
        direct: request_response::json::Behaviour<DirectEnvelope, DirectEnvelope>,
    }

    fn integration_swarm(keypair: Keypair) -> Swarm<IntegrationBehaviour> {
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .validation_mode(gossipsub::ValidationMode::Strict)
            .build()
            .expect("test GossipSub config");
        let gossipsub = gossipsub::Behaviour::new(
            MessageAuthenticity::Signed(keypair.clone()),
            gossipsub_config,
        )
        .expect("test GossipSub behaviour");
        let sync = request_response::json::Behaviour::new(
            [(StreamProtocol::new(SYNC_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(std::time::Duration::from_secs(5)),
        );
        let call_signal = request_response::json::Behaviour::new(
            [(
                StreamProtocol::new(CALL_SIGNAL_PROTOCOL),
                ProtocolSupport::Full,
            )],
            request_response::Config::default()
                .with_request_timeout(std::time::Duration::from_secs(5)),
        );
        let direct = request_response::json::Behaviour::new(
            [(StreamProtocol::new(DIRECT_PROTOCOL), ProtocolSupport::Full)],
            request_response::Config::default()
                .with_request_timeout(std::time::Duration::from_secs(5)),
        );
        SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )
            .expect("test TCP transport")
            .with_behaviour(|_| IntegrationBehaviour {
                gossipsub,
                sync,
                call_signal,
                direct,
            })
            .expect("test behaviour")
            .build()
    }

    #[test]
    fn two_nodes_exchange_sync_response_over_local_libp2p_transport() {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime.block_on(async {
            let mut owner = integration_swarm(Keypair::generate_ed25519());
            let mut member = integration_swarm(Keypair::generate_ed25519());
            let topic = IdentTopic::new("teamscord/test");
            owner
                .behaviour_mut()
                .gossipsub
                .subscribe(&topic)
                .expect("owner subscribe");
            assert!(matches!(
                owner
                    .behaviour_mut()
                    .gossipsub
                    .publish(topic.clone(), b"early-publish".to_vec()),
                Err(gossipsub::PublishError::NoPeersSubscribedToTopic)
            ));
            owner
                .listen_on("/ip4/127.0.0.1/tcp/0".parse().expect("listen address"))
                .expect("owner listen");
            let owner_address = loop {
                if let SwarmEvent::NewListenAddr { address, .. } = owner.select_next_some().await
                {
                    break address;
                }
            };
            member.dial(owner_address).expect("member dial");
            let mut response_received = false;
            let mut gossip_received = false;
            let mut gossip_published = false;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        event = owner.select_next_some() => {
                            match event {
                                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                    owner.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                }
                                SwarmEvent::Behaviour(IntegrationBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic: subscribed_topic })) if subscribed_topic == topic.hash() && !gossip_published => {
                                    owner.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                    owner.behaviour_mut().gossipsub.publish(topic.clone(), b"gossip-test".to_vec()).expect("owner publish");
                                    gossip_published = true;
                                }
                                SwarmEvent::Behaviour(IntegrationBehaviourEvent::Sync(request_response::Event::Message { message: request_response::Message::Request { channel, .. }, .. })) => {
                                    owner.behaviour_mut().sync.send_response(channel, SyncResponse {
                                        messages: Vec::new(),
                                        controls: Vec::new(),
                                        next: None,
                                    }).expect("owner response");
                                }
                                _ => {}
                            }
                        }
                        event = member.select_next_some() => {
                            match event {
                                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                    member.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                                    member
                                        .behaviour_mut()
                                        .gossipsub
                                        .subscribe(&topic)
                                        .expect("member subscribe after connection");
                                    member.behaviour_mut().sync.send_request(&peer_id, SyncRequest {
                                        group_id: "transport-test".into(),
                                        channel_id: None,
                                        after_created_at: 0,
                                        after_event_id: String::new(),
                                        after_control_timestamp: 0,
                                        after_control_event_id: String::new(),
                                        limit: 1,
                                    });
                                }
                                SwarmEvent::Behaviour(IntegrationBehaviourEvent::Sync(request_response::Event::Message { message: request_response::Message::Response { response, .. }, .. })) => {
                                    assert!(response.messages.is_empty());
                                    assert!(response.controls.is_empty());
                                    response_received = true;
                                }
                                SwarmEvent::Behaviour(IntegrationBehaviourEvent::Gossipsub(gossipsub::Event::Message { message, .. })) => {
                                    assert_eq!(message.data, b"gossip-test".to_vec());
                                    gossip_received = true;
                                }
                                _ => {}
                            }
                            if response_received && gossip_received {
                                break;
                            }
                        }
                    }
                }
            }).await.expect("local sync response timeout");
            assert!(response_received);
            assert!(gossip_received);
        });
    }

    #[test]
    fn two_nodes_exchange_call_signal_over_ephemeral_request_response_protocol() {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime.block_on(async {
            let mut owner = integration_swarm(Keypair::generate_ed25519());
            let mut member = integration_swarm(Keypair::generate_ed25519());
            owner
                .listen_on("/ip4/127.0.0.1/tcp/0".parse().expect("listen address"))
                .expect("owner listen");
            let owner_address = loop {
                if let SwarmEvent::NewListenAddr { address, .. } = owner.select_next_some().await
                {
                    break address;
                }
            };
            member.dial(owner_address).expect("member dial");
            let signal = CallSignal {
                event_id: "call-transport-test".into(),
                group_id: "transport-group".into(),
                channel_id: "transport-group:lounge".into(),
                call_id: "transport-call".into(),
                from_peer_id: member.local_peer_id().to_string(),
                from_public_key: Vec::new(),
                key_epoch: 1,
                created_at: now_millis(),
                nonce: vec![1, 2, 3],
                ciphertext: vec![4, 5, 6],
                signature: vec![7, 8, 9],
            };
            let mut request_sent = false;
            let mut request_received = false;
            let mut response_received = false;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        event = owner.select_next_some() => {
                            if let SwarmEvent::Behaviour(IntegrationBehaviourEvent::CallSignal(request_response::Event::Message {
                                message: request_response::Message::Request { request, channel, .. }, ..
                            })) = event {
                                assert_eq!(request.event_id, signal.event_id);
                                assert_eq!(request.group_id, signal.group_id);
                                assert_eq!(request.ciphertext, signal.ciphertext);
                                owner.behaviour_mut().call_signal.send_response(channel, request).expect("call response");
                                request_received = true;
                            }
                        }
                        event = member.select_next_some() => {
                            match event {
                                SwarmEvent::ConnectionEstablished { peer_id, .. } if !request_sent => {
                                    member.behaviour_mut().call_signal.send_request(&peer_id, signal.clone());
                                    request_sent = true;
                                }
                                SwarmEvent::Behaviour(IntegrationBehaviourEvent::CallSignal(request_response::Event::Message {
                                    message: request_response::Message::Response { response, .. }, ..
                                })) => {
                                    assert_eq!(response.event_id, signal.event_id);
                                    assert_eq!(response.key_epoch, signal.key_epoch);
                                    response_received = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    if request_sent && request_received && response_received { break; }
                }
            }).await.expect("call signal transport timeout");
            assert!(request_sent);
            assert!(request_received);
            assert!(response_received);
        });
    }

    #[test]
    fn timeout_uses_seconds_and_expires_deterministically() {
        assert!(!timeout_is_active(None, 100));
        assert!(timeout_is_active(Some(101), 100));
        assert!(!timeout_is_active(Some(100), 100));
        assert!(!timeout_is_active(Some(99), 100));
    }

    #[test]
    fn fixed_role_target_boundaries_are_enforced() {
        assert!(can_manage_target(&Role::Owner, &Role::Admin));
        assert!(!can_manage_target(&Role::Admin, &Role::Admin));
        assert!(can_manage_target(&Role::Admin, &Role::Mod));
        assert!(can_manage_target(&Role::Mod, &Role::Member));
        assert!(!can_manage_target(&Role::Mod, &Role::Mod));
        assert!(can_update_role(&Role::Owner, &Role::Admin, &Role::Mod));
        assert!(!can_update_role(&Role::Admin, &Role::Member, &Role::Admin));
        assert!(can_update_role(&Role::Admin, &Role::Member, &Role::Mod));
        assert!(!can_update_role(&Role::Owner, &Role::Owner, &Role::Admin));
    }

    #[test]
    fn admin_role_changes_require_owner_delegation_and_channel_permissions_apply() {
        let directory = tempfile::tempdir().expect("directory");
        let state = NodeState::open(directory.path().to_path_buf()).expect("state");
        let group = GroupRecord {
            id: "authorization-group".into(),
            name: "Autorização".into(),
            initials: "AU".into(),
            color: "#8276ff".into(),
            owner_peer_id: "owner".into(),
            created_at: 1,
            current_key_epoch: 1,
        };
        state.database.insert_group(&group).expect("group");
        state
            .database
            .insert_default_channels(&group.id)
            .expect("channels");
        let admin = MemberRecord {
            group_id: group.id.clone(),
            peer_id: "admin".into(),
            display_name: "Admin".into(),
            role: Role::Admin,
            public_key: vec![1],
            x25519_public_key: vec![2; 32],
            status: "active".into(),
            joined_at: 2,
            timeout_until: None,
            updated_at: 2,
        };
        let target = MemberRecord {
            group_id: group.id.clone(),
            peer_id: "member".into(),
            display_name: "Membro".into(),
            role: Role::Member,
            public_key: vec![3],
            x25519_public_key: vec![4; 32],
            status: "active".into(),
            joined_at: 2,
            timeout_until: None,
            updated_at: 2,
        };
        state.database.insert_member(&admin).expect("admin");
        state.database.insert_member(&target).expect("target");
        assert!(
            !role_change_is_authorized(&state, &group.id, &admin, &target, &Role::Mod)
                .expect("authorization")
        );
        state
            .database
            .insert_audit_event(&AuditEventRecord {
                event_id: "owner-admin-delegation".into(),
                group_id: group.id.clone(),
                key_epoch: 1,
                issuer_peer_id: group.owner_peer_id.clone(),
                kind: CONTROL_ROLE_CHANGED.into(),
                target_peer_id: Some(admin.peer_id.clone()),
                payload: serde_json::json!({ "role": "admin" }),
                logical_timestamp: 3,
                signature: vec![5],
            })
            .expect("delegation");
        assert!(
            role_change_is_authorized(&state, &group.id, &admin, &target, &Role::Mod)
                .expect("authorization")
        );
        state
            .database
            .set_channel_permission(&ChannelPermissionRecord {
                group_id: group.id.clone(),
                channel_id: format!("{}:lounge", group.id),
                role: Role::Member,
                can_join_voice: true,
                can_share_screen: false,
            })
            .expect("channel permission");
        let permission = effective_channel_permission(
            &state,
            &group.id,
            &format!("{}:lounge", group.id),
            &Role::Member,
        )
        .expect("read channel permission");
        assert!(permission.can_join_voice);
        assert!(!permission.can_share_screen);
    }

    #[test]
    fn sync_requires_active_membership_and_paginates_controls() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();
        let group = GroupRecord {
            id: "sync-group".into(),
            name: "Sync".into(),
            initials: "SY".into(),
            color: "#8276ff".into(),
            owner_peer_id: peer_id.to_string(),
            created_at: 1,
            current_key_epoch: 1,
        };
        database.insert_group(&group).expect("group");
        database
            .insert_member(&MemberRecord {
                group_id: group.id.clone(),
                peer_id: peer_id.to_string(),
                display_name: "Owner".into(),
                role: Role::Owner,
                public_key: keypair.public().encode_protobuf(),
                x25519_public_key: vec![7; X25519_KEY_BYTES],
                status: "active".into(),
                joined_at: 1,
                timeout_until: None,
                updated_at: 1,
            })
            .expect("member");
        for index in 0..501 {
            assert!(database
                .insert_audit_event(&AuditEventRecord {
                    event_id: format!("control-{index:03}"),
                    group_id: group.id.clone(),
                    key_epoch: 1,
                    issuer_peer_id: peer_id.to_string(),
                    kind: CONTROL_MEMBER_TIMEOUT.into(),
                    target_peer_id: Some(peer_id.to_string()),
                    payload: serde_json::json!({ "until": index }),
                    logical_timestamp: index,
                    signature: vec![1],
                })
                .expect("audit"));
        }
        let request = SyncRequest {
            group_id: group.id.clone(),
            channel_id: None,
            after_created_at: 0,
            after_event_id: String::new(),
            after_control_timestamp: 0,
            after_control_event_id: String::new(),
            limit: 500,
        };
        let first = build_sync_response(&database, &request, &peer_id).expect("first page");
        assert_eq!(first.controls.len(), 500);
        let next = first.next.expect("continuation");
        let second = build_sync_response(&database, &next, &peer_id).expect("second page");
        assert_eq!(second.controls.len(), 1);
        assert!(second.next.is_none());

        let mut member = database
            .get_member(&group.id, &peer_id.to_string())
            .expect("read")
            .expect("member");
        member.status = "banned".into();
        database.insert_member(&member).expect("ban");
        assert!(build_sync_response(&database, &request, &peer_id).is_err());
    }

    #[test]
    fn sync_cursor_waits_for_persisted_prefix_before_requesting_next_page() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        let group = GroupRecord {
            id: "cursor-group".into(),
            name: "Cursor".into(),
            initials: "CU".into(),
            color: "#8276ff".into(),
            owner_peer_id: "owner".into(),
            created_at: 1,
            current_key_epoch: 1,
        };
        database.insert_group(&group).expect("group");
        database
            .insert_default_channels(&group.id)
            .expect("channels");
        let message = |event_id: &str, created_at: i64| MessageEnvelope {
            event_id: event_id.into(),
            group_id: group.id.clone(),
            channel_id: format!("{}:geral", group.id),
            author_peer_id: "owner".into(),
            author_public_key: vec![1],
            author_name: "Owner".into(),
            initials: "OW".into(),
            color: "#8276ff".into(),
            created_at,
            key_epoch: 1,
            nonce: vec![1],
            ciphertext: vec![2],
            signature: vec![3],
        };
        let first = message("message-1", 1);
        let second = message("message-2", 2);
        database.insert_message(&first).expect("first message");
        let remote_peer = Keypair::generate_ed25519().public().to_peer_id();
        let response = SyncResponse {
            messages: vec![first.clone(), second.clone()],
            controls: Vec::new(),
            next: Some(SyncRequest {
                group_id: group.id.clone(),
                channel_id: Some(format!("{}:geral", group.id)),
                after_created_at: first.created_at,
                after_event_id: first.event_id.clone(),
                after_control_timestamp: 0,
                after_control_event_id: String::new(),
                limit: 500,
            }),
        };
        assert!(advance_sync_cursor(&database, &remote_peer, &response)
            .expect("cursor parcial")
            .is_none());
        assert_eq!(
            database
                .get_sync_cursor(
                    &remote_peer.to_string(),
                    &group.id,
                    &format!("{}:geral", group.id)
                )
                .expect("ler cursor")
                .expect("cursor inicial"),
            (first.created_at, first.event_id.clone())
        );
        database.insert_message(&second).expect("second message");
        let next = advance_sync_cursor(&database, &remote_peer, &response)
            .expect("cursor completo")
            .expect("próxima página");
        assert_eq!(next.after_created_at, second.created_at);
        assert_eq!(next.after_event_id, second.event_id);
    }

    #[test]
    fn first_sync_with_a_peer_starts_at_zero_and_reuses_persisted_cursor() {
        let directory = tempfile::tempdir().expect("tempdir");
        let database = Database::open(directory.path().join("teamscord.sqlite")).expect("db");
        let remote_peer_id = Keypair::generate_ed25519()
            .public()
            .to_peer_id()
            .to_string();
        assert_eq!(
            initial_sync_cursor(&database, &remote_peer_id, "group", "").expect("initial cursor"),
            (0, String::new())
        );
        database
            .set_sync_cursor(&remote_peer_id, "group", "", 42, "control-42")
            .expect("persist cursor");
        assert_eq!(
            initial_sync_cursor(&database, &remote_peer_id, "group", "").expect("persisted cursor"),
            (42, "control-42".into())
        );
    }

    #[test]
    fn network_addresses_are_normalized_and_legacy_config_keeps_bootstrap_optional() {
        let addresses = normalize_multiaddr_list(
            vec![
                " /ip4/127.0.0.1/tcp/4001 ".into(),
                "/ip4/127.0.0.1/tcp/4001".into(),
                String::new(),
            ],
            "bootstrap",
        )
        .expect("normalize bootstrap");
        assert_eq!(addresses, vec!["/ip4/127.0.0.1/tcp/4001"]);
        assert!(normalize_multiaddr_list(vec!["not-a-multiaddr".into()], "relay").is_err());

        let directory = tempfile::tempdir().expect("tempdir");
        fs::write(
            directory.path().join("network.json"),
            r#"{"relay_addresses":["/ip4/127.0.0.1/tcp/4001"]}"#,
        )
        .expect("legacy network config");
        let config = load_network_config(directory.path()).expect("load legacy network config");
        assert_eq!(config.relay_addresses, vec!["/ip4/127.0.0.1/tcp/4001"]);
        assert!(config.bootstrap_addresses.is_empty());

        let persisted = NetworkConfig {
            relay_addresses: config.relay_addresses,
            bootstrap_addresses: addresses,
        };
        save_network_config(directory.path(), &persisted).expect("save network config");
        assert_eq!(
            load_network_config(directory.path())
                .expect("reload network config")
                .bootstrap_addresses,
            vec!["/ip4/127.0.0.1/tcp/4001"]
        );
    }

    #[test]
    fn default_relay_is_available_without_manual_network_setup() {
        let addresses = default_relay_addresses();
        let expected = option_env!("TEAMSCORD_DEFAULT_RELAY_ADDRESS")
            .unwrap_or(BUILTIN_DEFAULT_RELAY_ADDRESS)
            .to_string();
        assert_eq!(addresses, vec![expected]);
    }

    #[test]
    fn reconnect_address_adds_peer_identity_without_overwriting_existing_identity() {
        let peer_id = Keypair::generate_ed25519().public().to_peer_id();
        let direct: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().expect("direct address");
        let with_peer: Multiaddr = format!("{direct}/p2p/{peer_id}")
            .parse()
            .expect("identified address");
        assert_eq!(peer_dial_address(direct, &peer_id), with_peer);
        assert_eq!(peer_dial_address(with_peer.clone(), &peer_id), with_peer);
        let mut known = HashMap::new();
        remember_peer_address(&mut known, &with_peer);
        remember_peer_address(&mut known, &with_peer);
        assert_eq!(known.get(&peer_id), Some(&vec![with_peer]));
    }

    #[test]
    fn media_config_accepts_stun_turn_and_rejects_invalid_schemes() {
        let normalized = validate_ice_server(IceServerConfig {
            urls: vec![
                " ".into(),
                "stun:stun.example.test:3478".into(),
                "turns:turn.example.test:5349?transport=tcp".into(),
            ],
            username: Some("user".into()),
            credential: Some("credential".into()),
        })
        .expect("valid ICE configuration");
        assert_eq!(normalized.urls.len(), 2);
        assert_eq!(normalized.urls[0], "stun:stun.example.test:3478");
        assert_eq!(
            normalized.urls[1],
            "turns:turn.example.test:5349?transport=tcp"
        );

        assert!(validate_ice_server(IceServerConfig {
            urls: vec!["https://turn.example.test".into()],
            username: None,
            credential: None,
        })
        .is_err());
        assert!(validate_ice_server(IceServerConfig {
            urls: vec![" ".into()],
            username: None,
            credential: None,
        })
        .is_err());
    }

    #[test]
    fn two_node_protocol_flow_syncs_history_and_call_presence_after_reopen() {
        let owner_directory = tempfile::tempdir().expect("owner tempdir");
        let member_directory = tempfile::tempdir().expect("member tempdir");
        let owner_keypair = Keypair::generate_ed25519();
        let member_keypair = Keypair::generate_ed25519();
        let owner_peer_id = owner_keypair.public().to_peer_id().to_string();
        let member_peer_id = member_keypair.public().to_peer_id().to_string();
        let group_id = format!("two-node-{}", Uuid::new_v4());
        let channel_id = format!("{group_id}:geral");
        let call_channel_id = format!("{group_id}:lounge");
        let group_key = new_group_key().expect("group key");
        let owner_state = NodeState::open(owner_directory.path().to_path_buf()).expect("owner");
        let member_state = NodeState::open(member_directory.path().to_path_buf()).expect("member");
        *owner_state.keypair.lock().expect("owner identity lock") = Some(owner_keypair.clone());
        *member_state.keypair.lock().expect("member identity lock") = Some(member_keypair.clone());

        let group = GroupRecord {
            id: group_id.clone(),
            name: "Dois Nodes".into(),
            initials: "DN".into(),
            color: "#8276ff".into(),
            owner_peer_id: owner_peer_id.clone(),
            created_at: 1,
            current_key_epoch: 1,
        };
        for database in [&owner_state.database, &member_state.database] {
            database.insert_group(&group).expect("group");
            database
                .insert_default_channels(&group_id)
                .expect("default channels");
            database
                .insert_key_epoch(&group_id, 1, &owner_peer_id, 1, &hash_key(&group_key))
                .expect("key epoch");
        }
        store_group_key_epoch(&group_id, 1, &group_key).expect("owner key");
        let owner_x25519 = [9u8; X25519_KEY_BYTES];
        let member_x25519 = [8u8; X25519_KEY_BYTES];
        let owner_member = MemberRecord {
            group_id: group_id.clone(),
            peer_id: owner_peer_id.clone(),
            display_name: "Owner".into(),
            role: Role::Owner,
            public_key: owner_keypair.public().encode_protobuf(),
            x25519_public_key: owner_x25519.to_vec(),
            status: "active".into(),
            joined_at: 1,
            timeout_until: None,
            updated_at: 1,
        };
        owner_state
            .database
            .insert_member(&owner_member)
            .expect("owner member");
        member_state
            .database
            .insert_member(&owner_member)
            .expect("owner mirror");
        let invite = crypto::create_invite(
            &owner_keypair,
            &group_id,
            &group.name,
            &group.initials,
            &group.color,
            &group_key,
            1,
            &BASE64.encode(owner_x25519),
            now_seconds() + 600,
        )
        .expect("invite");
        let mut join_event = ControlEvent {
            event_id: Uuid::new_v4().to_string(),
            group_id: group_id.clone(),
            key_epoch: 1,
            issuer_peer_id: member_peer_id.clone(),
            issuer_public_key: member_keypair.public().encode_protobuf(),
            kind: CONTROL_MEMBER_JOINED.into(),
            target_peer_id: Some(member_peer_id.clone()),
            payload: serde_json::json!({
                "display_name": "Membro",
                "public_key": BASE64.encode(member_keypair.public().encode_protobuf()),
                "x25519_public_key": BASE64.encode(member_x25519),
            }),
            logical_timestamp: 2,
            signature: Vec::new(),
        };
        let (proof_nonce, proof_ciphertext) = encrypt_message(
            &group_key,
            &invite,
            &join_proof_aad(&join_event).expect("join aad"),
        )
        .expect("join proof");
        join_event.payload["invite_nonce"] = serde_json::json!(BASE64.encode(proof_nonce));
        join_event.payload["invite_ciphertext"] =
            serde_json::json!(BASE64.encode(proof_ciphertext));
        sign_control_event(&mut join_event, &member_keypair).expect("join signature");
        validate_join_event(&owner_state, &join_event).expect("owner validates join");
        apply_control_event(&owner_state, &join_event).expect("owner applies join");
        apply_control_event(&member_state, &join_event).expect("member applies own join");
        for database in [&owner_state.database, &member_state.database] {
            database
                .insert_audit_event(&control_record(&join_event))
                .expect("join audit");
        }

        let mut envelope = MessageEnvelope {
            event_id: "two-node-message".into(),
            group_id: group_id.clone(),
            channel_id: channel_id.clone(),
            author_peer_id: owner_peer_id.clone(),
            author_public_key: owner_keypair.public().encode_protobuf(),
            author_name: "Owner".into(),
            initials: "OW".into(),
            color: "#8276ff".into(),
            created_at: 3,
            key_epoch: 1,
            nonce: Vec::new(),
            ciphertext: Vec::new(),
            signature: Vec::new(),
        };
        let (nonce, ciphertext) = encrypt_message(
            &group_key,
            "mensagem sincronizada",
            &aad_for_envelope(&envelope).expect("message aad"),
        )
        .expect("message encryption");
        envelope.nonce = nonce;
        envelope.ciphertext = ciphertext;
        envelope.signature = owner_keypair
            .sign(&signing_bytes(&envelope).expect("message signing bytes"))
            .expect("message signature");
        owner_state
            .database
            .insert_message(&envelope)
            .expect("owner message");
        let response = build_sync_response(
            &owner_state.database,
            &SyncRequest {
                group_id: group_id.clone(),
                channel_id: Some(channel_id.clone()),
                after_created_at: 0,
                after_event_id: String::new(),
                after_control_timestamp: 0,
                after_control_event_id: String::new(),
                limit: 500,
            },
            &member_keypair.public().to_peer_id(),
        )
        .expect("sync response");
        assert_eq!(response.messages.len(), 1);
        verify_author(
            &envelope.author_public_key,
            &envelope.author_peer_id,
            &signing_bytes(&response.messages[0]).expect("sync signing bytes"),
            &response.messages[0].signature,
        )
        .expect("synced signature");
        let synced_text = decrypt_message(
            &group_key,
            &response.messages[0].nonce,
            &response.messages[0].ciphertext,
            &aad_for_envelope(&response.messages[0]).expect("sync aad"),
        )
        .expect("synced message");
        assert_eq!(synced_text, "mensagem sincronizada");
        member_state
            .database
            .insert_message(&response.messages[0])
            .expect("member message");
        let reopened_member = Database::open(member_directory.path().join("teamscord.sqlite"))
            .expect("reopen member database");
        assert_eq!(
            reopened_member
                .list_messages(&group_id, &channel_id, 50)
                .expect("history")
                .len(),
            1
        );

        let call_id = format!("{group_id}:{call_channel_id}");
        let mut call_state = CallState {
            group_id: group_id.clone(),
            channel_id: call_channel_id.clone(),
            call_id: call_id.clone(),
            participants: vec![CallParticipant {
                peer_id: owner_peer_id.clone(),
                display_name: "Owner".into(),
                role: Role::Owner,
                muted: false,
                sharing_screen: false,
                connection_state: "connected".into(),
            }],
        };
        owner_state
            .call_states
            .lock()
            .expect("owner call lock")
            .insert(
                format!("{group_id}:{call_channel_id}:{call_id}"),
                call_state.clone(),
            );
        let join_signal = CallSignal {
            event_id: "two-node-call-join".into(),
            group_id: group_id.clone(),
            channel_id: call_channel_id.clone(),
            call_id: call_id.clone(),
            from_peer_id: member_peer_id.clone(),
            from_public_key: member_keypair.public().encode_protobuf(),
            key_epoch: 1,
            created_at: now_millis(),
            nonce: vec![1],
            ciphertext: vec![1],
            signature: vec![1],
        };
        let call_update = update_call_state(
            &owner_state,
            &join_signal,
            &CallSignalBody {
                to_peer_id: None,
                kind: "join".into(),
                payload: serde_json::json!({ "display_name": "Membro" }),
            },
        )
        .expect("call join")
        .expect("call state");
        assert_eq!(call_update.participants.len(), 2);
        call_state.participants = vec![CallParticipant {
            peer_id: member_peer_id,
            display_name: "Membro".into(),
            role: Role::Member,
            muted: false,
            sharing_screen: false,
            connection_state: "connected".into(),
        }];
        member_state
            .call_states
            .lock()
            .expect("member call lock")
            .insert(
                format!("{group_id}:{call_channel_id}:{call_id}"),
                call_state,
            );
        let owner_join_signal = CallSignal {
            event_id: "two-node-call-owner-join".into(),
            from_peer_id: owner_peer_id,
            from_public_key: owner_keypair.public().encode_protobuf(),
            ..join_signal
        };
        let member_call_update = update_call_state(
            &member_state,
            &owner_join_signal,
            &CallSignalBody {
                to_peer_id: None,
                kind: "join".into(),
                payload: serde_json::json!({ "display_name": "Owner" }),
            },
        )
        .expect("member call join")
        .expect("member call state");
        assert_eq!(member_call_update.participants.len(), 2);
    }

    #[test]
    fn encrypted_join_proof_rejects_tampering_and_wrong_group() {
        let directory = tempfile::tempdir().expect("tempdir");
        let state = NodeState::open(directory.path().to_path_buf()).expect("state");
        let owner = ensure_keypair(&state).expect("identity");
        let owner_peer_id = owner.public().to_peer_id().to_string();
        let group_id = format!("join-proof-{}", Uuid::new_v4());
        let group_key = new_group_key().expect("group key");
        let owner_x25519 = [9u8; X25519_KEY_BYTES];
        state
            .database
            .insert_group(&GroupRecord {
                id: group_id.clone(),
                name: "Private".into(),
                initials: "PR".into(),
                color: "#8276ff".into(),
                owner_peer_id: owner_peer_id.clone(),
                created_at: 1,
                current_key_epoch: 1,
            })
            .expect("group");
        state
            .database
            .insert_member(&MemberRecord {
                group_id: group_id.clone(),
                peer_id: owner_peer_id.clone(),
                display_name: "Owner".into(),
                role: Role::Owner,
                public_key: owner.public().encode_protobuf(),
                x25519_public_key: owner_x25519.to_vec(),
                status: "active".into(),
                joined_at: 1,
                timeout_until: None,
                updated_at: 1,
            })
            .expect("owner");
        store_group_key_epoch(&group_id, 1, &group_key).expect("key");
        state
            .database
            .insert_key_epoch(&group_id, 1, &owner_peer_id, 1, &hash_key(&group_key))
            .expect("epoch");
        let invite = crypto::create_invite(
            &owner,
            &group_id,
            "Private",
            "PR",
            "#8276ff",
            &group_key,
            1,
            &BASE64.encode(owner_x25519),
            now_seconds() + 600,
        )
        .expect("invite");
        let joiner = Keypair::generate_ed25519();
        let (_, joiner_x25519) = crypto::new_x25519_keypair().expect("agreement");
        let mut event = ControlEvent {
            event_id: Uuid::new_v4().to_string(),
            group_id: group_id.clone(),
            key_epoch: 1,
            issuer_peer_id: joiner.public().to_peer_id().to_string(),
            issuer_public_key: joiner.public().encode_protobuf(),
            kind: CONTROL_MEMBER_JOINED.into(),
            target_peer_id: Some(joiner.public().to_peer_id().to_string()),
            payload: serde_json::json!({
                "display_name": "Joiner",
                "public_key": BASE64.encode(joiner.public().encode_protobuf()),
                "x25519_public_key": BASE64.encode(joiner_x25519),
            }),
            logical_timestamp: now_millis(),
            signature: Vec::new(),
        };
        let (nonce, ciphertext) =
            encrypt_message(&group_key, &invite, &join_proof_aad(&event).expect("aad"))
                .expect("proof");
        event.payload["invite_nonce"] = serde_json::json!(BASE64.encode(nonce));
        event.payload["invite_ciphertext"] = serde_json::json!(BASE64.encode(ciphertext));
        sign_control_event(&mut event, &joiner).expect("sign");
        validate_join_event(&state, &event).expect("valid join proof");
        event.payload["invite_ciphertext"] = serde_json::json!("tampered");
        assert!(validate_join_event(&state, &event).is_err());
    }

    #[test]
    fn call_state_is_discovered_and_leave_preserves_other_participants() {
        let directory = tempfile::tempdir().expect("tempdir");
        let state = NodeState::open(directory.path().to_path_buf()).expect("state");
        let local = Keypair::generate_ed25519();
        let remote = Keypair::generate_ed25519();
        let local_peer_id = local.public().to_peer_id().to_string();
        let remote_peer_id = remote.public().to_peer_id().to_string();
        let group_id = "call-group".to_string();
        let channel_id = format!("{group_id}:voice");
        let call_id = format!("{group_id}:{channel_id}");
        state
            .database
            .insert_group(&GroupRecord {
                id: group_id.clone(),
                name: "Calls".into(),
                initials: "CA".into(),
                color: "#8276ff".into(),
                owner_peer_id: local_peer_id.clone(),
                created_at: 1,
                current_key_epoch: 1,
            })
            .expect("group");
        state
            .database
            .insert_default_channels(&group_id)
            .expect("channels");
        for (keypair, role, name) in [
            (&local, Role::Owner, "Local"),
            (&remote, Role::Member, "Remote"),
        ] {
            state
                .database
                .insert_member(&MemberRecord {
                    group_id: group_id.clone(),
                    peer_id: keypair.public().to_peer_id().to_string(),
                    display_name: name.into(),
                    role,
                    public_key: keypair.public().encode_protobuf(),
                    x25519_public_key: vec![7; X25519_KEY_BYTES],
                    status: "active".into(),
                    joined_at: 1,
                    timeout_until: None,
                    updated_at: 1,
                })
                .expect("member");
        }
        let make_signal = |keypair: &Keypair| CallSignal {
            event_id: Uuid::new_v4().to_string(),
            group_id: group_id.clone(),
            channel_id: channel_id.clone(),
            call_id: call_id.clone(),
            from_peer_id: keypair.public().to_peer_id().to_string(),
            from_public_key: keypair.public().encode_protobuf(),
            key_epoch: 1,
            created_at: now_millis(),
            nonce: vec![1],
            ciphertext: vec![1],
            signature: vec![1],
        };
        let join_body = |name: &str| CallSignalBody {
            to_peer_id: None,
            kind: "join".into(),
            payload: serde_json::json!({ "display_name": name }),
        };
        let remote_join = make_signal(&remote);
        let first = update_call_state(&state, &remote_join, &join_body("Remote"))
            .expect("remote join")
            .expect("call created");
        assert_eq!(first.participants.len(), 1);
        assert_eq!(first.participants[0].peer_id, remote_peer_id);

        let remote_state = make_signal(&remote);
        let connected = update_call_state(
            &state,
            &remote_state,
            &CallSignalBody {
                to_peer_id: None,
                kind: "peer-state".into(),
                payload: serde_json::json!({ "connection_state": "connected" }),
            },
        )
        .expect("connection state")
        .expect("call state");
        assert_eq!(connected.participants[0].connection_state, "connected");
        let invalid_state = make_signal(&remote);
        let normalized = update_call_state(
            &state,
            &invalid_state,
            &CallSignalBody {
                to_peer_id: None,
                kind: "peer-state".into(),
                payload: serde_json::json!({ "connection_state": "spoofed" }),
            },
        )
        .expect("invalid connection state")
        .expect("call state");
        assert_eq!(normalized.participants[0].connection_state, "connecting");

        let mute = make_signal(&remote);
        update_call_state(
            &state,
            &mute,
            &CallSignalBody {
                to_peer_id: None,
                kind: "mute".into(),
                payload: serde_json::json!({ "muted": true }),
            },
        )
        .expect("mute");
        let screen = make_signal(&remote);
        update_call_state(
            &state,
            &screen,
            &CallSignalBody {
                to_peer_id: None,
                kind: "screen-start".into(),
                payload: serde_json::json!({}),
            },
        )
        .expect("screen");

        let local_join = make_signal(&local);
        let joined = update_call_state(&state, &local_join, &join_body("Local"))
            .expect("local join")
            .expect("call state");
        assert_eq!(joined.participants.len(), 2);
        let remote_participant = joined
            .participants
            .iter()
            .find(|participant| participant.peer_id == remote_peer_id)
            .expect("remote participant");
        assert!(remote_participant.muted);
        assert!(remote_participant.sharing_screen);
        let competing_screen = make_signal(&local);
        assert!(update_call_state(
            &state,
            &competing_screen,
            &CallSignalBody {
                to_peer_id: None,
                kind: "screen-start".into(),
                payload: serde_json::json!({}),
            },
        )
        .is_err());

        let removed = remove_call_participants(&state, &group_id, Some(&remote_peer_id))
            .expect("remove remote call participant");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].participants.len(), 1);
        assert_eq!(removed[0].participants[0].peer_id, local_peer_id);

        let remote_leave = make_signal(&remote);
        let after_remote_leave = update_call_state(
            &state,
            &remote_leave,
            &CallSignalBody {
                to_peer_id: None,
                kind: "leave".into(),
                payload: serde_json::json!({}),
            },
        )
        .expect("remote leave")
        .expect("remaining local participant");
        assert_eq!(after_remote_leave.participants.len(), 1);
        assert_eq!(after_remote_leave.participants[0].peer_id, local_peer_id);

        let local_leave = make_signal(&local);
        let empty = update_call_state(
            &state,
            &local_leave,
            &CallSignalBody {
                to_peer_id: None,
                kind: "leave".into(),
                payload: serde_json::json!({}),
            },
        )
        .expect("local leave")
        .expect("empty call state");
        assert!(empty.participants.is_empty());
        assert!(!state
            .call_states
            .lock()
            .expect("call state lock")
            .contains_key(&format!("{group_id}:{channel_id}:{call_id}")));
    }

    #[test]
    fn call_join_handshake_returns_existing_participant_signal() {
        let directory = tempfile::tempdir().expect("tempdir");
        let state = NodeState::open(directory.path().to_path_buf()).expect("state");
        let local = Keypair::generate_ed25519();
        let remote = Keypair::generate_ed25519();
        *state.keypair.lock().expect("identity lock") = Some(local.clone());
        let group_id = "call-handshake-group".to_string();
        let channel_id = format!("{group_id}:lounge");
        let call_id = format!("{group_id}:{channel_id}");
        let local_peer_id = local.public().to_peer_id().to_string();
        let remote_peer_id = remote.public().to_peer_id().to_string();
        let group_key = new_group_key().expect("group key");
        state
            .database
            .insert_group(&GroupRecord {
                id: group_id.clone(),
                name: "Handshake".into(),
                initials: "HA".into(),
                color: "#8276ff".into(),
                owner_peer_id: local_peer_id.clone(),
                created_at: 1,
                current_key_epoch: 1,
            })
            .expect("group");
        state
            .database
            .insert_default_channels(&group_id)
            .expect("channels");
        for (keypair, role, name) in [
            (&local, Role::Owner, "Local"),
            (&remote, Role::Member, "Remote"),
        ] {
            state
                .database
                .insert_member(&MemberRecord {
                    group_id: group_id.clone(),
                    peer_id: keypair.public().to_peer_id().to_string(),
                    display_name: name.into(),
                    role,
                    public_key: keypair.public().encode_protobuf(),
                    x25519_public_key: vec![7; X25519_KEY_BYTES],
                    status: "active".into(),
                    joined_at: 1,
                    timeout_until: None,
                    updated_at: 1,
                })
                .expect("member");
        }
        store_group_key_epoch(&group_id, 1, &group_key).expect("key");
        state
            .database
            .insert_key_epoch(&group_id, 1, &local_peer_id, 1, &hash_key(&group_key))
            .expect("epoch");
        state.call_states.lock().expect("call state lock").insert(
            format!("{group_id}:{channel_id}:{call_id}"),
            CallState {
                group_id: group_id.clone(),
                channel_id: channel_id.clone(),
                call_id: call_id.clone(),
                participants: vec![CallParticipant {
                    peer_id: local_peer_id.clone(),
                    display_name: "Local".into(),
                    role: Role::Owner,
                    muted: false,
                    sharing_screen: false,
                    connection_state: "connected".into(),
                }],
            },
        );
        let mut request = CallSignal {
            event_id: "call-handshake-request".into(),
            group_id: group_id.clone(),
            channel_id: channel_id.clone(),
            call_id: call_id.clone(),
            from_peer_id: remote_peer_id.clone(),
            from_public_key: remote.public().encode_protobuf(),
            key_epoch: 1,
            created_at: now_millis(),
            nonce: Vec::new(),
            ciphertext: Vec::new(),
            signature: Vec::new(),
        };
        let body = serde_json::to_string(&CallSignalBody {
            to_peer_id: None,
            kind: "join".into(),
            payload: serde_json::json!({ "display_name": "Remote" }),
        })
        .expect("body");
        let (nonce, ciphertext) = encrypt_message(
            &group_key,
            &body,
            &protocol::call_signal_aad(&request).expect("call aad"),
        )
        .expect("signal encryption");
        request.nonce = nonce;
        request.ciphertext = ciphertext;
        request.signature = remote
            .sign(&protocol::call_signal_signing_bytes(&request).expect("call signing bytes"))
            .expect("signal signature");

        let response = call_signal_join_response(&state, &request).expect("join response");
        assert_eq!(response.from_peer_id, local_peer_id);
        assert_eq!(response.call_id, call_id);
        let response_body = decode_call_signal_body(&response).expect("response body");
        assert_eq!(response_body.kind, "join");
        assert_eq!(
            response_body.to_peer_id.as_deref(),
            Some(remote_peer_id.as_str())
        );

        state.call_states.lock().expect("call state lock").clear();
        assert!(call_signal_join_response(&state, &request).is_none());
    }

    #[test]
    fn key_rotation_advances_only_one_epoch_and_validates_payload() {
        let directory = tempfile::tempdir().expect("tempdir");
        let state = NodeState::open(directory.path().to_path_buf()).expect("state");
        let keypair = Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id().to_string();
        let group_id = "rotation-group";
        state
            .database
            .insert_group(&GroupRecord {
                id: group_id.into(),
                name: "Rotation".into(),
                initials: "RO".into(),
                color: "#8276ff".into(),
                owner_peer_id: peer_id.clone(),
                created_at: 1,
                current_key_epoch: 1,
            })
            .expect("group");
        let valid = ControlEvent {
            event_id: "rotation-2".into(),
            group_id: group_id.into(),
            key_epoch: 2,
            issuer_peer_id: peer_id.clone(),
            issuer_public_key: keypair.public().encode_protobuf(),
            kind: CONTROL_KEY_ROTATED.into(),
            target_peer_id: None,
            payload: serde_json::json!({ "epoch": 2 }),
            logical_timestamp: 2,
            signature: Vec::new(),
        };
        apply_control_event(&state, &valid).expect("valid rotation");
        assert_eq!(
            current_group_epoch(&state.database, group_id).expect("epoch"),
            2
        );

        let invalid = ControlEvent {
            event_id: "rotation-4".into(),
            group_id: group_id.into(),
            key_epoch: 4,
            issuer_peer_id: peer_id,
            issuer_public_key: keypair.public().encode_protobuf(),
            kind: CONTROL_KEY_ROTATED.into(),
            target_peer_id: None,
            payload: serde_json::json!({ "epoch": 4 }),
            logical_timestamp: 3,
            signature: Vec::new(),
        };
        assert!(apply_control_event(&state, &invalid).is_err());
    }

    #[test]
    fn two_nodes_exchange_direct_contact_envelope_over_local_transport() {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime.block_on(async {
            let owner_key = Keypair::generate_ed25519();
            let member_key = Keypair::generate_ed25519();
            let owner_id = owner_key.public().to_peer_id();
            let member_id = member_key.public().to_peer_id();
            let mut owner = integration_swarm(owner_key);
            let mut member = integration_swarm(member_key);
            owner
                .listen_on("/ip4/127.0.0.1/tcp/0".parse().expect("owner listen"))
                .expect("owner listen");
            let owner_address = loop {
                if let SwarmEvent::NewListenAddr { address, .. } = owner.select_next_some().await {
                    break address;
                }
            };
            member
                .dial(owner_address.with(Protocol::P2p(owner_id)))
                .expect("dial owner");
            let mut received = false;
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    tokio::select! {
                        event = owner.select_next_some() => {
                            if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event {
                                let envelope = DirectEnvelope {
                                    event_id: "direct-test".into(),
                                    kind: "friend_request".into(),
                                    from_peer_id: owner_id.to_string(),
                                    from_public_key: vec![1],
                                    from_x25519_public_key: vec![2; 32],
                                    to_peer_id: member_id.to_string(),
                                    created_at: 1,
                                    nonce: vec![3; 24],
                                    ciphertext: vec![4],
                                    signature: vec![5],
                                };
                                owner.behaviour_mut().direct.send_request(&peer_id, envelope);
                            }
                        }
                        event = member.select_next_some() => {
                            if let SwarmEvent::Behaviour(IntegrationBehaviourEvent::Direct(request_response::Event::Message { message: request_response::Message::Request { request, channel, .. }, .. })) = event {
                                assert_eq!(request.to_peer_id, member_id.to_string());
                                member.behaviour_mut().direct.send_response(channel, request).expect("member response");
                                received = true;
                            }
                        }
                    }
                    if received { break; }
                }
            }).await.expect("direct transport timeout");
            assert!(received);
        });
    }
}
