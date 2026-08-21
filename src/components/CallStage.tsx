import { forwardRef, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { Headphones, Mic, MicOff, MonitorUp, Phone, PhoneOff, Users, Volume2, VolumeX } from "lucide-react";
import { AUDIO_PREFERENCES_EVENT, readAudioPreferences, type AudioPreferences } from "../lib/audio";
import { MeshCall } from "../lib/webrtc";
import type { CallSignalEvent, CallState } from "../lib/tauri";

export type CallStageHandle = { toggleMute: () => Promise<void>; requestMute: (peerId: string) => Promise<void>; leave: () => Promise<void>; reannounce: () => Promise<void>; setInputVolume: (volume: number) => Promise<void> };

type CallStageProps = {
  groupId: string;
  channelId: string;
  localPeerId: string;
  signals: CallSignalEvent[];
  sharedState: CallState | null;
  deafened: boolean;
  muted: boolean;
  compact?: boolean;
  onToggleDeafened: () => void;
  onMutedChange: (muted: boolean) => void;
  onJoined: (state: CallState) => void;
  onLeft: () => void;
  onState: (state: CallState) => void;
  onError: (message: string) => void;
  displayName: string;
};

function StreamMedia({ stream, kind, muted, outputDeviceId, volume = 1 }: { stream: MediaStream; kind: "audio" | "video"; muted?: boolean; outputDeviceId?: string; volume?: number }) {
  const ref = useRef<HTMLMediaElement>(null);
  useEffect(() => {
    if (!ref.current) return;
    ref.current.srcObject = stream;
    ref.current.volume = volume;
    const sinkMedia = ref.current as HTMLMediaElement & { setSinkId?: (sinkId: string) => Promise<void> };
    if (outputDeviceId && typeof sinkMedia.setSinkId === "function") void sinkMedia.setSinkId(outputDeviceId).catch(() => undefined);
    void ref.current.play().catch(() => undefined);
  }, [stream, outputDeviceId, volume]);
  if (kind === "video") return <video ref={ref as React.RefObject<HTMLVideoElement>} autoPlay playsInline muted={muted} className="call-video" />;
  return <audio ref={ref as React.RefObject<HTMLAudioElement>} autoPlay controls={false} muted={muted} />;
}

export const CallStage = forwardRef<CallStageHandle, CallStageProps>(function CallStage({ groupId, channelId, localPeerId, signals, sharedState, deafened, muted, compact = false, onToggleDeafened, onMutedChange, onJoined, onLeft, onState, onError, displayName }, ref) {
  const callRef = useRef<MeshCall | null>(null);
  const signalRef = useRef<CallSignalEvent[]>([]);
  const [audioInputId, setAudioInputId] = useState(() => typeof window === "undefined" ? "" : localStorage.getItem("teamscord.audio-input") ?? "");
  const [audioInputs, setAudioInputs] = useState<MediaDeviceInfo[]>([]);
  const [state, setState] = useState<CallState | null>(sharedState);
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [screenStream, setScreenStream] = useState<MediaStream | null>(null);
  const [remoteStreams, setRemoteStreams] = useState<Record<string, MediaStream>>({});
  const [joined, setJoined] = useState(false);
  const [sharing, setSharing] = useState(false);
  const [starting, setStarting] = useState(false);
  const [audioPreferences, setAudioPreferences] = useState<AudioPreferences>(readAudioPreferences);
  const sharedParticipantIds = useRef<Set<string>>(new Set());

  useImperativeHandle(ref, () => ({
    toggleMute,
    requestMute: async (peerId: string) => {
      try { await callRef.current?.requestRemoteMute(peerId); } catch (error) { onError(`não foi possível solicitar mute: ${String(error)}`); }
    },
    leave,
    setInputVolume: async (volume: number) => { await callRef.current?.setInputVolume(volume); },
    reannounce: async () => { await callRef.current?.reannounce(); },
  }), [onMutedChange, onError]);

  useEffect(() => {
    const handleAudioPreferences = (event: Event) => {
      const next = (event as CustomEvent<AudioPreferences>).detail;
      if (!next) return;
      setAudioPreferences(next);
      setAudioInputId(next.input_device_id);
      void callRef.current?.setInputVolume(next.input_volume);
      if (joined && next.input_device_id !== audioInputId) void selectAudioInput(next.input_device_id);
    };
    window.addEventListener(AUDIO_PREFERENCES_EVENT, handleAudioPreferences);
    return () => window.removeEventListener(AUDIO_PREFERENCES_EVENT, handleAudioPreferences);
  }, [audioInputId, joined]);

  useEffect(() => { signalRef.current = signals; void Promise.all(signals.map((signal) => callRef.current?.handleSignal(signal))); }, [signals]);
  useEffect(() => {
    let disposed = false;
    const refreshDevices = async () => {
      if (!navigator.mediaDevices?.enumerateDevices) return;
      const devices = (await navigator.mediaDevices.enumerateDevices()).filter((device) => device.kind === "audioinput");
      if (disposed) return;
      setAudioInputs(devices);
      if (audioInputId && devices.some((device) => device.deviceId === audioInputId)) return;
      setAudioInputId(devices[0]?.deviceId ?? "");
    };
    void refreshDevices();
    navigator.mediaDevices?.addEventListener("devicechange", refreshDevices);
    return () => { disposed = true; navigator.mediaDevices?.removeEventListener("devicechange", refreshDevices); };
  }, [audioInputId]);
  useEffect(() => {
    if (sharedState?.group_id !== groupId || sharedState.channel_id !== channelId) return;
    const nextIds = new Set(sharedState.participants.map((participant) => participant.peer_id));
    for (const peerId of sharedParticipantIds.current) {
      if (peerId !== localPeerId && !nextIds.has(peerId)) callRef.current?.removePeer(peerId);
    }
    sharedParticipantIds.current = nextIds;
    setState(sharedState);
    if (callRef.current && joined && !nextIds.has(localPeerId)) void leave();
  }, [sharedState, groupId, channelId, localPeerId, joined]);
  useEffect(() => () => { void callRef.current?.close(); callRef.current = null; }, []);

  const participants = useMemo(() => state?.participants ?? [], [state]);
  const connection = !joined ? "não conectado" : participants.some((participant) => participant.connection_state === "failed" || participant.connection_state === "disconnected") ? "reconectando" : participants.some((participant) => participant.peer_id !== localPeerId && participant.connection_state === "connecting") ? "conectando" : "conectado";

  async function join() {
    if (callRef.current || !localPeerId) return;
    setStarting(true);
    const call = new MeshCall(groupId, channelId, localPeerId, {
      onLocalStream: setLocalStream,
      onScreenStream: (stream) => { setScreenStream(stream); setSharing(Boolean(stream)); },
      onRemoteStream: (peerId, stream) => setRemoteStreams((current) => { if (!stream) { const next = { ...current }; delete next[peerId]; return next; } return { ...current, [peerId]: stream }; }),
      onPeerState: (peerId, connectionState) => setState((current) => current ? ({ ...current, participants: current.participants.map((participant) => participant.peer_id === peerId ? { ...participant, connection_state: connectionState } : participant) }) : current),
      onState: (next) => { setState(next); onState(next); },
      onError,
    }, audioInputId, muted, audioPreferences.input_volume, displayName);
    callRef.current = call;
    try {
      const next = await call.start();
      setState(next);
      setJoined(true);
      onJoined(next);
      await Promise.all(signalRef.current.map((signal) => call.handleSignal(signal)));
    } catch {
      callRef.current = null;
      setJoined(false);
    } finally { setStarting(false); }
  }

  async function leave() {
    await callRef.current?.close();
    callRef.current = null;
    setJoined(false);
    setLocalStream(null);
    setScreenStream(null);
    setRemoteStreams({});
    setState(null);
    onLeft();
  }

  async function toggleMute() {
    try {
      const next = await callRef.current?.toggleMuted();
      if (typeof next === "boolean") onMutedChange(next);
    } catch (error) {
      onError(`não foi possível atualizar o microfone: ${String(error)}`);
    }
  }
  async function toggleScreen() {
    try {
      const next = await callRef.current?.toggleScreenShare();
      if (typeof next === "boolean") setSharing(next);
    } catch (error) {
      onError(`não foi possível atualizar o compartilhamento: ${String(error)}`);
    }
  }

  async function selectAudioInput(deviceId: string) {
    setAudioInputId(deviceId);
    localStorage.setItem("teamscord.audio-input", deviceId);
    try { await callRef.current?.setAudioInputDevice(deviceId); } catch (error) { onError(`não foi possível trocar o microfone: ${String(error)}`); }
  }

  if (compact) return <section className="call-dock" aria-label="Call ativa">
    <div className="call-dock-status"><span className="live-dot" /><div><strong>{connection}</strong><small>{participants.length} participante{participants.length === 1 ? "" : "s"} · {channelId.split(":").pop()}</small></div></div>
    <div className="call-dock-participants">{participants.slice(0, 8).map((participant) => <span key={participant.peer_id} title={participant.display_name} className={participant.connection_state === "connected" ? "connected" : ""}>{participant.display_name.slice(0, 2).toUpperCase()}</span>)}</div>
    <div className="call-dock-controls"><button className={muted ? "control-button danger" : "control-button"} onClick={() => void toggleMute()} title={muted ? "Ativar microfone" : "Desativar microfone"}>{muted ? <MicOff size={16} /> : <Mic size={16} />}</button><button className={deafened ? "control-button danger" : "control-button"} onClick={onToggleDeafened} title={deafened ? "Ativar áudio" : "Silenciar participantes"}>{deafened ? <VolumeX size={16} /> : <Headphones size={16} />}</button><button className="control-button leave" onClick={() => void leave()} title="Sair da call"><PhoneOff size={16} /></button></div>
  </section>;

  return <section className="call-stage real-call-stage">
    <div className="call-stage-header"><div><span className="eyebrow">CALL P2P · MÁXIMO 8</span><h2>{joined ? "conversa em andamento" : "entrar nesta conversa"}</h2><p>{participants.length} participante{participants.length === 1 ? "" : "s"} · chat, áudio e tela no mesmo canal</p></div><span className={`call-connection-pill ${joined ? "online" : "offline"}`}><i />{connection}</span></div>
    {!joined ? <div className="call-join-card"><div className="call-icon"><Volume2 size={29} /></div><h3>Áudio e tela entre os nodes</h3><p>Este canal tem chat próprio. Ao entrar, seus amigos verão sua presença aqui e a call continuará ativa enquanto você navega.</p><button className="primary-button" onClick={() => void join()} disabled={starting || !localPeerId}><Phone size={17} />{starting ? "conectando…" : "entrar na call"}</button><small><Headphones size={14} /> sem câmera · malha P2P</small></div> : <>
      <div className="call-grid">{screenStream && <div className="call-tile call-screen-tile"><StreamMedia stream={screenStream} kind="video" muted outputDeviceId={audioPreferences.output_device_id} volume={audioPreferences.output_volume} /><span className="call-tile-label"><MonitorUp size={14} /> sua tela</span></div>}{localStream && !screenStream && <div className="call-tile call-audio-tile"><div className="call-avatar">você</div><StreamMedia stream={localStream} kind="audio" muted /><span className="call-tile-label"><Mic size={14} /> {muted ? "mutado" : "você"}</span></div>}{Object.entries(remoteStreams).map(([peerId, stream]) => <div className="call-tile" key={peerId}>{stream.getVideoTracks().length > 0 ? <StreamMedia stream={stream} kind="video" muted /> : <div className="call-avatar">{peerId.slice(0, 6)}</div>}<StreamMedia stream={stream} kind="audio" muted={deafened} outputDeviceId={audioPreferences.output_device_id} volume={audioPreferences.output_volume} /><span className="call-tile-label"><i className="status-dot" />{participants.find((participant) => participant.peer_id === peerId)?.display_name ?? peerId.slice(0, 10)}</span></div>)}{Object.keys(remoteStreams).length === 0 && <div className="call-empty"><Users size={28} />Aguardando outros participantes…</div>}</div>
      <div className="call-participant-strip">{participants.map((participant) => <span className="call-participant" key={participant.peer_id}><i className={`status-dot ${participant.connection_state === "connected" ? "" : "status-dot-offline"}`} />{participant.display_name}{participant.muted && <MicOff size={13} />}{participant.sharing_screen && <MonitorUp size={13} />}</span>)}</div>
      <div className="call-controls"><button className={muted ? "control-button danger" : "control-button"} onClick={() => void toggleMute()} title={muted ? "Ativar microfone" : "Desativar microfone"}>{muted ? <MicOff size={18} /> : <Mic size={18} />}</button><button className={deafened ? "control-button danger" : "control-button"} onClick={onToggleDeafened} title={deafened ? "Ativar áudio" : "Silenciar participantes"} aria-label="silenciar participantes">{deafened ? <VolumeX size={18} /> : <Volume2 size={18} />}</button><label className="device-picker" title="Selecionar microfone"><Mic size={14} /><select aria-label="Selecionar microfone" value={audioInputId} onChange={(event) => void selectAudioInput(event.target.value)} disabled={!joined || !audioInputs.length}><option value="">microfone padrão</option>{audioInputs.map((device, index) => <option key={device.deviceId} value={device.deviceId}>{device.label || `microfone ${index + 1}`}</option>)}</select></label><button className={sharing ? "control-button active" : "control-button"} onClick={() => void toggleScreen()} title={sharing ? "Parar compartilhamento" : "Compartilhar tela"}><MonitorUp size={18} /></button><button className="control-button leave" onClick={() => void leave()} title="Sair da call"><PhoneOff size={18} /></button></div>
    </>}
  </section>;
});

export function CallDock({ state, channelName, muted, deafened, onToggleMute, onToggleDeafened, onLeave }: {
  state: CallState | null;
  channelName: string;
  muted: boolean;
  deafened: boolean;
  onToggleMute: () => void;
  onToggleDeafened: () => void;
  onLeave: () => void;
}) {
  const participants = state?.participants ?? [];
  const connection = participants.some((participant) => participant.connection_state === "failed" || participant.connection_state === "disconnected")
    ? "reconectando"
    : participants.some((participant) => participant.connection_state === "connecting") ? "conectando" : "conectado";
  return <div className="call-dock" aria-label={`Call ativa em ${channelName}`}>
    <span className={`call-dock-signal ${connection === "conectado" ? "online" : ""}`} />
    <div className="call-dock-status"><strong>{connection}</strong><small>{channelName} · {participants.length} participante{participants.length === 1 ? "" : "s"}</small></div>
    <div className="call-dock-participants">{participants.slice(0, 4).map((participant) => <span key={participant.peer_id} title={participant.display_name} className={participant.connection_state === "connected" ? "connected" : ""}>{participant.display_name.slice(0, 2).toUpperCase()}</span>)}{participants.length > 4 && <span>+{participants.length - 4}</span>}</div>
    <div className="call-dock-controls"><button className={muted ? "control-button danger" : "control-button"} onClick={onToggleMute} title={muted ? "Ativar microfone" : "Desativar microfone"} aria-label={muted ? "Ativar microfone" : "Desativar microfone"}>{muted ? <MicOff size={15} /> : <Mic size={15} />}</button><button className={deafened ? "control-button danger" : "control-button"} onClick={onToggleDeafened} title={deafened ? "Ativar áudio" : "Silenciar participantes"} aria-label={deafened ? "Ativar áudio" : "Silenciar participantes"}>{deafened ? <VolumeX size={15} /> : <Headphones size={15} />}</button><button className="control-button leave" onClick={onLeave} title="Sair da call" aria-label="Sair da call"><PhoneOff size={15} /></button></div>
  </div>;
}
