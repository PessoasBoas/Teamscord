import { nodeApi, type CallSignalEvent, type CallState, type IceServerConfig } from "./tauri";

const MAX_CALL_PARTICIPANTS = 8;

type CallCallbacks = {
  onLocalStream?: (stream: MediaStream | null) => void;
  onScreenStream?: (stream: MediaStream | null) => void;
  onRemoteStream?: (peerId: string, stream: MediaStream | null) => void;
  onPeerState?: (peerId: string, state: RTCPeerConnectionState) => void;
  onState?: (state: CallState) => void;
  onError?: (error: string) => void;
};

function iceConfiguration(config: IceServerConfig[]): RTCConfiguration {
  return { iceServers: config.map((server) => ({ urls: server.urls, username: server.username ?? undefined, credential: server.credential ?? undefined })) };
}

function signalId(event: CallSignalEvent) {
  return typeof event.signal.event_id === "string" ? event.signal.event_id : `${String(event.signal.from_peer_id)}:${event.body.kind}:${JSON.stringify(event.body.payload)}`;
}

export class MeshCall {
  private readonly peers = new Map<string, RTCPeerConnection>();
  private readonly remoteStreams = new Map<string, MediaStream>();
  private readonly pendingCandidates = new Map<string, RTCIceCandidateInit[]>();
  private readonly handledSignals = new Set<string>();
  private readonly negotiating = new Set<string>();
  private readonly pendingNegotiations = new Set<string>();
  private readonly pendingIceRestarts = new Set<string>();
  private readonly initialOfferers = new Set<string>();
  private localStream: MediaStream | null = null;
  private screenStream: MediaStream | null = null;
  private state: CallState | null = null;
  private mediaConfig: RTCConfiguration = {};
  private closed = false;
  private muted = false;
  private audioInputDeviceId: string | undefined;

  constructor(
    private readonly groupId: string,
    private readonly channelId: string,
    private readonly localPeerId: string,
    private readonly callbacks: CallCallbacks = {},
    audioInputDeviceId?: string,
    initialMuted = false,
  ) {
    this.audioInputDeviceId = audioInputDeviceId || undefined;
    this.muted = initialMuted;
  }

  async start(): Promise<CallState> {
    try {
      this.mediaConfig = iceConfiguration((await nodeApi.getMediaConfig()).ice_servers);
      if (!navigator.mediaDevices?.getUserMedia) throw new Error("WebRTC não está disponível neste WebView2");
      this.localStream = await navigator.mediaDevices.getUserMedia({ audio: this.audioInputDeviceId ? { deviceId: { exact: this.audioInputDeviceId } } : true, video: false });
      this.watchAudioTracks(this.localStream);
      this.localStream.getAudioTracks().forEach((track) => { track.enabled = !this.muted; });
      this.callbacks.onLocalStream?.(this.localStream);
      this.state = await nodeApi.joinCall(this.groupId, this.channelId);
      this.emitState();
      if (this.muted) await this.setMuted(true);
      return this.state;
    } catch (error) {
      const message = error instanceof DOMException ? `${error.name}: ${error.message}` : String(error);
      this.callbacks.onError?.(`não foi possível iniciar áudio: ${message}`);
      await this.close().catch(() => undefined);
      throw error;
    }
  }

