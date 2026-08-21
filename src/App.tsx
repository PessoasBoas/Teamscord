import { type CSSProperties, type FormEvent, type MouseEvent as ReactMouseEvent, useEffect, useMemo, useRef, useState } from "react";
import {
  Bell, ChevronDown, CircleHelp, Copy, Hash, Headphones, LogOut, Menu, Mic, MicOff, MonitorUp,
  Plus, Search, Settings, ShieldCheck, Signal, UserPlus, Users, Volume2, Wifi, X,
} from "lucide-react";
import { CallDock, CallStage, type CallStageHandle } from "./components/CallStage";
import { ChatPanel, EmptyWorkspace } from "./components/Chat";
import { GroupSettings } from "./components/GroupSettings";
import { channelIcon, ChannelRow, ChannelSection, MemberRow } from "./components/Navigation";
import { NetworkModal } from "./components/NetworkModal";
import { MenuCard, ModalHeader, PopoverPanel } from "./components/Ui";
import { UpdateModal } from "./components/UpdateModal";
import { UserSettings } from "./components/UserSettings";
import { callKey, memberIsInCall, mergeMessages, normalizeGroup, peerIsOnline, removeParticipant, selectInitialChannel } from "./lib/ui-model";
import { checkForUpdate, DISMISSED_UPDATE_STORAGE, type UpdateInfo } from "./lib/update-checker";
import {
  type AppNotification, type CallSignalEvent, type CallState, type Channel, type ChatMessage, type Group,
  type GroupMember, isDesktop, nodeApi, type NetworkDiagnostics, type NodeSnapshot, type NetworkStatus, type SearchResult, type UserPreferences,
} from "./lib/tauri";

const STORAGE_GROUP = "teamscord.active-group";
const STORAGE_CHANNEL = "teamscord.active-channel";
const STORAGE_PREFERENCES = "teamscord.user-preferences";
const DEFAULT_PREFERENCES: UserPreferences = { theme: "dark", font: "manrope", scale: "comfortable", display_name: "Você" };

type Modal = "network" | "create" | "join" | "settings" | "user-settings" | "create-channel" | null;
type Popover = "server" | "rail" | "notifications" | "search" | "members" | "profile" | null;
type CallTarget = { groupId: string; channelId: string };
type ContextMenuState = { kind: "server" | "channel" | "member" | "message"; groupId: string; channelId?: string; peerId?: string; messageId?: string; x: number; y: number };

function readStored(key: string) { return typeof window === "undefined" ? "" : localStorage.getItem(key) ?? ""; }
function readPreferences(): UserPreferences {
  if (typeof window === "undefined") return DEFAULT_PREFERENCES;
  try { return { ...DEFAULT_PREFERENCES, ...JSON.parse(localStorage.getItem(STORAGE_PREFERENCES) ?? "{}") }; } catch { return DEFAULT_PREFERENCES; }
}
function statusCopy(status: NetworkStatus) { return status === "online" ? "node online" : status === "starting" ? "conectando node" : status === "syncing" ? "sincronizando" : status === "reconnecting" ? "reconectando" : status === "preview" ? "prévia web" : "node offline"; }

