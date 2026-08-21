import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MeshCall } from "./webrtc";
import { nodeApi, type CallSignalEvent, type CallState } from "./tauri";

vi.mock("./tauri", () => ({
  nodeApi: {
    getMediaConfig: vi.fn(),
    joinCall: vi.fn(),
    leaveCall: vi.fn(),
    requestCallMute: vi.fn(),
    sendCallSignal: vi.fn(),
  },
}));

class FakeTrack extends EventTarget {
  readonly id: string;
  readonly kind: "audio" | "video";
  enabled = true;
  stopped = false;

  constructor(kind: "audio" | "video", id: string) {
    super();
    this.kind = kind;
    this.id = id;
  }

  stop() {
    this.stopped = true;
  }
}

class FakeMediaStream {
  private readonly tracks: FakeTrack[];

  constructor(tracks: FakeTrack[] = []) {
    this.tracks = [...tracks];
  }

  getTracks() { return [...this.tracks]; }
  getAudioTracks() { return this.tracks.filter((track) => track.kind === "audio"); }
  getVideoTracks() { return this.tracks.filter((track) => track.kind === "video"); }
  addTrack(track: FakeTrack) { if (!this.tracks.includes(track)) this.tracks.push(track); }
  removeTrack(track: FakeTrack) { const index = this.tracks.indexOf(track); if (index >= 0) this.tracks.splice(index, 1); }
}