  async handleSignal(event: CallSignalEvent): Promise<void> {
    if (this.closed || !this.state) return;
    const id = signalId(event);
    if (this.handledSignals.has(id)) return;
    this.handledSignals.add(id);
    const signal = event.signal as { group_id?: string; channel_id?: string; call_id?: string; from_peer_id?: string };
    if (signal.group_id !== this.groupId || signal.channel_id !== this.channelId || signal.call_id !== this.state.call_id) return;
    const fromPeerId = signal.from_peer_id;
    if (!fromPeerId || fromPeerId === this.localPeerId) return;
    const body = event.body;
    try {
      if (body.kind === "join") {
        await this.ensurePeer(fromPeerId, this.localPeerId < fromPeerId);
        return;
      }
      if (body.kind === "leave") {
        this.closePeer(fromPeerId);
        this.updateParticipant(fromPeerId, { connection_state: "disconnected" });
        return;
      }
      if (body.kind === "mute") {
        this.updateParticipant(fromPeerId, { muted: Boolean(body.payload.muted) });
        return;
      }
      if (body.kind === "mute-request") {
        await this.setMuted(Boolean(body.payload.muted));
        return;
      }
      if (body.kind === "screen-start" || body.kind === "screen-stop") {
        this.updateParticipant(fromPeerId, { sharing_screen: body.kind === "screen-start" });
        return;
      }
      if (body.kind === "peer-state") {
        const connectionState = body.payload.connection_state;
        if (connectionState === "new" || connectionState === "connecting" || connectionState === "connected" || connectionState === "disconnected" || connectionState === "failed" || connectionState === "closed") {
          this.updateParticipant(fromPeerId, { connection_state: connectionState });
        }
        return;
      }
      const peer = await this.ensurePeer(fromPeerId, false);
      if (body.kind === "offer") {
        const description = body.payload.description as RTCSessionDescriptionInit;
        const polite = this.localPeerId > fromPeerId;
        if (peer.signalingState !== "stable") {
          if (!polite) return;
          await peer.setLocalDescription({ type: "rollback" });
        }
        await peer.setRemoteDescription(description);
        this.pendingNegotiations.delete(fromPeerId);
        await this.flushCandidates(fromPeerId, peer);
        const answer = await peer.createAnswer();
        await peer.setLocalDescription(answer);
        await this.send(fromPeerId, "answer", { description: peer.localDescription });
      } else if (body.kind === "answer") {
        await peer.setRemoteDescription(body.payload.description as RTCSessionDescriptionInit);
        await this.flushCandidates(fromPeerId, peer);
        if (this.pendingNegotiations.has(fromPeerId)) {
          const iceRestart = this.pendingIceRestarts.delete(fromPeerId);
          void this.renegotiate(fromPeerId, peer, iceRestart);
        }
      } else if (body.kind === "ice") {
        const candidate = body.payload.candidate as RTCIceCandidateInit | undefined;
        if (!candidate) return;
        if (peer.remoteDescription) await peer.addIceCandidate(candidate);
        else this.pendingCandidates.set(fromPeerId, [...(this.pendingCandidates.get(fromPeerId) ?? []), candidate]);
      }
    } catch (error) {
      this.callbacks.onError?.(`falha na conexão de ${fromPeerId.slice(0, 10)}: ${String(error)}`);
    }
  }

  async setMuted(muted: boolean): Promise<void> {
    this.muted = muted;
    this.localStream?.getAudioTracks().forEach((track) => { track.enabled = !muted; });
    this.updateParticipant(this.localPeerId, { muted });
    if (this.state) await this.broadcast("mute", { muted });
  }

  async toggleMuted(): Promise<boolean> {
    const next = !this.muted;
    await this.setMuted(next);
    return next;
  }

  async requestRemoteMute(peerId: string, muted = true): Promise<void> {
    if (this.closed || !this.state) return;
    await nodeApi.requestCallMute(
      this.groupId,
      this.channelId,
      this.state.call_id,
      peerId,
      muted,
    );
  }

  async reannounce(): Promise<void> {
    if (this.closed || !this.state) return;
    const local = this.state.participants.find((participant) => participant.peer_id === this.localPeerId);
    await nodeApi.sendCallSignal(
      this.groupId,
      this.channelId,
      this.state.call_id,
      null,
      "join",
      { display_name: local?.display_name ?? "Membro" },
    );
  }

  async setAudioInputDevice(deviceId: string): Promise<void> {
    if (this.closed || !this.localStream || !navigator.mediaDevices?.getUserMedia) return;
    const nextStream = await navigator.mediaDevices.getUserMedia({ audio: deviceId ? { deviceId: { exact: deviceId } } : true, video: false });
    const nextTrack = nextStream.getAudioTracks()[0];
    if (!nextTrack) {
      nextStream.getTracks().forEach((track) => track.stop());
      throw new Error("o dispositivo selecionado não disponibilizou microfone");
    }
    nextTrack.enabled = !this.muted;
    this.watchAudioTrack(nextTrack);
    const previousTracks = this.localStream.getAudioTracks();
    await Promise.all([...this.peers.values()].map(async (peer) => {
      const sender = peer.getSenders().find((candidate) => candidate.track?.kind === "audio");
      if (sender) await sender.replaceTrack(nextTrack);
      else {
        peer.addTrack(nextTrack, nextStream);
        const peerId = [...this.peers.entries()].find(([, value]) => value === peer)?.[0];
        if (peerId) await this.renegotiate(peerId, peer);
      }
    }));
    this.localStream = nextStream;
    previousTracks.forEach((track) => track.stop());
    this.audioInputDeviceId = deviceId || undefined;
    this.callbacks.onLocalStream?.(nextStream);
  }

