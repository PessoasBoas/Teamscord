use libp2p::identity::Keypair;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::crypto::verify_author;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Admin,
    Mod,
    Member,
}

impl Role {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "mod" => Ok(Self::Mod),
            "member" => Ok(Self::Member),
            _ => Err("cargo inválido".into()),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Mod => "mod",
            Self::Member => "member",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ViewChannels,
    SendMessages,
    JoinVoice,
    ShareScreen,
    ManageInvites,
    ManageChannels,
    ManageMembers,
    ManageRoles,
    ModerateMessages,
    ManageGroup,
}

pub fn permissions_for(role: &Role) -> Vec<Permission> {
    let mut permissions = vec![Permission::ViewChannels, Permission::SendMessages];
    match role {
        Role::Owner => permissions.extend([
            Permission::JoinVoice,
            Permission::ShareScreen,
            Permission::ManageInvites,
            Permission::ManageChannels,
            Permission::ManageMembers,
            Permission::ManageRoles,
            Permission::ModerateMessages,
            Permission::ManageGroup,
        ]),
        Role::Admin => permissions.extend([
            Permission::JoinVoice,
            Permission::ShareScreen,
            Permission::ManageInvites,
            Permission::ManageChannels,
            Permission::ManageMembers,
            Permission::ManageRoles,
            Permission::ModerateMessages,
        ]),
        Role::Mod => permissions.extend([
            Permission::JoinVoice,
            Permission::ShareScreen,
            Permission::ModerateMessages,
        ]),
        Role::Member => permissions.extend([Permission::JoinVoice, Permission::ShareScreen]),
    }
    permissions
}

pub fn has_permission(role: &Role, permission: &Permission) -> bool {
    permissions_for(role).contains(permission)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlEvent {
    pub event_id: String,
    pub group_id: String,
    pub key_epoch: i64,
    pub issuer_peer_id: String,
    pub issuer_public_key: Vec<u8>,
    pub kind: String,
    pub target_peer_id: Option<String>,
    pub payload: Value,
    pub logical_timestamp: i64,
    pub signature: Vec<u8>,
}

pub fn control_signing_bytes(event: &ControlEvent) -> Result<Vec<u8>, String> {
    let mut unsigned = event.clone();
    unsigned.signature.clear();
    serde_json::to_vec(&unsigned)
        .map_err(|error| format!("evento administrativo inválido: {error}"))
}

pub fn sign_control_event(event: &mut ControlEvent, keypair: &Keypair) -> Result<(), String> {
    event.signature = keypair
        .sign(&control_signing_bytes(event)?)
        .map_err(|error| format!("não foi possível assinar evento administrativo: {error}"))?;
    Ok(())
}

pub fn verify_control_event(event: &ControlEvent) -> Result<(), String> {
    verify_author(
        &event.issuer_public_key,
        &event.issuer_peer_id,
        &control_signing_bytes(event)?,
        &event.signature,
    )
}

pub const CONTROL_MEMBER_JOINED: &str = "member_joined";
pub const CONTROL_ROLE_CHANGED: &str = "role_changed";
pub const CONTROL_MEMBER_KICKED: &str = "member_kicked";
pub const CONTROL_MEMBER_BANNED: &str = "member_banned";
pub const CONTROL_MEMBER_UNBANNED: &str = "member_unbanned";
pub const CONTROL_MEMBER_TIMEOUT: &str = "member_timeout";
pub const CONTROL_MEMBER_LEFT: &str = "member_left";
pub const CONTROL_CHANNEL_CREATED: &str = "channel_created";
pub const CONTROL_CHANNEL_UPDATED: &str = "channel_updated";
pub const CONTROL_CHANNEL_DELETED: &str = "channel_deleted";
pub const CONTROL_CHANNEL_REORDERED: &str = "channel_reordered";
pub const CONTROL_CHANNEL_PERMISSION_UPDATED: &str = "channel_permission_updated";
pub const CONTROL_MESSAGE_DELETED: &str = "message_deleted";
pub const CONTROL_KEY_ROTATED: &str = "key_rotated";
pub const CONTROL_KEY_SHARE: &str = "key_share";
pub const CONTROL_GROUP_OWNER_TRANSFERRED: &str = "group_owner_transferred";
pub const CONTROL_GROUP_DELETED: &str = "group_deleted";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_roles_have_expected_authority_boundaries() {
        assert!(has_permission(&Role::Owner, &Permission::ManageRoles));
        assert!(has_permission(&Role::Admin, &Permission::ManageChannels));
        assert!(has_permission(&Role::Admin, &Permission::ManageRoles));
        assert!(has_permission(&Role::Mod, &Permission::ModerateMessages));
        assert!(!has_permission(&Role::Mod, &Permission::ManageRoles));
        assert!(!has_permission(
            &Role::Member,
            &Permission::ModerateMessages
        ));
    }

    #[test]
    fn control_event_signature_detects_payload_changes() {
        let keypair = Keypair::generate_ed25519();
        let mut event = ControlEvent {
            event_id: "event-1".into(),
            group_id: "group-1".into(),
            key_epoch: 1,
            issuer_peer_id: keypair.public().to_peer_id().to_string(),
            issuer_public_key: keypair.public().encode_protobuf(),
            kind: CONTROL_ROLE_CHANGED.into(),
            target_peer_id: Some("peer-2".into()),
            payload: serde_json::json!({ "role": "mod" }),
            logical_timestamp: 1,
            signature: Vec::new(),
        };
        sign_control_event(&mut event, &keypair).expect("sign");
        verify_control_event(&event).expect("verify");
        event.payload = serde_json::json!({ "role": "owner" });
        assert!(verify_control_event(&event).is_err());
    }
}
