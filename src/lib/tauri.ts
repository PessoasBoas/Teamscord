import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";

/** Channels exposed by the current UI. `screen` is migrated by Rust and is not creatable. */
export type ChannelKind = "text" | "voice";
export type LegacyChannelKind = ChannelKind | "screen";
export type Role = "owner" | "admin" | "mod" | "member";
export type Permission = "view_channels" | "send_messages" | "join_voice" | "share_screen" | "manage_invites" | "manage_channels" | "manage_members" | "manage_roles" | "moderate_messages" | "manage_group";

export type Channel = {
  id: string;
  group_id: string;
  name: string;
  kind: ChannelKind;
  position: number;
};

export type Group = {
  id: string;
  name: string;
  initials: string;
  color: string;
  owner_peer_id: string;
  created_at: number;
  channels: Channel[];
};

export type ChatMessage = {
  id: string;
  group_id: string;
  channel_id: string;
  author_peer_id: string;
  author: string;
  initials: string;
  color: string;
  content: string;
  timestamp: string;
  created_at: number;
  mine: boolean;
};

export type ContactCard = {
  peer_id: string;
  display_name: string;
  public_key: number[];
  x25519_public_key: number[];
  addresses: string[];
  encoded: string;
};
export type Friend = { peer_id: string; display_name: string; status: "pending" | "accepted" | "rejected" | "blocked"; requested_by: string; online: boolean; updated_at: number };
export type DirectMessage = { id: string; conversation_id: string; from_peer_id: string; to_peer_id: string; author: string; content: string; created_at: number; mine: boolean };

export type NodeSnapshot = {
  peer_id: string;
  listen_addresses: string[];
  connected_peers: number;
  is_running: boolean;
  relay_addresses: string[];
  bootstrap_addresses: string[];
  relay_connected: boolean;
  last_sync_at?: number | null;
};
export type NetworkDiagnosticCheck = { name: string; status: "ok" | "error" | "waiting" | "info"; detail: string };
export type NetworkDiagnostics = { status: "healthy" | "waiting" | "offline"; summary: string; checks: NetworkDiagnosticCheck[]; connected_peers: number; checked_at: number };

export type NetworkStatus = "starting" | "online" | "offline" | "syncing" | "preview" | "reconnecting";
export type SyncStatus = { group_id: string; state: "idle" | "syncing" | "synced" | "waiting" | "error"; updated_at: number; error?: string | null };
export type UserPreferences = {
  theme: "dark" | "light" | "system";
  font: "manrope" | "system" | "mono";
  scale: "compact" | "comfortable" | "large";
  display_name: string;
  accent_color?: string;
  avatar_color?: string;
};
export type SearchResult = { kind: "message" | "channel" | "member"; id: string; title: string; subtitle: string; group_id: string; channel_id?: string; created_at?: number };
export type AppNotification = { id: string; kind: "message" | "member" | "call" | "system"; title: string; body: string; created_at: number; read: boolean; group_id?: string; channel_id?: string };

export type NetworkConfig = {
  relay_addresses: string[];
  bootstrap_addresses: string[];
};

export type NodeContact = { peer_id: string; addresses: string[]; source: string; last_seen: number };
export type RelayStatus = { address: string; peer_id?: string | null; state: "connected" | "connecting" | "offline"; last_seen?: number | null };
export type PeerPresence = { peer_id: string; state: string; last_seen: number; source: string; active_calls?: PresenceCall[] };
export type ConnectionDiagnostic = { peer_id: string; state: string; detail: string; checked_at: number };
export type NetworkStatusView = { snapshot: NodeSnapshot; relays: RelayStatus[]; peers: NodeContact[] };
export type PresenceCall = { group_id: string; channel_id: string; call_id: string; display_name: string };

export type GroupCreateResult = {
  group: Group;
  invite: string;
};

export type GroupMember = {
  group_id: string;
  peer_id: string;
  display_name: string;
  role: Role;
  public_key: number[];
  x25519_public_key: number[];
  status: string;
  joined_at: number;
  timeout_until?: number | null;
  updated_at: number;
};

