use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use getrandom::fill as fill_random;
use libp2p::{
    identity::{Keypair, PublicKey},
    PeerId,
};
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub const GROUP_KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 24;
pub const X25519_KEY_BYTES: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvitePayload {
    pub version: u8,
    pub group_id: String,
    pub group_name: String,
    pub initials: String,
    pub color: String,
    pub owner_peer_id: String,
    pub owner_public_key: String,
    #[serde(default)]
    pub owner_x25519_public_key: Option<String>,
    pub group_key: String,
    #[serde(default = "default_key_epoch")]
    pub key_epoch: i64,
    pub expires_at: i64,
    pub signature: String,
    #[serde(default)]
    pub issuer_peer_id: Option<String>,
    #[serde(default)]
    pub issuer_public_key: Option<String>,
    #[serde(default)]
    pub issuer_signature: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UnsignedInvite<'a> {
    version: u8,
    group_id: &'a str,
    group_name: &'a str,
    initials: &'a str,
    color: &'a str,
    owner_peer_id: &'a str,
    owner_public_key: &'a str,
    owner_x25519_public_key: &'a str,
    group_key: &'a str,
    key_epoch: i64,
    expires_at: i64,
}

#[derive(Clone, Debug, Serialize)]
struct UnsignedIssuedInvite<'a> {
    version: u8,
    group_id: &'a str,
    group_name: &'a str,
    initials: &'a str,
    color: &'a str,
    owner_peer_id: &'a str,
    owner_public_key: &'a str,
    owner_x25519_public_key: &'a str,
    group_key: &'a str,
    key_epoch: i64,
    expires_at: i64,
    issuer_peer_id: &'a str,
    issuer_public_key: &'a str,
}

#[derive(Clone, Debug, Serialize)]
struct LegacyUnsignedInvite<'a> {
    version: u8,
    group_id: &'a str,
    group_name: &'a str,
    initials: &'a str,
    color: &'a str,
    owner_peer_id: &'a str,
    owner_public_key: &'a str,
    group_key: &'a str,
    expires_at: i64,
}

#[derive(Clone, Debug, Serialize)]
struct LegacyUnsignedIssuedInvite<'a> {
    version: u8,
    group_id: &'a str,
    group_name: &'a str,
    initials: &'a str,
    color: &'a str,
    owner_peer_id: &'a str,
    owner_public_key: &'a str,
    group_key: &'a str,
    expires_at: i64,
    issuer_peer_id: &'a str,
    issuer_public_key: &'a str,
}

fn default_key_epoch() -> i64 {
    1
}

pub fn new_group_key() -> Result<[u8; GROUP_KEY_BYTES], String> {
    let mut key = [0u8; GROUP_KEY_BYTES];
    fill_random(&mut key)
        .map_err(|error| format!("não foi possível gerar chave do grupo: {error}"))?;
    Ok(key)
}

pub fn encrypt_message(
    key: &[u8; GROUP_KEY_BYTES],
    plaintext: &str,
    aad: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|error| format!("chave de grupo inválida: {error}"))?;
    let mut nonce = [0u8; NONCE_BYTES];
    fill_random(&mut nonce).map_err(|error| format!("não foi possível gerar nonce: {error}"))?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad,
            },
        )
        .map_err(|_| "não foi possível cifrar mensagem".to_string())?;
    Ok((nonce.to_vec(), ciphertext))
}

pub fn decrypt_message(
    key: &[u8; GROUP_KEY_BYTES],
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<String, String> {
    if nonce.len() != NONCE_BYTES {
        return Err("nonce inválido".into());
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|error| format!("chave de grupo inválida: {error}"))?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| "mensagem não pôde ser autenticada".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "mensagem não está em UTF-8".into())
}

pub fn encode_invite(payload: &InvitePayload) -> Result<String, String> {
    let bytes =
        serde_json::to_vec(payload).map_err(|error| format!("convite inválido: {error}"))?;
    Ok(BASE64.encode(bytes))
}

pub fn decode_invite(invite: &str) -> Result<InvitePayload, String> {
    let bytes = BASE64
        .decode(invite.trim())
        .map_err(|error| format!("convite não é base64 válido: {error}"))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("convite inválido: {error}"))
}

