import type { FormEvent } from "react";
import { Activity, Check, CircleAlert, Copy, Plus, RefreshCw, ShieldCheck, Wifi } from "lucide-react";
import type { NetworkDiagnostics, NodeSnapshot } from "../lib/tauri";
import { ModalHeader } from "./Ui";

export function NetworkModal({ node, diagnostics, checkingDiagnostics, runDiagnostics, peerAddress, setPeerAddress, connectPeer, relayAddress, setRelayAddress, addRelay, bootstrapAddress, setBootstrapAddress, addBootstrap, mediaConfigText, setMediaConfigText, saveMediaConfig, copied, copyPeerId, onClose }: {
  node: NodeSnapshot;
  diagnostics: NetworkDiagnostics | null;
  checkingDiagnostics: boolean;
  runDiagnostics: () => void;
  peerAddress: string;
  setPeerAddress: (value: string) => void;
  connectPeer: (event: FormEvent) => void;
  relayAddress: string;
  setRelayAddress: (value: string) => void;
  addRelay: (event: FormEvent) => void;
  bootstrapAddress: string;
  setBootstrapAddress: (value: string) => void;
  addBootstrap: (event: FormEvent) => void;
  mediaConfigText: string;
  setMediaConfigText: (value: string) => void;
  saveMediaConfig: (event: FormEvent) => void;
  copied: boolean;
  copyPeerId: () => void;
  onClose: () => void;
}) {
  return <div className="modal-backdrop" onClick={onClose}>
    <section className="network-modal" onClick={(event) => event.stopPropagation()}>
      <ModalHeader eyebrow="REDE PRIVADA" title="Seu node" onClose={onClose} />
      <div className="identity-panel"><div className="identity-icon"><ShieldCheck size={25} /></div><div><span>peer id</span><strong>{node.peer_id || "iniciando…"}</strong></div><button className="icon-button" onClick={copyPeerId} title="Copiar peer id">{copied ? <Check size={17} /> : <Copy size={17} />}</button></div>
      <div className="network-stats"><div><span>estado do node</span><strong><i className={`status-dot ${node.is_running ? "" : "status-dot-offline"}`} />{node.is_running ? "online" : "offline"}</strong></div><div><span>peers conectados</span><strong>{node.connected_peers}</strong></div><div><span>relay</span><strong><i className={`status-dot ${node.relay_connected ? "" : "status-dot-offline"}`} />{node.relay_connected ? "conectado" : "direto / aguardando"}</strong></div></div>
      <div className={`diagnostic-panel diagnostic-${diagnostics?.status ?? "waiting"}`}><div className="diagnostic-heading"><div><span className="eyebrow">VERIFICAÇÃO P2P</span><strong>{diagnostics?.summary ?? "verificando o caminho entre nodes…"}</strong></div><button className="icon-button" onClick={runDiagnostics} disabled={checkingDiagnostics} title="Verificar conexão agora" aria-label="Verificar conexão agora"><RefreshCw size={15} className={checkingDiagnostics ? "spin" : ""} /></button></div>{diagnostics?.checks.map((check) => <div className="diagnostic-check" key={check.name}><span className={`diagnostic-check-${check.status}`}>{check.status === "ok" ? <Check size={13} /> : <CircleAlert size={13} />}</span><div><strong>{check.name === "sync-probe" ? "sincronização" : check.name === "recovery" ? "recuperação" : check.name}</strong><small>{check.detail}</small></div></div>)}<small className="diagnostic-footnote"><Activity size={12} /> automático a cada 15s · último teste {diagnostics ? new Date(diagnostics.checked_at).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" }) : "—"}</small></div>
      <div className="listen-addresses"><span>endereços anunciados</span>{node.listen_addresses.length ? node.listen_addresses.map((address) => <code key={address}>{address}</code>) : <code>aguardando endereço discável…</code>}<small>estes endereços já são filtrados para não expor 0.0.0.0.</small></div>
      <div className="relay-config"><span>relays configurados</span>{node.relay_addresses.length ? node.relay_addresses.map((address) => <code key={address}>{address}</code>) : <small>nenhum relay — conexão direta continua disponível</small>}<form className="connect-form" onSubmit={addRelay}><label>adicionar relay</label><div><input value={relayAddress} onChange={(event) => setRelayAddress(event.target.value)} placeholder="/ip4/SEU_IP/tcp/4001/p2p/..." /><button type="submit" className="connect-button"><Plus size={15} /> salvar</button></div></form></div>
      <div className="relay-config"><span>bootstraps configurados</span>{node.bootstrap_addresses.length ? node.bootstrap_addresses.map((address) => <code key={address}>{address}</code>) : <small>nenhum bootstrap — use o endereço de um node conhecido</small>}<form className="connect-form" onSubmit={addBootstrap}><label>adicionar bootstrap</label><div><input value={bootstrapAddress} onChange={(event) => setBootstrapAddress(event.target.value)} placeholder="/ip4/192.168.0.12/tcp/4001/p2p/..." /><button type="submit" className="connect-button"><Plus size={15} /> salvar</button></div></form></div>
      <form className="connect-form" onSubmit={saveMediaConfig}><label>ICE para WebRTC · lista JSON</label><textarea className="ice-config" value={mediaConfigText} onChange={(event) => setMediaConfigText(event.target.value)} rows={4} placeholder={'[{"urls":["stun:stun.example.com:3478"]}'} /><button type="submit" className="connect-button">salvar STUN/TURN</button></form>
      <form className="connect-form" onSubmit={connectPeer}><label>conectar agora a outro node</label><div><input value={peerAddress} onChange={(event) => setPeerAddress(event.target.value)} placeholder="/ip4/192.168.0.12/tcp/4001/p2p/..." /><button type="submit" className="connect-button"><Wifi size={15} /> conectar</button></div></form>
      <p className="modal-note">Bootstrap inicia conexões automaticamente; relay libp2p e TURN são opcionais. Credenciais ICE ficam no armazenamento seguro do Windows e nunca no convite.</p>
    </section>
  </div>;
}