export type RoleView = { role: Role; permissions: Permission[] };
export type ChannelPermission = { group_id: string; channel_id: string; role: Role; can_join_voice: boolean; can_share_screen: boolean };
export type AuditEvent = {
  event_id: string;
  group_id: string;
  key_epoch: number;
  issuer_peer_id: string;
  kind: string;
  target_peer_id?: string | null;
  payload: Record<string, unknown>;
  logical_timestamp: number;
  signature: number[];
};

export type IceServerConfig = { urls: string[]; username?: string | null; credential?: string | null };
export type MediaConfig = { ice_servers: IceServerConfig[] };
export type CallParticipant = { peer_id: string; display_name: string; role: Role; muted: boolean; sharing_screen: boolean; connection_state: string };
export type CallState = { group_id: string; channel_id: string; call_id: string; participants: CallParticipant[] };
export type CallSignal = { event_id: string; group_id: string; channel_id: string; call_id: string; from_peer_id: string; key_epoch: number; created_at: number; nonce: number[]; ciphertext: number[]; signature: number[] };
export type CallSignalEvent = { signal: Record<string, unknown>; body: { to_peer_id?: string | null; kind: string; payload: Record<string, unknown> } };

export type NodeEvent = {
  kind: "ready" | "snapshot" | "message" | "direct-message" | "friend-updated" | "error" | "group-control" | "member-updated" | "channel-updated" | "call-signal" | "call-state" | "media-error" | "key-epoch-changed" | "sync-state" | "peer-updated" | "relay-state" | "peer-presence" | "sync-progress" | "connection-diagnostic";
  message?: ChatMessage;
  snapshot?: NodeSnapshot;
  error?: string;
  data?: Record<string, unknown> | CallSignalEvent | CallState | SyncStatus;
};