#[allow(clippy::too_many_arguments)]
pub fn create_invite(
    keypair: &Keypair,
    group_id: &str,
    group_name: &str,
    initials: &str,
    color: &str,
    group_key: &[u8; GROUP_KEY_BYTES],
    key_epoch: i64,
    owner_x25519_public_key: &str,
    expires_at: i64,
) -> Result<String, String> {
    let owner_public_key = BASE64.encode(keypair.public().encode_protobuf());
    let owner_peer_id = keypair.public().to_peer_id().to_string();
    let group_key_encoded = BASE64.encode(group_key);
    let unsigned = UnsignedInvite {
        version: 5,
        group_id,
        group_name,
        initials,
        color,
        owner_peer_id: &owner_peer_id,
        owner_public_key: &owner_public_key,
        owner_x25519_public_key,
        group_key: &group_key_encoded,
        key_epoch,
        expires_at,
    };
    let signing_bytes = serde_json::to_vec(&unsigned)
        .map_err(|error| format!("não foi possível assinar convite: {error}"))?;
    let signature = keypair
        .sign(&signing_bytes)
        .map_err(|error| format!("não foi possível assinar convite: {error}"))?;
    encode_invite(&InvitePayload {
        version: unsigned.version,
        group_id: group_id.into(),
        group_name: group_name.into(),
        initials: initials.into(),
        color: color.into(),
        owner_peer_id,
        owner_public_key,
        owner_x25519_public_key: Some(owner_x25519_public_key.into()),
        group_key: group_key_encoded,
        key_epoch,
        expires_at,
        signature: BASE64.encode(signature),
        issuer_peer_id: None,
        issuer_public_key: None,
        issuer_signature: None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn create_delegated_invite(
    issuer_keypair: &Keypair,
    owner_peer_id: &str,
    owner_public_key: &str,
    group_id: &str,
    group_name: &str,
    initials: &str,
    color: &str,
    group_key: &[u8; GROUP_KEY_BYTES],
    key_epoch: i64,
    owner_x25519_public_key: &str,
    expires_at: i64,
) -> Result<String, String> {
    let issuer_public_key = BASE64.encode(issuer_keypair.public().encode_protobuf());
    let issuer_peer_id = issuer_keypair.public().to_peer_id().to_string();
    let group_key_encoded = BASE64.encode(group_key);
    let unsigned = UnsignedIssuedInvite {
        version: 6,
        group_id,
        group_name,
        initials,
        color,
        owner_peer_id,
        owner_public_key,
        owner_x25519_public_key,
        group_key: &group_key_encoded,
        key_epoch,
        expires_at,
        issuer_peer_id: &issuer_peer_id,
        issuer_public_key: &issuer_public_key,
    };
    let signing_bytes = serde_json::to_vec(&unsigned)
        .map_err(|error| format!("não foi possível assinar convite delegado: {error}"))?;
    let signature = issuer_keypair
        .sign(&signing_bytes)
        .map_err(|error| format!("não foi possível assinar convite delegado: {error}"))?;
    encode_invite(&InvitePayload {
        version: unsigned.version,
        group_id: group_id.into(),
        group_name: group_name.into(),
        initials: initials.into(),
        color: color.into(),
        owner_peer_id: owner_peer_id.into(),
        owner_public_key: owner_public_key.into(),
        owner_x25519_public_key: Some(owner_x25519_public_key.into()),
        group_key: group_key_encoded,
        key_epoch,
        expires_at,
        signature: String::new(),
        issuer_peer_id: Some(issuer_peer_id),
        issuer_public_key: Some(issuer_public_key),
        issuer_signature: Some(BASE64.encode(signature)),
    })
}

pub fn validate_invite(invite: &InvitePayload, now: i64) -> Result<[u8; GROUP_KEY_BYTES], String> {
    if !matches!(invite.version, 1..=6) {
        return Err("versão de convite não suportada".into());
    }
    if invite.expires_at < now {
        return Err("convite expirado".into());
    }
    let public_key_bytes = BASE64
        .decode(&invite.owner_public_key)
        .map_err(|error| format!("chave pública inválida: {error}"))?;
    let public_key = PublicKey::try_decode_protobuf(&public_key_bytes)
        .map_err(|error| format!("chave pública inválida: {error}"))?;
    if public_key.to_peer_id().to_string() != invite.owner_peer_id {
        return Err("peer id do convite não corresponde à chave pública".into());
    }
    if invite.version == 2 || invite.version == 4 || invite.version == 6 {
        let issuer_peer_id = invite
            .issuer_peer_id
            .as_deref()
            .ok_or("emissor do convite ausente")?;
        let issuer_public_key_encoded = invite
            .issuer_public_key
            .as_deref()
            .ok_or("chave do emissor ausente")?;
        let issuer_signature = invite
            .issuer_signature
            .as_deref()
            .ok_or("assinatura do emissor ausente")?;
        let issuer_public_key_bytes = BASE64
            .decode(issuer_public_key_encoded)
            .map_err(|error| format!("chave do emissor inválida: {error}"))?;
        let issuer_public_key = PublicKey::try_decode_protobuf(&issuer_public_key_bytes)
            .map_err(|error| format!("chave do emissor inválida: {error}"))?;
        if issuer_public_key.to_peer_id().to_string() != issuer_peer_id {
            return Err("peer id do emissor não corresponde à chave pública".into());
        }
        let signing_bytes = if invite.version == 6 {
            serde_json::to_vec(&UnsignedIssuedInvite {
                version: invite.version,
                group_id: &invite.group_id,
                group_name: &invite.group_name,
                initials: &invite.initials,
                color: &invite.color,
                owner_peer_id: &invite.owner_peer_id,
                owner_public_key: &invite.owner_public_key,
                owner_x25519_public_key: invite
                    .owner_x25519_public_key
                    .as_deref()
                    .ok_or("chave de acordo do Owner ausente")?,
                group_key: &invite.group_key,
                key_epoch: invite.key_epoch,
                expires_at: invite.expires_at,
                issuer_peer_id,
                issuer_public_key: issuer_public_key_encoded,
            })
        } else {
            serde_json::to_vec(&LegacyUnsignedIssuedInvite {
                version: invite.version,
                group_id: &invite.group_id,
                group_name: &invite.group_name,
                initials: &invite.initials,
                color: &invite.color,
                owner_peer_id: &invite.owner_peer_id,
                owner_public_key: &invite.owner_public_key,
                group_key: &invite.group_key,
                expires_at: invite.expires_at,
                issuer_peer_id,
                issuer_public_key: issuer_public_key_encoded,
            })
        }
        .map_err(|error| format!("convite inválido: {error}"))?;
        let signature = BASE64
            .decode(issuer_signature)
            .map_err(|error| format!("assinatura do emissor inválida: {error}"))?;
        if !issuer_public_key.verify(&signing_bytes, &signature) {
            return Err("assinatura do emissor inválida".into());
        }
    } else {
        let signing_bytes = if invite.version == 5 {
            serde_json::to_vec(&UnsignedInvite {
                version: invite.version,
                group_id: &invite.group_id,
                group_name: &invite.group_name,
                initials: &invite.initials,
                color: &invite.color,
                owner_peer_id: &invite.owner_peer_id,
                owner_public_key: &invite.owner_public_key,
                owner_x25519_public_key: invite
                    .owner_x25519_public_key
                    .as_deref()
                    .ok_or("chave de acordo do Owner ausente")?,
                group_key: &invite.group_key,
                key_epoch: invite.key_epoch,
                expires_at: invite.expires_at,
            })
        } else {
            serde_json::to_vec(&LegacyUnsignedInvite {
                version: invite.version,
                group_id: &invite.group_id,
                group_name: &invite.group_name,
                initials: &invite.initials,
                color: &invite.color,
                owner_peer_id: &invite.owner_peer_id,
                owner_public_key: &invite.owner_public_key,
                group_key: &invite.group_key,
                expires_at: invite.expires_at,
            })
        }
        .map_err(|error| format!("convite inválido: {error}"))?;
        let signature = BASE64
            .decode(&invite.signature)
            .map_err(|error| format!("assinatura inválida: {error}"))?;
        if !public_key.verify(&signing_bytes, &signature) {
            return Err("assinatura do convite inválida".into());
        }
    }
    let decoded_key = BASE64
        .decode(&invite.group_key)
        .map_err(|error| format!("chave do grupo inválida: {error}"))?;
    decoded_key
        .try_into()
        .map_err(|_| "chave do grupo inválida".to_string())
}

pub fn verify_author(
    public_key_bytes: &[u8],
    author_peer_id: &str,
    signing_bytes: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let public_key = PublicKey::try_decode_protobuf(public_key_bytes)
        .map_err(|error| format!("chave de autor inválida: {error}"))?;
    let peer_id = public_key.to_peer_id();
    let expected = author_peer_id
        .parse::<PeerId>()
        .map_err(|error| format!("peer id inválido: {error}"))?;
    if peer_id != expected || !public_key.verify(signing_bytes, signature) {
        return Err("assinatura da mensagem inválida".into());
    }
    Ok(())
}

pub fn new_x25519_keypair() -> Result<([u8; X25519_KEY_BYTES], [u8; X25519_KEY_BYTES]), String> {
    let mut secret_bytes = [0u8; X25519_KEY_BYTES];
    fill_random(&mut secret_bytes)
        .map_err(|error| format!("não foi possível gerar chave de acordo: {error}"))?;
    let secret = StaticSecret::from(secret_bytes);
    let public = X25519PublicKey::from(&secret);
    Ok((secret_bytes, public.to_bytes()))
}

pub fn encrypt_for_recipient(
    secret_bytes: &[u8; X25519_KEY_BYTES],
    recipient_public_bytes: &[u8],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    let recipient_public: [u8; X25519_KEY_BYTES] = recipient_public_bytes
        .try_into()
        .map_err(|_| "chave pública de acordo inválida".to_string())?;
    let shared =
        StaticSecret::from(*secret_bytes).diffie_hellman(&X25519PublicKey::from(recipient_public));
    let cipher = XChaCha20Poly1305::new_from_slice(shared.as_bytes())
        .map_err(|error| format!("chave de acordo inválida: {error}"))?;
    let mut nonce = [0u8; NONCE_BYTES];
    fill_random(&mut nonce).map_err(|error| format!("não foi possível gerar nonce: {error}"))?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| "não foi possível proteger compartilhamento de chave".to_string())?;
    Ok((nonce.to_vec(), ciphertext))
}

pub fn decrypt_from_sender(
    secret_bytes: &[u8; X25519_KEY_BYTES],
    sender_public_bytes: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, String> {
    if nonce.len() != NONCE_BYTES {
        return Err("nonce de compartilhamento inválido".into());
    }
    let sender_public: [u8; X25519_KEY_BYTES] = sender_public_bytes
        .try_into()
        .map_err(|_| "chave pública do remetente inválida".to_string())?;
    let shared =
        StaticSecret::from(*secret_bytes).diffie_hellman(&X25519PublicKey::from(sender_public));
    let cipher = XChaCha20Poly1305::new_from_slice(shared.as_bytes())
        .map_err(|error| format!("chave de acordo inválida: {error}"))?;
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| "compartilhamento de chave não pôde ser autenticado".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encryption_round_trip_and_tamper_rejection() {
        let key = new_group_key().expect("key");
        let (nonce, ciphertext) = encrypt_message(&key, "olá rede", b"event-1").expect("encrypt");
        assert_eq!(
            decrypt_message(&key, &nonce, &ciphertext, b"event-1").unwrap(),
            "olá rede"
        );
        assert!(decrypt_message(&key, &nonce, &ciphertext, b"event-2").is_err());
    }

    #[test]
    fn invite_is_signed_and_expiration_is_checked() {
        let keypair = Keypair::generate_ed25519();
        let group_key = new_group_key().expect("key");
        let owner_x25519 = BASE64.encode([7u8; X25519_KEY_BYTES]);
        let invite = create_invite(
            &keypair,
            "group-1",
            "Amigos",
            "AM",
            "#8276ff",
            &group_key,
            2,
            &owner_x25519,
            100,
        )
        .expect("invite");
        let decoded = decode_invite(&invite).expect("decode");
        assert_eq!(decoded.key_epoch, 2);
        assert_eq!(
            decoded.owner_x25519_public_key.as_deref(),
            Some(owner_x25519.as_str())
        );
        assert_eq!(validate_invite(&decoded, 99).unwrap(), group_key);
        assert!(validate_invite(&decoded, 101).is_err());
    }

    #[test]
    fn agreement_key_exchange_authenticates_and_rejects_tampering() {
        let (alice_secret, alice_public) = new_x25519_keypair().expect("alice");
        let (bob_secret, bob_public) = new_x25519_keypair().expect("bob");
        let (nonce, ciphertext) =
            encrypt_for_recipient(&alice_secret, &bob_public, b"epoch-2-key", b"group-1:2:bob")
                .expect("encrypt");
        assert_eq!(
            decrypt_from_sender(
                &bob_secret,
                &alice_public,
                &nonce,
                &ciphertext,
                b"group-1:2:bob",
            )
            .expect("decrypt"),
            b"epoch-2-key"
        );
        assert!(decrypt_from_sender(
            &bob_secret,
            &alice_public,
            &nonce,
            &ciphertext,
            b"tampered-aad"
        )
        .is_err());
    }

    #[test]
    fn delegated_invite_keeps_owner_and_validates_issuer_signature() {
        let owner = Keypair::generate_ed25519();
        let admin = Keypair::generate_ed25519();
        let group_key = new_group_key().expect("key");
        let owner_public_key = BASE64.encode(owner.public().encode_protobuf());
        let owner_x25519 = BASE64.encode([8u8; X25519_KEY_BYTES]);
        let invite = create_delegated_invite(
            &admin,
            &owner.public().to_peer_id().to_string(),
            &owner_public_key,
            "group-1",
            "Amigos",
            "AM",
            "#8276ff",
            &group_key,
            2,
            &owner_x25519,
            100,
        )
        .expect("invite");
        let decoded = decode_invite(&invite).expect("decode");
        assert_eq!(
            decoded.owner_peer_id,
            owner.public().to_peer_id().to_string()
        );
        assert_eq!(validate_invite(&decoded, 99).unwrap(), group_key);
    }
}