function App() {
  const [groups, setGroups] = useState<Group[]>([]);
  const [activeGroupId, setActiveGroupId] = useState(() => readStored(STORAGE_GROUP));
  const [activeChannelId, setActiveChannelId] = useState(() => readStored(STORAGE_CHANNEL));
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [historyRevision, setHistoryRevision] = useState(0);
  const [draft, setDraft] = useState("");
  const [members, setMembers] = useState<GroupMember[]>([]);
  const [node, setNode] = useState<NodeSnapshot>({ peer_id: "", listen_addresses: [], connected_peers: 0, is_running: false, relay_addresses: [], bootstrap_addresses: [], relay_connected: false, last_sync_at: null });
  const [connectedPeerIds, setConnectedPeerIds] = useState<Set<string>>(() => new Set());
  const [nodeStatus, setNodeStatus] = useState<NetworkStatus>(isDesktop() ? "starting" : "preview");
  const [syncLabel, setSyncLabel] = useState("aguardando sincronização");
  const [loadingMessages, setLoadingMessages] = useState(false);
  const [loadingMembers, setLoadingMembers] = useState(false);
  const [busy, setBusy] = useState(false);
  const [modal, setModal] = useState<Modal>(null);
  const [popover, setPopover] = useState<Popover>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);
  const [groupName, setGroupName] = useState("");
  const [inviteText, setInviteText] = useState("");
  const [generatedInvite, setGeneratedInvite] = useState("");
  const [peerAddress, setPeerAddress] = useState("");
  const [relayAddress, setRelayAddress] = useState("");
  const [bootstrapAddress, setBootstrapAddress] = useState("");
  const [mediaConfigText, setMediaConfigText] = useState("[]");
  const [feedback, setFeedback] = useState("");
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const [muted, setMuted] = useState(false);
  const [deafened, setDeafened] = useState(false);
  const [callTarget, setCallTarget] = useState<CallTarget | null>(null);
  const [callSignals, setCallSignals] = useState<CallSignalEvent[]>([]);
  const [callStates, setCallStates] = useState<Record<string, CallState>>({});
  const [notifications, setNotifications] = useState<AppNotification[]>([]);
  const [unreadByChannel, setUnreadByChannel] = useState<Record<string, number>>({});
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<SearchResult[]>([]);
  const [preferences, setPreferences] = useState<UserPreferences>(readPreferences);
  const [availableUpdate, setAvailableUpdate] = useState<UpdateInfo | null>(null);
  const [networkDiagnostics, setNetworkDiagnostics] = useState<NetworkDiagnostics | null>(null);
  const [checkingDiagnostics, setCheckingDiagnostics] = useState(false);
  const [newChannelName, setNewChannelName] = useState("");
  const [newChannelKind, setNewChannelKind] = useState<Channel["kind"]>("text");
  const activeSelection = useRef({ groupId: "", channelId: "" });
  const localPeerIdRef = useRef("");
  const callTargetRef = useRef<CallTarget | null>(null);
  const callRef = useRef<CallStageHandle>(null);

  const activeGroup = groups.find((group) => group.id === activeGroupId) ?? groups[0];
  const channels = activeGroup?.channels ?? [];
  const activeChannel = channels.find((channel) => channel.id === activeChannelId) ?? channels.find((channel) => channel.kind === "text") ?? channels[0];
  const textChannels = channels.filter((channel) => channel.kind === "text");
  const voiceChannels = channels.filter((channel) => channel.kind === "voice");
  const visibleCallTarget = callTarget ?? (activeGroup && activeChannel?.kind === "voice" ? { groupId: activeGroup.id, channelId: activeChannel.id } : null);
  const callIsVisible = Boolean(visibleCallTarget && activeGroup?.id === visibleCallTarget.groupId && activeChannel?.id === visibleCallTarget.channelId && activeChannel.kind === "voice");
  const unreadNotifications = notifications.filter((notification) => !notification.read).length;
  const channelDescription = activeChannel?.kind === "voice" ? "chat, áudio e compartilhamento de tela no mesmo canal" : activeChannel ? "conversa cifrada entre os nodes do grupo" : "crie ou entre em um grupo para começar";

  useEffect(() => {
    document.documentElement.dataset.font = preferences.font;
    document.documentElement.dataset.scale = preferences.scale;
    localStorage.setItem(STORAGE_PREFERENCES, JSON.stringify(preferences));
    const media = window.matchMedia?.("(prefers-color-scheme: light)");
    const applyTheme = () => {
      document.documentElement.dataset.theme = preferences.theme === "system"
        ? (media?.matches ? "light" : "dark")
        : preferences.theme;
    };
    applyTheme();
    if (preferences.theme !== "system" || !media) return;
    media.addEventListener?.("change", applyTheme);
    return () => media.removeEventListener?.("change", applyTheme);
  }, [preferences]);

  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    const check = async () => {
      const update = await checkForUpdate();
      if (cancelled || !update || readStored(DISMISSED_UPDATE_STORAGE) === update.version) return;
      setAvailableUpdate(update);
    };
    const initialTimer = window.setTimeout(() => void check(), 1_500);
    const interval = window.setInterval(() => void check(), 6 * 60 * 60 * 1_000);
    return () => {
      cancelled = true;
      window.clearTimeout(initialTimer);
      window.clearInterval(interval);
    };
  }, []);

  function dismissUpdate() {
    if (availableUpdate) localStorage.setItem(DISMISSED_UPDATE_STORAGE, availableUpdate.version);
    setAvailableUpdate(null);
  }

  function handleGlobalKeyDown(event: KeyboardEvent) {
    const target = event.target as HTMLElement | null;
    const editing = target?.matches("input, textarea, select");
    const escape = event.key === "Escape" || event.key === "Esc" || event.code === "Escape";
    if (escape) {
      if (modal) setModal(null);
      else if (popover) setPopover(null);
      else if (contextMenu) setContextMenu(null);
      event.preventDefault();
      return;
    }
    if (!editing && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      setPopover("search");
    }
  }

  useEffect(() => {
    document.addEventListener("keydown", handleGlobalKeyDown, true);
    return () => document.removeEventListener("keydown", handleGlobalKeyDown, true);
  }, [modal, popover, contextMenu]);

  async function refreshGroups() {
    const loaded = (await nodeApi.getGroups()).map(normalizeGroup);
    setGroups(loaded);
    return loaded;
  }

  async function refreshMembers(groupId: string) {
    if (!isDesktop()) return;
    setLoadingMembers(true);
    try { setMembers(await nodeApi.listGroupMembers(groupId)); } catch (reason) { setError(String(reason)); } finally { setLoadingMembers(false); }
  }

  useEffect(() => {
    if (!isDesktop()) return;
    let dispose: (() => void) | undefined;
    void (async () => {
      try {
        const snapshot = await nodeApi.startNode();
        setNode(snapshot);
        localPeerIdRef.current = snapshot.peer_id;
        setConnectedPeerIds(new Set([snapshot.peer_id]));
        setNodeStatus(snapshot.is_running ? "online" : "offline");
        await refreshGroups();
        dispose = await nodeApi.onNodeEvent((event) => {
          if (event.snapshot) { localPeerIdRef.current = event.snapshot.peer_id; setNode(event.snapshot); setConnectedPeerIds((current) => new Set([...current, event.snapshot?.peer_id ?? ""])); setNodeStatus(event.snapshot.is_running ? "online" : "offline"); }
          if (event.message) {
            const incoming = event.message;
            const current = activeSelection.current;
            if (incoming.group_id === current.groupId && incoming.channel_id === current.channelId) {
              setMessages((items) => mergeMessages(items, incoming));
            } else {
              setUnreadByChannel((items) => ({ ...items, [callKey(incoming.group_id, incoming.channel_id)]: (items[callKey(incoming.group_id, incoming.channel_id)] ?? 0) + 1 }));
              setNotifications((items) => [{ id: incoming.id, kind: "message" as const, title: incoming.author, body: incoming.content, created_at: incoming.created_at, read: false, group_id: incoming.group_id, channel_id: incoming.channel_id }, ...items].slice(0, 100));
            }
          }
          if (event.kind === "call-signal" && event.data) {
            const signal = event.data as CallSignalEvent;
            setCallSignals((items) => [...items.filter((item) => item.signal.event_id !== signal.signal.event_id), signal].slice(-200));
            const signalMeta = signal.signal as { event_id?: string; from_peer_id?: string; group_id?: string; channel_id?: string };
            const fromPeerId = signalMeta.from_peer_id;
            if (signal.body.kind === "join" && fromPeerId && fromPeerId !== localPeerIdRef.current) {
              setNotifications((items) => items.some((item) => item.id === signalMeta.event_id) ? items : [{ id: signalMeta.event_id ?? crypto.randomUUID(), kind: "call" as const, title: "Entrada na call", body: `${fromPeerId.slice(0, 12)} entrou em uma call.`, created_at: Date.now(), read: false, group_id: signalMeta.group_id, channel_id: signalMeta.channel_id }, ...items].slice(0, 100));
            }
          }
          if (event.kind === "call-state" && event.data) {
            const state = event.data as CallState;
            setCallStates((items) => ({ ...items, [callKey(state.group_id, state.channel_id)]: state }));
          }
          if (event.kind === "sync-state" && event.data) {
            const state = event.data as { state?: string };
            setSyncLabel(state.state === "synced" ? "sincronizado" : state.state === "error" ? "erro de sincronização" : state.state === "waiting" ? "aguardando peer" : "sincronizando");
            setNodeStatus(state.state === "syncing" ? "syncing" : "online");
          }
          if (event.kind === "peer-updated" && event.data) {
            const peer = event.data as { peer_id?: string; state?: string };
            const peerId = peer.peer_id;
            setNodeStatus(peer.state === "disconnected" ? "reconnecting" : "syncing");
          }
          if (event.kind === "peer-presence" && event.data) {
            const presence = event.data as { peer_id?: string; state?: string };
            if (presence.peer_id) setConnectedPeerIds((current) => {
              const next = new Set(current);
              if (presence.state === "online") next.add(presence.peer_id!);
              else if (presence.state === "offline") next.delete(presence.peer_id!);
              return next;
            });
            setNodeStatus(presence.state === "offline" ? "reconnecting" : "online");
            if (presence.state === "online") void callRef.current?.reannounce().catch((reason) => setError(String(reason)));
          }
          if (event.kind === "relay-state" && event.data) {
            const relay = event.data as { state?: string };
            setFeedback(relay.state === "connected" ? "relay conectado" : "relay desconectado — tentando conexão direta");
          }
          if (event.data && (event.kind === "group-control" || event.kind === "member-updated")) {
            const control = event.data as { group_id?: string; kind?: string; target_peer_id?: string | null };
            if (control.kind === "group_deleted") {
              const currentCall = callTargetRef.current;
              if (currentCall?.groupId === control.group_id) {
                void callRef.current?.leave();
                setCallTarget(null);
              }
              setCallStates((items) => Object.fromEntries(Object.entries(items).filter(([key]) => !key.startsWith(`${control.group_id}:`))));
            } else if (control.kind === "message_deleted") {
              setHistoryRevision((value) => value + 1);
            } else if (control.target_peer_id === localPeerIdRef.current
              && control.group_id === callTargetRef.current?.groupId
              && ["member_kicked", "member_banned", "member_left"].includes(control.kind ?? "")) {
              void callRef.current?.leave();
              setCallTarget(null);
            }
          }
          if (event.kind === "group-control" || event.kind === "channel-updated" || event.kind === "member-updated" || event.kind === "key-epoch-changed") {
            void refreshGroups().catch((reason) => setError(String(reason)));
            if (activeSelection.current.groupId) void refreshMembers(activeSelection.current.groupId);
            if (event.kind === "key-epoch-changed") setHistoryRevision((value) => value + 1);
            if (event.kind !== "key-epoch-changed") setNotifications((items) => [{ id: crypto.randomUUID(), kind: "member" as const, title: "Grupo atualizado", body: "As alterações foram sincronizadas entre os nodes.", created_at: Date.now(), read: false, group_id: activeSelection.current.groupId }, ...items].slice(0, 100));
          }
          if (event.kind === "media-error") setError(event.error ?? "falha de mídia");
          if (event.error) setError(event.error);
        });
        const currentSnapshot = await nodeApi.getNodeSnapshot();
        localPeerIdRef.current = currentSnapshot.peer_id;
        setNode(currentSnapshot);
        setConnectedPeerIds((current) => new Set([...current, currentSnapshot.peer_id]));
        setNodeStatus(currentSnapshot.is_running ? "online" : "offline");
      } catch (reason) { setNodeStatus("offline"); setError(String(reason)); }
    })();
    return () => dispose?.();
  }, []);

  useEffect(() => {
    activeSelection.current = { groupId: activeGroup?.id ?? "", channelId: activeChannel?.id ?? "" };
    if (activeGroup?.id) void refreshMembers(activeGroup.id);
  }, [activeGroup?.id, activeChannel?.id]);

  useEffect(() => { callTargetRef.current = callTarget; }, [callTarget]);

  useEffect(() => {
    if (activeGroupId) localStorage.setItem(STORAGE_GROUP, activeGroupId);
    if (activeChannelId) localStorage.setItem(STORAGE_CHANNEL, activeChannelId);
  }, [activeGroupId, activeChannelId]);

  useEffect(() => {
    if (!groups.length) { setActiveGroupId(""); setActiveChannelId(""); return; }
    if (!groups.some((group) => group.id === activeGroupId)) {
      setActiveGroupId(groups[0].id);
      setActiveChannelId(selectInitialChannel(groups[0], activeChannelId));
    }
  }, [groups, activeGroupId]);

  useEffect(() => {
    if (activeGroup && !channels.some((channel) => channel.id === activeChannelId)) setActiveChannelId(selectInitialChannel(activeGroup));
  }, [activeGroup?.id, activeChannelId, channels]);

  useEffect(() => {
    if (!activeGroup || !activeChannel || !isDesktop()) return;
    let cancelled = false;
    setLoadingMessages(true);
    setNodeStatus("syncing");
    void nodeApi.getMessages(activeGroup.id, activeChannel.id).then((loaded) => {
      if (cancelled) return;
      setMessages(loaded);
      setUnreadByChannel((items) => ({ ...items, [callKey(activeGroup.id, activeChannel.id)]: 0 }));
    }).catch((reason) => { if (!cancelled) setError(String(reason)); }).finally(() => {
      if (cancelled) return;
      setLoadingMessages(false);
      setNodeStatus((current) => current === "offline" ? "offline" : "online");
      setSyncLabel("sincronizado");
    });
    return () => { cancelled = true; };
  }, [activeGroup?.id, activeChannel?.id, historyRevision]);

  useEffect(() => {
    if (popover !== "search" || !activeGroup || !searchQuery.trim() || !isDesktop()) { if (!searchQuery.trim()) setSearchResults([]); return; }
    const timer = window.setTimeout(() => void nodeApi.searchMessages(activeGroup.id, searchQuery.trim()).then(setSearchResults).catch((reason) => setError(String(reason))), 180);
    return () => window.clearTimeout(timer);
  }, [popover, searchQuery, activeGroup?.id]);

  useEffect(() => {
    if (modal !== "network" || !isDesktop()) return;
    void nodeApi.getMediaConfig().then((config) => setMediaConfigText(JSON.stringify(config.ice_servers, null, 2))).catch((reason) => setError(String(reason)));
  }, [modal]);

  async function runNetworkDiagnostics() {
    if (!isDesktop()) return;
    setCheckingDiagnostics(true);
    try { setNetworkDiagnostics(await nodeApi.runNetworkDiagnostics()); }
    catch (reason) { setError(`verificação P2P falhou: ${String(reason)}`); }
    finally { setCheckingDiagnostics(false); }
  }

  useEffect(() => {
    if (!isDesktop()) return;
    let cancelled = false;
    const check = async () => {
      try {
        const result = await nodeApi.runNetworkDiagnostics();
        if (!cancelled) setNetworkDiagnostics(result);
      } catch {
        // A checagem automática não deve interromper o uso do chat; o painel mostra o último resultado válido.
      }
    };
    void check();
    const interval = window.setInterval(() => void check(), 15_000);
    return () => { cancelled = true; window.clearInterval(interval); };
  }, []);

  function selectGroup(group: Group) { setActiveGroupId(group.id); setActiveChannelId(selectInitialChannel(group)); setMessages([]); setPopover(null); setContextMenu(null); setError(""); }
  function selectChannel(channel: Channel) { setActiveChannelId(channel.id); setMessages([]); setPopover(null); setContextMenu(null); setFeedback(""); }
  function openModal(next: Modal) { setPopover(null); setContextMenu(null); setModal(next); }
  function openContextMenu(event: ReactMouseEvent, target: Omit<ContextMenuState, "x" | "y">) {
    event.preventDefault();
    setPopover(null);
    setContextMenu({ ...target, x: Math.min(event.clientX, window.innerWidth - 230), y: Math.min(event.clientY, window.innerHeight - 190) });
  }

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!activeGroup || !activeChannel || !draft.trim()) return;
    if (!isDesktop()) { setError("Abra o app desktop para enviar mensagens pela rede."); return; }
    setBusy(true);
    try { const message = await nodeApi.sendMessage(activeGroup.id, activeChannel.id, draft.trim(), preferences.display_name); setMessages((items) => mergeMessages(items, message)); setDraft(""); setError(""); } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }

  async function handleCreateGroup(event: FormEvent) {
    event.preventDefault();
    if (!groupName.trim() || !isDesktop()) return;
    setBusy(true);
    try { const result = await nodeApi.createGroup(groupName); const group = normalizeGroup(result.group); setGroups((items) => [...items.filter((item) => item.id !== group.id), group]); selectGroup(group); setGeneratedInvite(result.invite); setGroupName(""); setFeedback("grupo criado — compartilhe o convite"); setError(""); } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }

  async function handleJoinGroup(event: FormEvent) {
    event.preventDefault();
    if (!inviteText.trim() || !isDesktop()) return;
    setBusy(true);
    try { const group = normalizeGroup(await nodeApi.joinGroup(inviteText)); setGroups((items) => [...items.filter((item) => item.id !== group.id), group]); selectGroup(group); setInviteText(""); setModal(null); setFeedback(`você entrou em ${group.name}`); setError(""); } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }

  async function createChannel(event: FormEvent) {
    event.preventDefault();
    if (!activeGroup || !newChannelName.trim() || !isDesktop()) return;
    try { const channel = await nodeApi.createChannel(activeGroup.id, newChannelName.trim(), newChannelKind); const next = normalizeGroup({ ...activeGroup, channels: [...activeGroup.channels, channel] }); setGroups((items) => items.map((group) => group.id === next.id ? next : group)); setNewChannelName(""); setModal(null); setFeedback(`${newChannelKind === "voice" ? "canal de voz" : "canal de texto"} criado`); } catch (reason) { setError(String(reason)); }
  }

  async function leaveActiveGroup() {
    if (!activeGroup || !isDesktop() || !window.confirm(`Sair de ${activeGroup.name}? Você precisará de um novo convite para voltar.`)) return;
    try {
      if (callTarget?.groupId === activeGroup.id) await callRef.current?.leave();
      await nodeApi.leaveGroup(activeGroup.id);
      if (callTarget?.groupId === activeGroup.id) setCallTarget(null);
      const next = await refreshGroups();
      if (!next.some((group) => group.id === activeGroup.id)) { setActiveGroupId(next[0]?.id ?? ""); setActiveChannelId(next[0]?.channels[0]?.id ?? ""); }
      setFeedback("você saiu do servidor");
    } catch (reason) { setError(String(reason)); }
  }

  async function handleGroupDeleted() {
    setModal(null);
    const next = await refreshGroups();
    setActiveGroupId(next[0]?.id ?? "");
    setActiveChannelId(next[0]?.channels[0]?.id ?? "");
    setMessages([]);
  }

  async function copyText(value: string, label: string) { try { await navigator.clipboard?.writeText(value); setCopied(true); setFeedback(label); window.setTimeout(() => setCopied(false), 1400); } catch { setError("não foi possível copiar para a área de transferência"); } }
  async function connectPeer(event: FormEvent) { event.preventDefault(); if (!peerAddress.trim() || !isDesktop()) return; try { await nodeApi.connectPeer(peerAddress.trim()); setFeedback("tentativa de conexão enviada"); setPeerAddress(""); } catch (reason) { setError(String(reason)); } }
  async function addRelay(event: FormEvent) { event.preventDefault(); const address = relayAddress.trim(); if (!address || !isDesktop()) return; try { const config = await nodeApi.setNetworkConfig([...node.relay_addresses, address], node.bootstrap_addresses); setNode((current) => ({ ...current, relay_addresses: config.relay_addresses, bootstrap_addresses: config.bootstrap_addresses })); setRelayAddress(""); setFeedback("relay salvo"); } catch (reason) { setError(String(reason)); } }
  async function addBootstrap(event: FormEvent) { event.preventDefault(); const address = bootstrapAddress.trim(); if (!address || !isDesktop()) return; try { const config = await nodeApi.setNetworkConfig(node.relay_addresses, [...node.bootstrap_addresses, address]); setNode((current) => ({ ...current, relay_addresses: config.relay_addresses, bootstrap_addresses: config.bootstrap_addresses })); setBootstrapAddress(""); setFeedback("bootstrap salvo — conexão automática iniciada"); } catch (reason) { setError(String(reason)); } }
  async function saveMediaConfig(event: FormEvent) { event.preventDefault(); try { const parsed = JSON.parse(mediaConfigText); if (!Array.isArray(parsed)) throw new Error("ICE precisa ser uma lista JSON"); await nodeApi.setMediaConfig(parsed); setFeedback("configuração ICE salva no Credential Manager"); } catch (reason) { setError(`configuração ICE inválida: ${String(reason)}`); } }
  async function deleteContextChannel() {
    const target = contextMenu;
    if (!target?.channelId || !window.confirm("Excluir este canal? A ação será registrada na auditoria.")) return;
    try {
      await nodeApi.deleteChannel(target.groupId, target.channelId);
      setContextMenu(null);
      await refreshGroups();
      setFeedback("canal excluído");
    } catch (reason) { setError(String(reason)); }
  }
  async function deleteContextMessage() {
    const target = contextMenu;
    if (!target?.channelId || !target.messageId || !window.confirm("Excluir esta mensagem? A ação ficará registrada na auditoria.")) return;
    try {
      await nodeApi.deleteMessage(target.groupId, target.channelId, target.messageId);
      setMessages((items) => items.filter((message) => message.id !== target.messageId));
      setContextMenu(null);
      setFeedback("mensagem excluída");
    } catch (reason) { setError(String(reason)); }
  }
  function openContextGroupSettings(groupId: string) {
    const group = groups.find((item) => item.id === groupId);
    if (!group) return;
    setActiveGroupId(group.id);
    setActiveChannelId(selectInitialChannel(group));
    setMessages([]);
    setPopover(null);
    setContextMenu(null);
    setModal("settings");
  }
  function openContextCreateChannel(groupId: string, kind: Channel["kind"]) {
    const group = groups.find((item) => item.id === groupId);
    if (!group) return;
    setActiveGroupId(group.id);
    setActiveChannelId(selectInitialChannel(group));
    setNewChannelKind(kind);
    setPopover(null);
    setContextMenu(null);
    setModal("create-channel");
  }
  function openContextChannel(groupId: string, channelId: string) {
    const group = groups.find((item) => item.id === groupId);
    const channel = group?.channels.find((item) => item.id === channelId);
    if (!group || !channel) return;
    setActiveGroupId(group.id);
    setActiveChannelId(channel.id);
    setMessages([]);
    setPopover(null);
    setContextMenu(null);
  }
  async function copyContextMemberPeerId(peerId: string) {
    setContextMenu(null);
    await copyText(peerId, "peer id do membro copiado");
  }
  async function requestContextMemberMute(peerId: string) {
    setContextMenu(null);
    await callRef.current?.requestMute(peerId);
    setFeedback("solicitação de mute enviada");
  }
  function handlePreferences(next: UserPreferences) { setPreferences(next); }
  async function toggleLocalMute() {
    if (callTarget) {
      await callRef.current?.toggleMute();
      return;
    }
    setMuted((value) => !value);
    setFeedback("preferência de microfone atualizada para a próxima call");
  }
  function markNotificationsRead() { setNotifications((items) => items.map((item) => ({ ...item, read: true }))); }
  function memberInActiveCall(peerId: string) {
    return activeGroup ? memberIsInCall(callStates, activeGroup.id, peerId) : false;
  }
  function memberIsOnline(peerId: string) { return peerIsOnline(peerId, node.peer_id, node.is_running, connectedPeerIds); }
  function onCallJoined(state: CallState) { setCallTarget({ groupId: state.group_id, channelId: state.channel_id }); setCallStates((items) => ({ ...items, [callKey(state.group_id, state.channel_id)]: state })); setFeedback("você entrou na call — a sessão continua ao navegar"); }
  function onCallLeft() {
    const currentCall = callTargetRef.current;
    if (currentCall) {
      setCallStates((items) => {
        const next = { ...items };
        const key = callKey(currentCall.groupId, currentCall.channelId);
        const currentState = next[key];
        if (!currentState) return next;
        const remaining = removeParticipant(currentState, localPeerIdRef.current);
        if (remaining) next[key] = remaining;
        else delete next[key];
        return next;
      });
    }
    callTargetRef.current = null;
    setCallTarget(null);
    setMuted(false);
    setDeafened(false);
    setFeedback("você saiu da call");
  }

  const statusLabel = statusCopy(nodeStatus);
  const renderCall = visibleCallTarget ? <CallStage ref={callRef} groupId={visibleCallTarget.groupId} channelId={visibleCallTarget.channelId} localPeerId={node.peer_id} displayName={preferences.display_name} signals={callSignals.filter((signal) => { const value = signal.signal as { group_id?: string; channel_id?: string }; return value.group_id === visibleCallTarget.groupId && value.channel_id === visibleCallTarget.channelId; })} sharedState={callStates[callKey(visibleCallTarget.groupId, visibleCallTarget.channelId)] ?? null} muted={muted} deafened={deafened} compact={false} onToggleDeafened={() => setDeafened((value) => !value)} onMutedChange={setMuted} onJoined={onCallJoined} onLeft={onCallLeft} onState={(state) => setCallStates((items) => ({ ...items, [callKey(state.group_id, state.channel_id)]: state }))} onError={setError} /> : null;

  return <div className="app-shell" tabIndex={-1} onClick={() => contextMenu && setContextMenu(null)} onKeyDownCapture={(event) => {
    if (event.key === "Escape" || event.key === "Esc") {
      event.preventDefault();
      if (modal) setModal(null);
      else if (popover) setPopover(null);
      else if (contextMenu) setContextMenu(null);
    }
  }}>
    <aside className="server-rail">
      <div className="brand-mark">T</div><div className="server-divider" />
      {groups.map((group) => <button className={`server-button ${activeGroup?.id === group.id ? "active" : ""}`} key={group.id} style={{ "--server-color": group.color } as CSSProperties} onClick={() => selectGroup(group)} onContextMenu={(event) => openContextMenu(event, { kind: "server", groupId: group.id })} title={group.name}><span>{group.initials}</span>{activeGroup?.id === group.id && <i />}</button>)}
      <button className="server-button add-server" title="Criar ou entrar em servidor" onClick={() => setPopover(popover === "rail" ? null : "rail")}><Plus size={21} /></button>
      {popover === "rail" && <MenuCard className="rail-menu"><button onClick={() => openModal("create")}><Plus size={15} /> Criar servidor</button><button onClick={() => openModal("join")}><UserPlus size={15} /> Entrar por convite</button></MenuCard>}
      <div className="rail-bottom"><button className="icon-button" title="Ajuda" aria-label="Ajuda"><CircleHelp size={20} /></button></div>
    </aside>

      <aside className="channel-sidebar">
      <button className="workspace-picker" onClick={() => setPopover(popover === "server" ? null : "server")} onContextMenu={(event) => activeGroup && openContextMenu(event, { kind: "server", groupId: activeGroup.id })} aria-expanded={popover === "server"}><span>{activeGroup?.name ?? "Seus grupos"}</span><ChevronDown size={17} /></button>
      {popover === "server" && <MenuCard className="server-menu"><button onClick={() => activeGroup && void nodeApi.createInvite(activeGroup.id).then((invite) => copyText(invite, "convite do servidor copiado")).catch((reason) => setError(String(reason)))} disabled={!activeGroup}><UserPlus size={15} /> Criar link de convite</button><button onClick={() => openModal("settings")} disabled={!activeGroup}><Settings size={15} /> Configurações do servidor</button><button onClick={() => openModal("create-channel")} disabled={!activeGroup}><Plus size={15} /> Criar canal</button><span className="menu-separator" /><button onClick={() => void leaveActiveGroup()} disabled={!activeGroup}><LogOut size={15} /> Sair do servidor</button></MenuCard>}
      <div className="sidebar-scroll">
        <div className={`network-card ${nodeStatus === "offline" ? "network-card-offline" : ""}`}><div className="network-card-heading"><span className="live-dot" />{statusLabel.toUpperCase()}</div><div className="network-card-copy">{syncLabel} · mensagens cifradas e distribuídas entre os nodes conectados</div><button onClick={() => openModal("network")}>ver detalhes <Signal size={14} /></button></div>
        {activeGroup ? <>
          <ChannelSection title="texto" icon={<Hash size={13} />} action={<button className="section-action" onClick={() => { setNewChannelKind("text"); openModal("create-channel"); }} aria-label="Criar canal de texto"><Plus size={15} /></button>}>
            {textChannels.map((channel) => <ChannelRow key={channel.id} channel={channel} active={activeChannel?.id === channel.id} unread={unreadByChannel[callKey(activeGroup.id, channel.id)] ?? 0} onClick={() => selectChannel(channel)} onContextMenu={(event) => openContextMenu(event, { kind: "channel", groupId: activeGroup.id, channelId: channel.id })} />)}
          </ChannelSection>
          <ChannelSection title="voz" icon={<Volume2 size={13} />} action={<button className="section-action" onClick={() => { setNewChannelKind("voice"); openModal("create-channel"); }} aria-label="Criar canal de voz"><Plus size={15} /></button>}>
            {voiceChannels.map((channel) => <div key={channel.id}><ChannelRow channel={channel} active={activeChannel?.id === channel.id} unread={unreadByChannel[callKey(activeGroup.id, channel.id)] ?? 0} onClick={() => selectChannel(channel)} onContextMenu={(event) => openContextMenu(event, { kind: "channel", groupId: activeGroup.id, channelId: channel.id })} />{(callStates[callKey(activeGroup.id, channel.id)]?.participants ?? []).map((participant) => <div className="voice-participant" key={participant.peer_id}><span className="avatar tiny avatar-purple">{participant.display_name.slice(0, 2).toUpperCase()}</span><span>{participant.display_name}</span>{participant.muted && <MicOff size={12} />}</div>)}</div>)}
          </ChannelSection>
          <div className="members-heading"><span>membros · {members.length}</span><button className="section-action" onClick={() => setPopover("members")} aria-label="Abrir lista de membros"><Users size={15} /></button></div>
          {loadingMembers ? <div className="sidebar-loading">carregando membros…</div> : members.slice(0, 12).map((member) => <MemberRow key={member.peer_id} member={member} online={memberIsOnline(member.peer_id)} activeCall={memberInActiveCall(member.peer_id)} onContextMenu={(event) => openContextMenu(event, { kind: "member", groupId: activeGroup.id, peerId: member.peer_id })} />)}
          {!loadingMembers && !members.length && <div className="sidebar-loading">nenhum membro sincronizado</div>}
        </> : <div className="sidebar-empty"><Users size={22} /><strong>nenhum grupo ainda</strong><span>crie um servidor ou entre por convite</span><button onClick={() => openModal("create")}>criar servidor</button><button className="secondary-link" onClick={() => openModal("join")}>usar convite</button></div>}
      </div>
      <div className={`user-panel ${callTarget ? "has-call" : ""}`}>
        {callTarget && <CallDock state={callStates[callKey(callTarget.groupId, callTarget.channelId)] ?? null} channelName={groups.find((group) => group.id === callTarget.groupId)?.channels.find((channel) => channel.id === callTarget.channelId)?.name ?? "call"} muted={muted} deafened={deafened} onToggleMute={() => void toggleLocalMute()} onToggleDeafened={() => setDeafened((value) => !value)} onLeave={() => void callRef.current?.leave()} />}
        <div className="user-profile-controls"><span className="avatar avatar-purple">{preferences.display_name.slice(0, 2).toUpperCase()}<span className={`presence ${node.is_running ? "online" : "idle"}`} /></span><div className="user-copy"><strong>{preferences.display_name}</strong><small>{callTarget ? "em call" : statusLabel}</small></div><button className={`icon-button ${muted ? "is-on" : ""}`} onClick={() => void toggleLocalMute()} title={muted ? "Ativar microfone" : "Desativar microfone"} aria-label={muted ? "Ativar microfone" : "Desativar microfone"}>{muted ? <MicOff size={17} /> : <Mic size={17} />}</button><button className={`icon-button ${deafened ? "is-on" : ""}`} onClick={() => setDeafened((value) => !value)} title={deafened ? "Ativar áudio" : "Silenciar participantes"} aria-label={deafened ? "Ativar áudio" : "Silenciar participantes"}>{deafened ? <Volume2 size={17} /> : <Headphones size={17} />}</button><button className="icon-button" title="Configurações do usuário" aria-label="Configurações do usuário" onClick={() => openModal("user-settings")}><Settings size={17} /></button></div>
      </div>
    </aside>

    <main className="main-panel">
      <header className="topbar"><div className="channel-title"><span className="title-icon">{activeChannel ? channelIcon(activeChannel.kind) : <Users size={17} />}</span><div><h1>{activeChannel?.name ?? "comece por um grupo"}</h1><p>{channelDescription}</p></div></div><div className="top-actions"><button className="top-action" title="Membros do servidor" onClick={() => setPopover(popover === "members" ? null : "members")}><Users size={18} /><span>{members.length}</span></button><button className="top-action notification-action" title="Notificações" onClick={() => { setPopover(popover === "notifications" ? null : "notifications"); markNotificationsRead(); }}><Bell size={18} />{unreadNotifications > 0 && <b>{unreadNotifications > 9 ? "9+" : unreadNotifications}</b>}</button><button className="top-action" title="Buscar mensagens, canais e membros" onClick={() => setPopover(popover === "search" ? null : "search")}><Search size={18} /></button><div className="top-divider" /><button className="profile-button" onClick={() => setPopover(popover === "profile" ? null : "profile")}><span className="avatar small avatar-purple">{preferences.display_name.slice(0, 2).toUpperCase()}<span className={`presence ${node.is_running ? "online" : "idle"}`} /></span><ChevronDown size={15} /></button></div>
        {popover === "profile" && <MenuCard className="profile-menu"><strong>{preferences.display_name}</strong><span>{node.peer_id ? `${node.peer_id.slice(0, 18)}…` : "node não iniciado"}</span><button onClick={() => openModal("user-settings")}><Settings size={15} /> Configurações do usuário</button></MenuCard>}
        {popover === "notifications" && <PopoverPanel className="notification-panel" title="Notificações" onClose={() => setPopover(null)}>{notifications.length ? notifications.map((notification) => <article className={`notification-row ${notification.read ? "read" : ""}`} key={notification.id}><span className="notification-icon">{notification.kind === "message" ? <Hash size={14} /> : notification.kind === "call" ? <Volume2 size={14} /> : <ShieldCheck size={14} />}</span><div><strong>{notification.title}</strong><p>{notification.body}</p><time>{new Date(notification.created_at).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}</time></div></article>) : <div className="empty-state">Nenhuma notificação nova.</div>}</PopoverPanel>}
        {popover === "search" && <PopoverPanel className="search-panel" title="Buscar no servidor" onClose={() => setPopover(null)}><input autoFocus value={searchQuery} onChange={(event) => setSearchQuery(event.target.value)} placeholder="mensagens, canais ou membros" />{searchResults.length ? <div className="search-results">{searchResults.map((result) => <button key={`${result.kind}:${result.id}`} onClick={() => { if (result.channel_id) { setActiveGroupId(result.group_id); setActiveChannelId(result.channel_id); } setPopover(null); }}>{result.kind === "message" ? <Hash size={14} /> : result.kind === "member" ? <Users size={14} /> : <Volume2 size={14} />}<span><strong>{result.title}</strong><small>{result.subtitle}</small></span></button>)}</div> : <div className="empty-state">Digite para pesquisar o histórico local.</div>}</PopoverPanel>}
        {popover === "members" && <PopoverPanel className="members-panel" title={`Membros · ${members.length}`} onClose={() => setPopover(null)}>{members.length ? members.map((member) => <MemberRow key={member.peer_id} member={member} online={memberIsOnline(member.peer_id)} activeCall={memberInActiveCall(member.peer_id)} />) : <div className="empty-state">A lista será preenchida após a sincronização.</div>}</PopoverPanel>}
      </header>

      {!activeGroup || !activeChannel ? <EmptyWorkspace onCreate={() => openModal("create")} onJoin={() => openModal("join")} /> : <div className={`workspace-layer ${activeChannel.kind === "voice" ? "voice-layout" : "text-layout"}`}>
        {visibleCallTarget && <div className={`call-stage-host ${callIsVisible ? "full" : "background-call"}`}>{renderCall}</div>}
        <ChatPanel channel={activeChannel} messages={messages} loading={loadingMessages} draft={draft} busy={busy} onDraft={setDraft} onSubmit={handleSubmit} onMessageContextMenu={(event, message) => openContextMenu(event, { kind: "message", groupId: activeGroup.id, channelId: activeChannel.id, messageId: message.id })} />
      </div>}
      <footer className="status-bar"><div className="status-item"><Wifi size={14} /><span>{statusLabel}</span><span className={`status-dot ${nodeStatus === "offline" ? "status-dot-offline" : ""}`} /></div><div className="status-item muted-status">{node.connected_peers} peers próximos <span>·</span> {syncLabel}</div><button onClick={() => openModal("network")} className="network-link">identidade do node <span>{node.peer_id ? `${node.peer_id.slice(0, 12)}…` : "indisponível"}</span></button></footer>
    </main>

      {(error || feedback) && <div className={`toast ${error ? "toast-error" : ""}`} role="status"><span>{error || feedback}</span><button onClick={() => { setError(""); setFeedback(""); }} aria-label="Fechar aviso"><X size={14} /></button></div>}
    {contextMenu && <div className="context-menu" role="menu" style={{ left: contextMenu.x, top: contextMenu.y }} onClick={(event) => event.stopPropagation()} onContextMenu={(event) => event.preventDefault()}>
      {contextMenu.kind === "server" && <>
        <button role="menuitem" onClick={() => { const group = groups.find((item) => item.id === contextMenu.groupId); if (group) void nodeApi.createInvite(group.id).then((invite) => copyText(invite, "convite do servidor copiado")).catch((reason) => setError(String(reason))); setContextMenu(null); }}><UserPlus size={15} /> Criar link de convite</button>
        <button role="menuitem" onClick={() => openContextCreateChannel(contextMenu.groupId, "text")}><Plus size={15} /> Criar canal de texto</button>
        <button role="menuitem" onClick={() => openContextCreateChannel(contextMenu.groupId, "voice")}><Volume2 size={15} /> Criar canal de voz</button>
        <span className="menu-separator" />
        <button role="menuitem" onClick={() => openContextGroupSettings(contextMenu.groupId)}><Settings size={15} /> Configurações do servidor</button>
      </>}
      {contextMenu.kind === "channel" && contextMenu.channelId && <>
        <button role="menuitem" onClick={() => openContextChannel(contextMenu.groupId, contextMenu.channelId ?? "")}><Hash size={15} /> Abrir canal</button>
        <button role="menuitem" onClick={() => openContextGroupSettings(contextMenu.groupId)}><Settings size={15} /> Gerenciar canais</button>
        <button role="menuitem" className="danger-text" onClick={() => void deleteContextChannel()}><X size={15} /> Excluir canal</button>
      </>}
      {contextMenu.kind === "message" && contextMenu.channelId && contextMenu.messageId && <>
        <button role="menuitem" className="danger-text" onClick={() => void deleteContextMessage()}><X size={15} /> Excluir mensagem</button>
      </>}
      {contextMenu.kind === "member" && contextMenu.peerId && <>
        <button role="menuitem" onClick={() => void copyContextMemberPeerId(contextMenu.peerId ?? "")}><Copy size={15} /> Copiar peer ID</button>
        {callTarget && memberInActiveCall(contextMenu.peerId) && <button role="menuitem" onClick={() => void requestContextMemberMute(contextMenu.peerId ?? "")}><MicOff size={15} /> Silenciar na call</button>}
        <button role="menuitem" onClick={() => openContextGroupSettings(contextMenu.groupId)}><ShieldCheck size={15} /> Gerenciar membro</button>
      </>}
    </div>}
    {modal === "network" && <NetworkModal node={node} diagnostics={networkDiagnostics} checkingDiagnostics={checkingDiagnostics} runDiagnostics={() => void runNetworkDiagnostics()} peerAddress={peerAddress} setPeerAddress={setPeerAddress} connectPeer={connectPeer} relayAddress={relayAddress} setRelayAddress={setRelayAddress} addRelay={addRelay} bootstrapAddress={bootstrapAddress} setBootstrapAddress={setBootstrapAddress} addBootstrap={addBootstrap} mediaConfigText={mediaConfigText} setMediaConfigText={setMediaConfigText} saveMediaConfig={saveMediaConfig} copied={copied} copyPeerId={() => copyText(node.peer_id, "peer id copiado")} onClose={() => setModal(null)} />}
    {modal === "create" && <div className="modal-backdrop" onClick={() => setModal(null)}><section className="network-modal form-modal" onClick={(event) => event.stopPropagation()}><ModalHeader eyebrow="NOVO SERVIDOR" title="Criar servidor" onClose={() => setModal(null)} />{generatedInvite ? <><p className="modal-note modal-intro">Servidor criado. Compartilhe este convite com seus amigos; ele expira em 30 dias.</p><div className="invite-box"><code>{generatedInvite}</code><button className="connect-button" onClick={() => void copyText(generatedInvite, "convite copiado")}><Copy size={15} /> {copied ? "copiado" : "copiar"}</button></div><button className="primary-button modal-primary" onClick={() => setModal(null)}>concluir</button></> : <form className="modal-form" onSubmit={handleCreateGroup}><label>nome do servidor</label><input autoFocus value={groupName} onChange={(event) => setGroupName(event.target.value)} placeholder="ex.: Amigos do bairro" maxLength={80} /><button className="primary-button modal-primary" disabled={busy || !groupName.trim()}>{busy ? "criando…" : "criar servidor"}</button></form>}</section></div>}
    {modal === "join" && <div className="modal-backdrop" onClick={() => setModal(null)}><section className="network-modal form-modal" onClick={(event) => event.stopPropagation()}><ModalHeader eyebrow="ENTRAR EM SERVIDOR" title="Usar convite" onClose={() => setModal(null)} /><form className="modal-form" onSubmit={handleJoinGroup}><label>convite assinado</label><textarea autoFocus value={inviteText} onChange={(event) => setInviteText(event.target.value)} placeholder="cole o convite recebido aqui" rows={5} /><button className="primary-button modal-primary" disabled={busy || !inviteText.trim()}>{busy ? "validando…" : "entrar no servidor"}</button></form><p className="modal-note">O convite é validado localmente e a chave do servidor fica protegida no armazenamento seguro do sistema.</p></section></div>}
    {modal === "create-channel" && activeGroup && <div className="modal-backdrop" onClick={() => setModal(null)}><section className="network-modal form-modal" onClick={(event) => event.stopPropagation()}><ModalHeader eyebrow={activeGroup.name.toUpperCase()} title="Criar canal" onClose={() => setModal(null)} /><form className="modal-form" onSubmit={createChannel}><label>nome do canal</label><input autoFocus value={newChannelName} onChange={(event) => setNewChannelName(event.target.value)} placeholder="ex.: jogos" maxLength={40} /><label>tipo</label><div className="preference-options"><button type="button" className={newChannelKind === "text" ? "selected" : ""} onClick={() => setNewChannelKind("text")}><Hash size={14} /> texto</button><button type="button" className={newChannelKind === "voice" ? "selected" : ""} onClick={() => setNewChannelKind("voice")}><Volume2 size={14} /> voz + chat + tela</button></div><button className="primary-button modal-primary" disabled={!newChannelName.trim()}>criar canal</button></form></section></div>}
    {modal === "settings" && activeGroup && <GroupSettings group={activeGroup} localPeerId={node.peer_id} onClose={() => setModal(null)} onChanged={async () => { await refreshGroups(); await refreshMembers(activeGroup.id); }} onDeleted={handleGroupDeleted} onFeedback={setFeedback} onError={setError} />}
    {modal === "user-settings" && <UserSettings preferences={preferences} onChange={handlePreferences} onClose={() => setModal(null)} />}
    {availableUpdate && <UpdateModal update={availableUpdate} onLater={dismissUpdate} />}
  </div>;
}

export default App;