export const isDesktop = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const nodeApi = {
  startNode: () => invoke<NodeSnapshot>("start_node"),
  getNodeSnapshot: () => invoke<NodeSnapshot>("get_node_snapshot"),
  getNetworkStatus: () => invoke<NetworkStatusView>("get_network_status"),
  getKnownPeers: () => invoke<NodeContact[]>("get_known_peers"),
  getContactCard: (displayName = "Você") => invoke<ContactCard>("get_contact_card", { displayName }),
  listFriends: () => invoke<Friend[]>("list_friends"),
  createFriendRequest: (contact: string, displayName = "Você") => invoke<Friend>("create_friend_request", { contact, displayName }),
  respondFriendRequest: (peerId: string, response: "accept" | "reject" | "block", displayName = "Você") => invoke<Friend>("respond_friend_request", { peerId, response, displayName }),
  getDirectMessages: (peerId: string) => invoke<DirectMessage[]>("get_direct_messages", { peerId }),
  sendDirectMessage: (peerId: string, content: string, displayName = "Você") => invoke<DirectMessage>("send_direct_message", { peerId, content, displayName }),
  runNetworkDiagnostics: () => invoke<NetworkDiagnostics>("run_network_diagnostics"),
  getNetworkConfig: () => invoke<NetworkConfig>("get_network_config"),
  setNetworkConfig: (relayAddresses: string[], bootstrapAddresses: string[] = []) => invoke<NetworkConfig>("set_network_config", { relayAddresses, bootstrapAddresses }),
  getMediaConfig: () => invoke<MediaConfig>("get_media_config"),
  setMediaConfig: (iceServers: IceServerConfig[]) => invoke<MediaConfig>("set_media_config", { iceServers }),
  getGroups: () => invoke<Group[]>("get_groups"),
  createGroup: (name: string) => invoke<GroupCreateResult>("create_group", { name }),
  createInvite: (groupId: string) => invoke<string>("create_invite", { groupId }),
  joinGroup: (invite: string) => invoke<Group>("join_group", { invite }),
  leaveGroup: (groupId: string) => invoke<void>("leave_group", { groupId }),
  listGroupMembers: (groupId: string) => invoke<GroupMember[]>("list_group_members", { groupId }),
  getGroupRoles: () => invoke<RoleView[]>("get_group_roles"),
  getChannelPermissions: (groupId: string, channelId: string) => invoke<ChannelPermission[]>("get_channel_permissions", { groupId, channelId }),
  setChannelPermission: (groupId: string, channelId: string, role: Role, canJoinVoice: boolean, canShareScreen: boolean) => invoke<void>("set_channel_permission", { groupId, channelId, role, canJoinVoice, canShareScreen }),
  updateMemberRole: (groupId: string, peerId: string, role: Role) => invoke<void>("update_member_role", { groupId, peerId, role }),
  transferGroupOwnership: (groupId: string, peerId: string) => invoke<void>("transfer_group_ownership", { groupId, peerId }),
  deleteGroup: (groupId: string) => invoke<void>("delete_group", { groupId }),
  kickMember: (groupId: string, peerId: string) => invoke<void>("kick_member", { groupId, peerId }),
  banMember: (groupId: string, peerId: string) => invoke<void>("ban_member", { groupId, peerId }),
  unbanMember: (groupId: string, peerId: string) => invoke<void>("unban_member", { groupId, peerId }),
  timeoutMember: (groupId: string, peerId: string, durationSeconds: number) => invoke<void>("timeout_member", { groupId, peerId, durationSeconds }),
  createChannel: (groupId: string, name: string, kind: ChannelKind) => invoke<Channel>("create_channel", { groupId, name, kind }),
  updateChannel: (channel: Channel) => invoke<void>("update_channel", { channel }),
  deleteChannel: (groupId: string, channelId: string) => invoke<void>("delete_channel", { groupId, channelId }),
  reorderChannels: (groupId: string, channelIds: string[]) => invoke<void>("reorder_channels", { groupId, channelIds }),
  getAuditLog: (groupId: string) => invoke<AuditEvent[]>("get_audit_log", { groupId, limit: 200 }),
  getMessages: (groupId: string, channelId: string) => invoke<ChatMessage[]>("get_messages", { groupId, channelId, limit: 200 }),
  searchMessages: (groupId: string, query: string, limit = 50) => invoke<SearchResult[]>("search_messages", { groupId, query, limit }),
  sendMessage: (groupId: string, channelId: string, content: string, authorName = "Você") => invoke<ChatMessage>("send_message", { groupId, channelId, authorName, content }),
  deleteMessage: (groupId: string, channelId: string, messageId: string) => invoke<void>("delete_message", { groupId, channelId, messageId }),
  joinCall: (groupId: string, channelId: string, callId?: string, displayName?: string) => invoke<CallState>("join_call", { groupId, channelId, callId, displayName }),
  leaveCall: (groupId: string, channelId: string, callId: string) => invoke<void>("leave_call", { groupId, channelId, callId }),
  sendCallSignal: (groupId: string, channelId: string, callId: string, toPeerId: string | null, kind: string, payload: Record<string, unknown>) => invoke<void>("send_call_signal", { groupId, channelId, callId, toPeerId, kind, payload }),
  requestCallMute: (groupId: string, channelId: string, callId: string, peerId: string, muted: boolean) => invoke<void>("request_call_mute", { groupId, channelId, callId, peerId, muted }),
  getCallState: (groupId: string, channelId: string, callId: string) => invoke<CallState>("get_call_state", { groupId, channelId, callId }),
  connectPeer: (address: string) => invoke<void>("connect_peer", { address }),
  testPeerConnection: (address: string) => invoke<ConnectionDiagnostic>("test_peer_connection", { address }),
  openExternalUrl: (url: string) => invoke<void>("open_external_url", { url }),
  onNodeEvent: (handler: (event: NodeEvent) => void): Promise<UnlistenFn> => listen<NodeEvent>("node://event", (event) => handler(event.payload)),
};
