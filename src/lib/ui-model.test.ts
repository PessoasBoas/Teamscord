import { describe, expect, it } from "vitest";
import type { CallState, ChatMessage, Group, GroupMember } from "./tauri";
import { activeMembers, callKey, memberIsInCall, mergeMessages, normalizeGroup, peerIsOnline, removeParticipant, selectInitialChannel } from "./ui-model";

const group = (channels: Group["channels"]): Group => ({
  id: "group-1",
  name: "Amigos",
  initials: "AM",
  color: "#8276ff",
  owner_peer_id: "peer-owner",
  created_at: 1,
  channels,
});

const message = (id: string, created_at: number): ChatMessage => ({
  id,
  group_id: "group-1",
  channel_id: "group-1:geral",
  author_peer_id: "peer-1",
  author: "Membro",
  initials: "ME",
  color: "#8276ff",
  content: id,
  timestamp: "00:00",
  created_at,
  mine: false,
});

describe("modelo de estado da interface", () => {
  it("migra canais screen antigos para voz e restaura o canal persistido", () => {
    const migrated = normalizeGroup(group([
      { id: "group-1:screen", group_id: "group-1", name: "transmissão", kind: "screen" as "voice", position: 0 },
      { id: "group-1:geral", group_id: "group-1", name: "geral", kind: "text", position: 1 },
    ]));
    expect(migrated.channels[0].kind).toBe("voice");
    expect(selectInitialChannel(migrated, "group-1:geral")).toBe("group-1:geral");
    expect(selectInitialChannel(migrated, "missing")).toBe("group-1:geral");
  });

  it("deduplica e ordena mensagens recebidas fora de ordem", () => {
    const first = mergeMessages([message("b", 20)], message("a", 10));
    expect(first.map((item) => item.id)).toEqual(["a", "b"]);
    expect(mergeMessages(first, message("a", 10))).toBe(first);
  });

  it("mantém a call remota quando o participante local sai", () => {
    const state: CallState = {
      group_id: "group-1",
      channel_id: "group-1:lounge",
      call_id: "call-1",
      participants: [
        { peer_id: "local", display_name: "Local", role: "member", muted: false, sharing_screen: false, connection_state: "connected" },
        { peer_id: "remote", display_name: "Remoto", role: "member", muted: true, sharing_screen: true, connection_state: "connected" },
      ],
    };
    const remaining = removeParticipant(state, "local");
    expect(remaining?.participants.map((participant) => participant.peer_id)).toEqual(["remote"]);
    expect(memberIsInCall({ [callKey(state.group_id, state.channel_id)]: remaining! }, "group-1", "remote")).toBe(true);
    expect(removeParticipant(remaining, "remote")).toBeNull();
  });

  it("exibe somente membros ativos na lista de presença", () => {
    const members: GroupMember[] = [
      { group_id: "group-1", peer_id: "active", display_name: "Ativo", role: "member", public_key: [], x25519_public_key: [], status: "active", joined_at: 1, updated_at: 1 },
      { group_id: "group-1", peer_id: "banned", display_name: "Banido", role: "member", public_key: [], x25519_public_key: [], status: "banned", joined_at: 1, updated_at: 1 },
    ];
    expect(activeMembers(members).map((member) => member.peer_id)).toEqual(["active"]);
  });

  it("só exibe presença online quando o node está ativo", () => {
    const connected = new Set(["remote"]);
    expect(peerIsOnline("local", "local", true, connected)).toBe(true);
    expect(peerIsOnline("remote", "local", true, connected)).toBe(true);
    expect(peerIsOnline("local", "local", false, connected)).toBe(false);
    expect(peerIsOnline("remote", "local", false, connected)).toBe(false);
  });
});
