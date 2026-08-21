import type { CallState, ChatMessage, Group, GroupMember } from "./tauri";

export function callKey(groupId: string, channelId: string) {
  return `${groupId}:${channelId}`;
}

export function normalizeGroup(group: Group): Group {
  return {
    ...group,
    channels: group.channels.map((channel) => ({
      ...channel,
      kind: ((channel.kind as string) === "screen" ? "voice" : channel.kind) as Group["channels"][number]["kind"],
    })),
  };
}

export function selectInitialChannel(group: Group, persistedChannelId?: string): string {
  if (persistedChannelId && group.channels.some((channel) => channel.id === persistedChannelId)) {
    return persistedChannelId;
  }
  return group.channels.find((channel) => channel.kind === "text")?.id ?? group.channels[0]?.id ?? "";
}

export function mergeMessages(current: ChatMessage[], incoming: ChatMessage): ChatMessage[] {
  if (current.some((message) => message.id === incoming.id)) return current;
  return [...current, incoming].sort((left, right) => left.created_at - right.created_at || left.id.localeCompare(right.id));
}

export function removeParticipant(state: CallState | null | undefined, peerId: string): CallState | null {
  if (!state) return null;
  const participants = state.participants.filter((participant) => participant.peer_id !== peerId);
  return participants.length ? { ...state, participants } : null;
}

export function memberIsInCall(states: Record<string, CallState>, groupId: string, peerId: string): boolean {
  return Object.values(states).some((state) => state.group_id === groupId
    && state.participants.some((participant) => participant.peer_id === peerId));
}

export function activeMembers(members: GroupMember[]): GroupMember[] {
  return members.filter((member) => member.status === "active");
}

export function peerIsOnline(peerId: string, localPeerId: string, nodeRunning: boolean, connectedPeerIds: ReadonlySet<string>): boolean {
  return nodeRunning && (peerId === localPeerId || connectedPeerIds.has(peerId));
}
