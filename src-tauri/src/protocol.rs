use serde::{Deserialize, Serialize};

use crate::access::ControlEvent;

pub const SYNC_PROTOCOL: &str = "/teamscord/sync/1";
pub const CALL_SIGNAL_PROTOCOL: &str = "/teamscord/call-signal/1";
pub const PRESENCE_PROTOCOL: &str = "/teamscord/presence/1";
pub const DIRECT_PROTOCOL: &str = "/teamscord/direct/1";

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceCall {
    pub group_id: String,
    pub channel_id: String,
    pub call_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PresenceAnnouncement {
    pub event_id: String,
    pub peer_id: String,
    pub public_key: Vec<u8>,
    pub state: String,
    pub active_calls: Vec<PresenceCall>,
    pub created_at: i64,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContactCard {
    pub version: u8,
    pub peer_id: String,
    pub display_name: String,
    pub public_key: Vec<u8>,
    pub x25519_public_key: Vec<u8>,
    pub addresses: Vec<String>,
    pub created_at: i64,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectEnvelope {
    pub event_id: String,
    pub kind: String,
    pub from_peer_id: String,
    pub from_public_key: Vec<u8>,
    pub from_x25519_public_key: Vec<u8>,
    pub to_peer_id: String,
    pub created_at: i64,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectBody {
    pub request_id: String,
    pub conversation_id: String,
    pub display_name: Option<String>,
    pub contact_card: Option<ContactCard>,
    pub content: Option<String>,
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

pub fn presence_signing_bytes(announcement: &PresenceAnnouncement) -> Result<Vec<u8>, String> {
    let mut unsigned = announcement.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned).map_err(|error| format!("presença inválida: {error}"))
}

pub fn contact_card_signing_bytes(card: &ContactCard) -> Result<Vec<u8>, String> {
    let mut unsigned = card.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned).map_err(|error| format!("cartão de contato inválido: {error}"))
}

pub fn direct_signing_bytes(envelope: &DirectEnvelope) -> Result<Vec<u8>, String> {
    let mut unsigned = envelope.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned).map_err(|error| format!("envelope direto inválido: {error}"))
}

pub fn direct_aad(envelope: &DirectEnvelope) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&(
        DIRECT_PROTOCOL,
        &envelope.event_id,
        &envelope.kind,
        &envelope.from_peer_id,
        &envelope.to_peer_id,
        envelope.created_at,
    ))
    .map_err(|error| format!("AAD direto inválido: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    #[test]
    fn contact_card_signature_detects_identity_and_payload_changes() {
        let keypair = Keypair::generate_ed25519();
        let mut card = ContactCard {
            version: 1,
            peer_id: keypair.public().to_peer_id().to_string(),
            display_name: "Alice".into(),
            public_key: keypair.public().encode_protobuf(),
            x25519_public_key: vec![7; 32],
            addresses: vec!["/ip4/127.0.0.1/tcp/4000".into()],
            created_at: 1,
            signature: Vec::new(),
        };
        card.signature = keypair
            .sign(&contact_card_signing_bytes(&card).unwrap())
            .unwrap();
        assert!(keypair
            .public()
            .verify(&contact_card_signing_bytes(&card).unwrap(), &card.signature));
        card.display_name = "Mallory".into();
        assert!(!keypair
            .public()
            .verify(&contact_card_signing_bytes(&card).unwrap(), &card.signature));
    }

    #[test]
    fn direct_aad_changes_when_recipient_changes() {
        let mut envelope = DirectEnvelope {
            event_id: "event".into(),
            kind: "direct_message".into(),
            from_peer_id: "from".into(),
            from_public_key: vec![1],
            from_x25519_public_key: vec![2; 32],
            to_peer_id: "to".into(),
            created_at: 10,
            nonce: vec![3; 24],
            ciphertext: vec![4],
            signature: Vec::new(),
        };
        let first = direct_aad(&envelope).unwrap();
        envelope.to_peer_id = "other".into();
        assert_ne!(first, direct_aad(&envelope).unwrap());
    }
}