class FakePeerConnection {
  static instances: FakePeerConnection[] = [];
  readonly config: RTCConfiguration;
  readonly offerOptions: Array<RTCOfferOptions | undefined> = [];
  readonly senders: Array<{ track: FakeTrack | null; replaceTrack: (track: FakeTrack) => Promise<void> }> = [];
  signalingState: RTCSignalingState = "stable";
  connectionState: RTCPeerConnectionState = "new";
  remoteDescription: RTCSessionDescriptionInit | null = null;
  localDescription: RTCSessionDescriptionInit | null = null;
  onnegotiationneeded: (() => void) | null = null;
  onicecandidate: ((event: { candidate: { toJSON: () => RTCIceCandidateInit } | null }) => void) | null = null;
  ontrack: ((event: { track: FakeTrack }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;

  constructor(config: RTCConfiguration) {
    this.config = config;
    FakePeerConnection.instances.push(this);
  }

  addTrack(track: FakeTrack) {
    const sender = {
      track,
      replaceTrack: async (nextTrack: FakeTrack) => { sender.track = nextTrack; },
    };
    this.senders.push(sender);
    return sender;
  }

  getSenders() { return [...this.senders]; }

  removeTrack(sender: { track: FakeTrack | null }) {
    sender.track = null;
  }

  async createOffer(options?: RTCOfferOptions): Promise<RTCSessionDescriptionInit> {
    this.offerOptions.push(options);
    return { type: "offer", sdp: "fake-offer" };
  }

  async createAnswer(): Promise<RTCSessionDescriptionInit> {
    return { type: "answer", sdp: "fake-answer" };
  }

  async setLocalDescription(description: RTCSessionDescriptionInit) {
    this.localDescription = description;
    this.signalingState = description.type === "offer" ? "have-local-offer" : "stable";
  }

  async setRemoteDescription(description: RTCSessionDescriptionInit) {
    this.remoteDescription = description;
    this.signalingState = description.type === "offer" ? "have-remote-offer" : "stable";
  }

  async addIceCandidate() {}

  restartIce() {}

  close() {
    this.connectionState = "closed";
    this.onconnectionstatechange?.();
  }
}

type TestNavigator = { mediaDevices: { getUserMedia: ReturnType<typeof vi.fn>; getDisplayMedia: ReturnType<typeof vi.fn> } };

const localCallState: CallState = {
  group_id: "group-1",
  channel_id: "group-1:lounge",
  call_id: "call-1",
  participants: [{
    peer_id: "local",
    display_name: "Local",
    role: "member",
    muted: false,
    sharing_screen: false,
    connection_state: "connected",
  }],
};

function signal(body: CallSignalEvent["body"], fromPeerId = "remote", eventId: string = crypto.randomUUID()): CallSignalEvent {
  return {
    signal: {
      event_id: eventId,
      group_id: "group-1",
      channel_id: "group-1:lounge",
      call_id: "call-1",
      from_peer_id: fromPeerId,
    },
    body,
  };
}

describe("MeshCall", () => {
  let navigatorStub: TestNavigator;
  let microphone: FakeTrack;
  let nextMicrophone: FakeTrack;
  let displayStream: FakeMediaStream;

  beforeEach(() => {
    vi.clearAllMocks();
    FakePeerConnection.instances = [];
    microphone = new FakeTrack("audio", "microphone-1");
    nextMicrophone = new FakeTrack("audio", "microphone-2");
    displayStream = new FakeMediaStream([
      new FakeTrack("video", "screen-1"),
      new FakeTrack("audio", "system-audio-1"),
    ]);
    navigatorStub = {
      mediaDevices: {
        getUserMedia: vi.fn().mockResolvedValue(new FakeMediaStream([microphone])),
        getDisplayMedia: vi.fn().mockResolvedValue(displayStream),
      },
    };
    vi.stubGlobal("navigator", navigatorStub);
    vi.stubGlobal("MediaStream", FakeMediaStream);
    vi.stubGlobal("RTCPeerConnection", FakePeerConnection);
    vi.mocked(nodeApi.getMediaConfig).mockResolvedValue({ ice_servers: [{ urls: ["stun:example.test:3478"] }] });
    vi.mocked(nodeApi.joinCall).mockResolvedValue(localCallState);
    vi.mocked(nodeApi.leaveCall).mockResolvedValue(undefined);
    vi.mocked(nodeApi.requestCallMute).mockResolvedValue(undefined);
    vi.mocked(nodeApi.sendCallSignal).mockResolvedValue(undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("inicia microfone, alterna mute, troca dispositivo e fecha a call", async () => {
    const onLocalStream = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onLocalStream }, undefined, false, 0.75, "Sheldon");

    await call.start();
    expect(navigatorStub.mediaDevices.getUserMedia).toHaveBeenCalledWith({ audio: true, video: false });
    expect(vi.mocked(nodeApi.joinCall)).toHaveBeenCalledWith("group-1", "group-1:lounge", undefined, "Sheldon");
    expect(onLocalStream).toHaveBeenCalledWith(expect.any(FakeMediaStream));
    expect(microphone.enabled).toBe(true);

    const onError = vi.fn();
    const watchedCall = new MeshCall("group-1", "group-1:lounge", "local", { onError });
    await watchedCall.start();
    microphone.dispatchEvent(new Event("ended"));
    expect(onError).toHaveBeenCalledWith("o microfone foi desconectado; selecione outro dispositivo na call");
    await watchedCall.close();

    await call.toggleMuted();
    expect(microphone.enabled).toBe(false);
    navigatorStub.mediaDevices.getUserMedia.mockResolvedValueOnce(new FakeMediaStream([nextMicrophone]));
    await call.setAudioInputDevice("microphone-2");
    expect(navigatorStub.mediaDevices.getUserMedia).toHaveBeenLastCalledWith({ audio: { deviceId: { exact: "microphone-2" } }, video: false });
    expect(microphone.stopped).toBe(true);
    expect(nextMicrophone.stopped).toBe(false);

    await call.close();
    expect(vi.mocked(nodeApi.leaveCall)).toHaveBeenCalledWith("group-1", "group-1:lounge", "call-1");
    expect(nextMicrophone.stopped).toBe(true);
  });

  it("compartilha tela com áudio, impede segunda tela e trata permissão recusada", async () => {
    const onScreenStream = vi.fn();
    const onError = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onScreenStream, onError });
    await call.start();

    await call.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }, "remote", "screen-peer-join"));
    const peer = FakePeerConnection.instances[0];
    await expect(call.toggleScreenShare()).resolves.toBe(true);
    expect(navigatorStub.mediaDevices.getDisplayMedia).toHaveBeenCalledWith({ video: true, audio: true });
    expect(call.getState()?.participants[0].sharing_screen).toBe(true);
    expect(peer.getSenders().map((sender) => sender.track?.kind)).toEqual(expect.arrayContaining(["audio", "video"]));
    expect(vi.mocked(nodeApi.sendCallSignal)).toHaveBeenCalledWith(
      "group-1", "group-1:lounge", "call-1", "remote", "screen-start", expect.any(Object),
    );
    call.stopScreenShare();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(call.getState()?.participants[0].sharing_screen).toBe(false);
    expect(peer.getSenders().some((sender) => sender.track?.kind === "video")).toBe(false);
    expect(onScreenStream).toHaveBeenLastCalledWith(null);

    await call.handleSignal(signal({ kind: "screen-start", payload: { has_audio: true } }, "remote", "screen-start-remote"));
    expect(await call.toggleScreenShare()).toBe(false);
    expect(onError).toHaveBeenCalledWith("já existe outra tela compartilhada nesta call");
    await call.handleSignal(signal({ kind: "screen-stop", payload: {} }, "remote", "screen-stop-remote"));
    navigatorStub.mediaDevices.getDisplayMedia.mockRejectedValueOnce(new DOMException("blocked", "NotAllowedError"));
    expect(await call.toggleScreenShare()).toBe(false);
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("compartilhamento de tela recusado ou indisponível"));
    await call.close();
  });

  it("encerra automaticamente o compartilhamento quando a captura da tela termina", async () => {
    const onScreenStream = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onScreenStream });
    await call.start();
    await call.toggleScreenShare();

    displayStream.getVideoTracks()[0].dispatchEvent(new Event("ended"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(call.getState()?.participants[0].sharing_screen).toBe(false);
    expect(displayStream.getTracks().every((track) => track.stopped)).toBe(true);
    expect(onScreenStream).toHaveBeenLastCalledWith(null);
    await call.close();
  });

  it("fecha a call com tela ativa sem publicar sinal depois do leave", async () => {
    const onError = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onError });
    await call.start();
    await call.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }, "remote", "close-screen-peer"));
    await call.toggleScreenShare();
    vi.mocked(nodeApi.sendCallSignal).mockClear();
    await call.close();
    expect(vi.mocked(nodeApi.sendCallSignal)).not.toHaveBeenCalledWith(
      "group-1", "group-1:lounge", "call-1", "remote", "screen-stop", expect.any(Object),
    );
    expect(onError).not.toHaveBeenCalled();
  });

  it("negocia offer/answer/ICE, mantém estado remoto e envia solicitação de mute", async () => {
    const onState = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onState });
    await call.start();

    await call.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }));
    expect(call.getState()?.participants.some((participant) => participant.peer_id === "remote")).toBe(true);
    expect(FakePeerConnection.instances).toHaveLength(1);
    expect(vi.mocked(nodeApi.sendCallSignal)).toHaveBeenCalledWith(
      "group-1", "group-1:lounge", "call-1", "remote", "offer", expect.any(Object),
    );

    const answerer = new MeshCall("group-1", "group-1:lounge", "zzz", { onState });
    vi.mocked(nodeApi.joinCall).mockResolvedValueOnce({ ...localCallState, participants: [{ ...localCallState.participants[0], peer_id: "zzz" }] });
    await answerer.start();
    await answerer.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }, "aaa", "join-aaa"));
    await answerer.handleSignal(signal({ kind: "offer", payload: { description: { type: "offer", sdp: "remote-offer" } } }, "aaa", "offer-aaa"));
    await answerer.handleSignal(signal({ kind: "ice", payload: { candidate: { candidate: "candidate:1" } } }, "aaa", "ice-aaa"));
    expect(vi.mocked(nodeApi.sendCallSignal)).toHaveBeenCalledWith(
      "group-1", "group-1:lounge", "call-1", "aaa", "answer", expect.any(Object),
    );

    await call.requestRemoteMute("remote");
    expect(vi.mocked(nodeApi.requestCallMute)).toHaveBeenCalledWith("group-1", "group-1:lounge", "call-1", "remote", true);
    await answerer.close();
    await call.close();
  });

  it("aplica o estado de conexão remoto sem criar uma peer connection desnecessária", async () => {
    const onState = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onState });
    await call.start();

    await call.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }, "remote", "join-state"));
    expect(FakePeerConnection.instances).toHaveLength(1);

    await call.handleSignal(signal({ kind: "peer-state", payload: { connection_state: "connected" } }, "remote", "state-connected"));
    expect(call.getState()?.participants.find((participant) => participant.peer_id === "remote")?.connection_state).toBe("connected");
    expect(FakePeerConnection.instances).toHaveLength(1);

    await call.handleSignal(signal({ kind: "peer-state", payload: { connection_state: "invalid" } }, "remote", "state-invalid"));
    expect(call.getState()?.participants.find((participant) => participant.peer_id === "remote")?.connection_state).toBe("connected");
    await call.close();
  });

  it("expõe falha de conexão e orienta configurar TURN", async () => {
    const onPeerState = vi.fn();
    const onError = vi.fn();
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onPeerState, onError });
    await call.start();
    await call.handleSignal(signal({ kind: "join", payload: { display_name: "Remoto" } }, "remote", "join-failed"));

    const peer = FakePeerConnection.instances[0];
    peer.connectionState = "failed";
    peer.signalingState = "stable";
    peer.onconnectionstatechange?.();
    expect(onPeerState).toHaveBeenCalledWith("remote", "failed");
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("configure TURN"));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(peer.offerOptions.some((options) => options?.iceRestart === true)).toBe(true);
    expect(call.getState()?.participants.find((participant) => participant.peer_id === "remote")?.connection_state).toBe("failed");
    await call.close();
  });

  it("converte falha de microfone em erro compreensível", async () => {
    const onError = vi.fn();
    navigatorStub.mediaDevices.getUserMedia.mockRejectedValueOnce(new DOMException("blocked", "NotAllowedError"));
    const call = new MeshCall("group-1", "group-1:lounge", "local", { onError });

    await expect(call.start()).rejects.toThrow("blocked");
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("não foi possível iniciar áudio: NotAllowedError"));
    expect(vi.mocked(nodeApi.joinCall)).not.toHaveBeenCalled();
  });
});