  async toggleScreenShare(): Promise<boolean> {
    if (this.screenStream) {
      this.stopScreenShare();
      return false;
    }
    if (this.state?.participants.some((participant) => participant.peer_id !== this.localPeerId && participant.sharing_screen)) {
      this.callbacks.onError?.("já existe outra tela compartilhada nesta call");
      return false;
    }
    try {
      if (!navigator.mediaDevices?.getDisplayMedia) throw new Error("captura de tela não está disponível neste WebView2");
      this.screenStream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
      this.callbacks.onScreenStream?.(this.screenStream);
      const tracks = this.screenStream.getTracks();
      await Promise.all([...this.peers.entries()].map(async ([peerId, peer]) => {
        tracks.forEach((track) => peer.addTrack(track, this.screenStream!));
        await this.renegotiate(peerId, peer);
      }));
      this.screenStream.getVideoTracks()[0]?.addEventListener("ended", () => this.stopScreenShare());
      this.updateParticipant(this.localPeerId, { sharing_screen: true });
      await this.broadcast("screen-start", { has_audio: this.screenStream.getAudioTracks().length > 0 });
      return true;
    } catch (error) {
      this.callbacks.onError?.(`compartilhamento de tela recusado ou indisponível: ${String(error)}`);
      return false;
    }
  }

