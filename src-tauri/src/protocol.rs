use serde::{Deserialize, Serialize};

use crate::access::ControlEvent;

pub const SYNC_PROTOCOL: &str = "/teamscord/sync/1";
pub const CALL_SIGNAL_PROTOCOL: &str = "/teamscord/call-signal/1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub event_id: String,
    pub group_id: String,
    pub channel_id: String,
    pub author_peer_id: String,
    pub author_public_key: Vec<u8>,
    pub author_name: String,
    pub initials: String,
    pub color: String,
    pub created_at: i64,
    pub key_epoch: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    pub group_id: String,
    pub channel_id: Option<String>,
    pub after_created_at: i64,
    pub after_event_id: String,
    pub after_control_timestamp: i64,
    pub after_control_event_id: String,
    pub limit: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    pub messages: Vec<MessageEnvelope>,
    pub controls: Vec<ControlEvent>,
    pub next: Option<SyncRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallSignal {
    pub event_id: String,
    pub group_id: String,
    pub channel_id: String,
    pub call_id: String,
    pub from_peer_id: String,
    pub from_public_key: Vec<u8>,
    pub key_epoch: i64,
    pub created_at: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallSignalBody {
    pub to_peer_id: Option<String>,
    pub kind: String,
    pub payload: serde_json::Value,
}

pub fn signing_bytes(envelope: &MessageEnvelope) -> Result<Vec<u8>, String> {
    let mut unsigned = envelope.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned).map_err(|error| format!("envelope inválido: {error}"))
}

pub fn aad_for_envelope(envelope: &MessageEnvelope) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        "teamscord-message-v1",
        &envelope.event_id,
        &envelope.group_id,
        &envelope.channel_id,
        &envelope.author_peer_id,
        envelope.created_at,
        envelope.key_epoch,
    ))
    .map_err(|error| format!("AAD inválido: {error}"))
}

pub fn call_signal_signing_bytes(signal: &CallSignal) -> Result<Vec<u8>, String> {
    let mut unsigned = signal.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned).map_err(|error| format!("sinal de call inválido: {error}"))
}

pub fn call_signal_aad(signal: &CallSignal) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        CALL_SIGNAL_PROTOCOL,
        &signal.event_id,
        &signal.group_id,
        &signal.channel_id,
        &signal.call_id,
        &signal.from_peer_id,
        signal.key_epoch,
        signal.created_at,
    ))
    .map_err(|error| format!("AAD de call inválido: {error}"))
}