  stopScreenShare(): void {
    if (!this.screenStream) return;
    const tracks = new Set(this.screenStream.getTracks());
    this.screenStream.getTracks().forEach((track) => track.stop());
    this.screenStream = null;
    this.callbacks.onScreenStream?.(null);
    this.updateParticipant(this.localPeerId, { sharing_screen: false });
    void Promise.all([...this.peers.entries()].map(async ([peerId, peer]) => {
      for (const sender of peer.getSenders()) if (sender.track && tracks.has(sender.track)) peer.removeTrack(sender);
      if (peer.connectionState !== "closed") await this.renegotiate(peerId, peer);
    })).catch((error) => this.callbacks.onError?.(`não foi possível encerrar a tela: ${String(error)}`));
    if (!this.closed) void this.broadcast("screen-stop", {}).catch((error) => this.callbacks.onError?.(`não foi possível anunciar o fim da tela: ${String(error)}`));
  }

  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;
    if (this.state) await nodeApi.leaveCall(this.groupId, this.channelId, this.state.call_id).catch(() => undefined);
    this.stopScreenShare();
    this.localStream?.getTracks().forEach((track) => track.stop());
    this.localStream = null;
    this.peers.forEach((peer) => peer.close());
    this.peers.clear();
    this.remoteStreams.clear();
    this.callbacks.onLocalStream?.(null);
    this.callbacks.onScreenStream?.(null);
    this.state = null;
  }

  getState(): CallState | null { return this.state; }

  removePeer(peerId: string): void {
    this.closePeer(peerId);
    if (this.state) {
      this.state = {
        ...this.state,
        participants: this.state.participants.filter((participant) => participant.peer_id !== peerId),
      };
      this.emitState();
    }
  }

  private async ensurePeer(peerId: string, offerer: boolean): Promise<RTCPeerConnection> {
    const existing = this.peers.get(peerId);
    if (existing) return existing;
    if (!this.state?.participants.some((participant) => participant.peer_id === peerId)
      && (this.state?.participants.length ?? 0) >= MAX_CALL_PARTICIPANTS) {
      throw new Error("esta call já atingiu o limite de 8 participantes");
    }
    if (offerer) this.initialOfferers.add(peerId);
    const peer = new RTCPeerConnection(this.mediaConfig);
    this.peers.set(peerId, peer);
    this.updateParticipant(peerId, { connection_state: "connecting" });
    this.localStream?.getTracks().forEach((track) => peer.addTrack(track, this.localStream!));
    this.screenStream?.getTracks().forEach((track) => peer.addTrack(track, this.screenStream!));
    peer.onnegotiationneeded = () => {
      if (!this.initialOfferers.has(peerId) && !peer.remoteDescription) {
        this.pendingNegotiations.add(peerId);
        return;
      }
      void this.renegotiate(peerId, peer);
    };
    peer.onicecandidate = (event) => { if (event.candidate) void this.send(peerId, "ice", { candidate: event.candidate.toJSON() }); };
    peer.ontrack = (event) => {
      const stream = this.remoteStreams.get(peerId) ?? new MediaStream();
      if (!stream.getTracks().some((track) => track.id === event.track.id)) stream.addTrack(event.track);
      this.remoteStreams.set(peerId, stream);
      event.track.addEventListener("ended", () => {
        stream.removeTrack(event.track);
        if (!stream.getTracks().length) {
          this.remoteStreams.delete(peerId);
          this.callbacks.onRemoteStream?.(peerId, null);
        } else this.callbacks.onRemoteStream?.(peerId, stream);
      });
      this.callbacks.onRemoteStream?.(peerId, stream);
    };
    peer.onconnectionstatechange = () => {
      this.callbacks.onPeerState?.(peerId, peer.connectionState);
      this.updateParticipant(peerId, { connection_state: peer.connectionState });
      void this.send(peerId, "peer-state", { connection_state: peer.connectionState });
      if (peer.connectionState === "failed") {
        this.callbacks.onError?.(`a conexão de ${peerId.slice(0, 10)} falhou; configure TURN se estiver fora da LAN`);
        peer.restartIce();
        void this.renegotiate(peerId, peer, true).catch((error) => this.callbacks.onError?.(`não foi possível reiniciar a conexão de ${peerId.slice(0, 10)}: ${String(error)}`));
      }
    };
    if (offerer) await this.renegotiate(peerId, peer);
    return peer;
  }

  private watchAudioTracks(stream: MediaStream) {
    stream.getAudioTracks().forEach((track) => this.watchAudioTrack(track));
  }

  private watchAudioTrack(track: MediaStreamTrack) {
    track.addEventListener("ended", () => {
      if (!this.closed && this.localStream?.getAudioTracks().some((candidate) => candidate.id === track.id)) {
        this.callbacks.onError?.("o microfone foi desconectado; selecione outro dispositivo na call");
      }
    }, { once: true });
  }

  private async renegotiate(peerId: string, peer: RTCPeerConnection, iceRestart = false): Promise<void> {
    if (this.closed) return;
    if (this.negotiating.has(peerId) || peer.signalingState !== "stable") {
      this.pendingNegotiations.add(peerId);
      if (iceRestart) this.pendingIceRestarts.add(peerId);
      return;
    }
    this.pendingNegotiations.delete(peerId);
    this.pendingIceRestarts.delete(peerId);
    this.negotiating.add(peerId);
    try {
      const offer = await peer.createOffer(iceRestart ? { iceRestart: true } : undefined);
      await peer.setLocalDescription(offer);
      await this.send(peerId, "offer", { description: peer.localDescription });
    } finally {
      this.negotiating.delete(peerId);
      if (this.pendingNegotiations.has(peerId) && peer.signalingState === "stable") {
        const restart = this.pendingIceRestarts.delete(peerId);
        void this.renegotiate(peerId, peer, restart);
      }
    }
  }

  private async flushCandidates(peerId: string, peer: RTCPeerConnection) {
    const candidates = this.pendingCandidates.get(peerId) ?? [];
    this.pendingCandidates.delete(peerId);
    await Promise.all(candidates.map((candidate) => peer.addIceCandidate(candidate)));
  }

  private updateParticipant(peerId: string, patch: Partial<CallState["participants"][number]>) {
    if (!this.state) return;
    const existing = this.state.participants.find((participant) => participant.peer_id === peerId);
    if (!existing && this.state.participants.length >= MAX_CALL_PARTICIPANTS) {
      this.callbacks.onError?.("esta call já atingiu o limite de 8 participantes");
      return;
    }
    if (existing) this.state = { ...this.state, participants: this.state.participants.map((participant) => participant.peer_id === peerId ? { ...participant, ...patch } : participant) };
    else this.state = { ...this.state, participants: [...this.state.participants, { peer_id: peerId, display_name: peerId.slice(0, 10), role: "member", muted: false, sharing_screen: false, connection_state: "connecting", ...patch }] };
    this.emitState();
  }

  private emitState() { if (this.state) this.callbacks.onState?.(this.state); }
  private closePeer(peerId: string) {
    this.peers.get(peerId)?.close();
    this.peers.delete(peerId);
    this.pendingCandidates.delete(peerId);
    this.pendingNegotiations.delete(peerId);
    this.pendingIceRestarts.delete(peerId);
    this.initialOfferers.delete(peerId);
    this.remoteStreams.delete(peerId);
    this.callbacks.onRemoteStream?.(peerId, null);
  }
  private async broadcast(kind: string, payload: Record<string, unknown>) { await Promise.all([...this.peers.keys()].map((peerId) => this.send(peerId, kind, payload))); }
  private async send(peerId: string, kind: string, payload: Record<string, unknown>) { if (this.state) await nodeApi.sendCallSignal(this.groupId, this.channelId, this.state.call_id, peerId, kind, payload); }
}
