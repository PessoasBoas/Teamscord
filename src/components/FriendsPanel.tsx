import { FormEvent, useEffect, useMemo, useState } from "react";
import { Check, Copy, MessageCircle, QrCode, Send, ShieldBan, Upload, UserPlus, UserRoundX, X } from "lucide-react";
import { type ContactCard, type DirectMessage, type Friend, isDesktop, nodeApi } from "../lib/tauri";
import { qrDataUrl, readQrImage } from "../lib/qr";

type Props = { displayName: string; mode?: "modal" | "page"; onClose: () => void; onError: (message: string) => void; onFeedback: (message: string) => void };

function shortPeer(peerId: string) { return `${peerId.slice(0, 12)}…${peerId.slice(-6)}`; }

export function FriendsPanel({ displayName, mode = "modal", onClose, onError, onFeedback }: Props) {
  const [friends, setFriends] = useState<Friend[]>([]);
  const [card, setCard] = useState<ContactCard | null>(null);
  const [cardQr, setCardQr] = useState("");
  const [contactText, setContactText] = useState("");
  const [selectedPeer, setSelectedPeer] = useState("");
  const [messages, setMessages] = useState<DirectMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);

  const selected = friends.find((friend) => friend.peer_id === selectedPeer) ?? null;
  const pending = useMemo(() => friends.filter((friend) => friend.status === "pending"), [friends]);
  const accepted = useMemo(() => friends.filter((friend) => friend.status === "accepted"), [friends]);

  async function refresh() {
    if (!isDesktop()) return;
    try { setFriends(await nodeApi.listFriends()); }
    catch (reason) { onError(String(reason)); }
  }

  useEffect(() => {
    if (!isDesktop()) return;
    void refresh();
    void nodeApi.getContactCard(displayName).then((value) => { setCard(value); return qrDataUrl(value.encoded, 280); }).then(setCardQr).catch((reason) => onError(String(reason)));
  }, [displayName]);
  useEffect(() => {
    if (!selectedPeer || !isDesktop()) { setMessages([]); return; }
    void nodeApi.getDirectMessages(selectedPeer).then(setMessages).catch((reason) => onError(String(reason)));
  }, [selectedPeer]);

  async function copy(value: string, message: string) {
    try { await navigator.clipboard.writeText(value); onFeedback(message); }
    catch (reason) { onError(String(reason)); }
  }

  async function addFriend(event: FormEvent) {
    event.preventDefault();
    if (!contactText.trim()) return;
    setBusy(true);
    try { await nodeApi.createFriendRequest(contactText.trim(), displayName); setContactText(""); await refresh(); onFeedback("solicitação de amizade enviada"); }
    catch (reason) { onError(String(reason)); }
    finally { setBusy(false); }
  }

  async function respond(peerId: string, response: "accept" | "reject" | "block") {
    setBusy(true);
    try { await nodeApi.respondFriendRequest(peerId, response, displayName); await refresh(); onFeedback(response === "accept" ? "amizade aceita" : response === "block" ? "contato bloqueado" : "solicitação recusada"); }
    catch (reason) { onError(String(reason)); }
    finally { setBusy(false); }
  }

  async function send(event: FormEvent) {
    event.preventDefault();
    if (!selectedPeer || !draft.trim()) return;
    setBusy(true);
    try { const message = await nodeApi.sendDirectMessage(selectedPeer, draft, displayName); setMessages((items) => [...items, message]); setDraft(""); }
    catch (reason) { onError(String(reason)); }
    finally { setBusy(false); }
  }

  async function importQr(file: File | undefined) {
    if (!file) return;
    try { setContactText(await readQrImage(file)); onFeedback("QR lido; confirme o envio da solicitação"); }
    catch (reason) { onError(String(reason)); }
  }

  const content = <section className={`friends-modal ${mode === "page" ? "friends-page" : ""}`} onClick={(event) => event.stopPropagation()}>
      <header className="friends-header"><div><span className="eyebrow">CENTRAL DE CONTATOS</span><h2>Amigos e mensagens privadas</h2><p>Identidade local, pedidos assinados e conversa cifrada entre peers.</p></div><button className="icon-button" onClick={onClose} aria-label="Fechar"><X size={19} /></button></header>
      <div className="friends-layout">
        <aside className="friends-sidebar">
          <div className="contact-card-preview">
            <div className="friends-section-title"><span>seu cartão</span><QrCode size={15} /></div>
            {cardQr ? <img className="contact-qr" src={cardQr} alt="QR do seu cartão de contato" /> : <div className="contact-qr-placeholder">gerando QR…</div>}
            <strong>{displayName}</strong><code>{card ? shortPeer(card.peer_id) : "node offline"}</code>
            {card && <button className="secondary-button" onClick={() => void copy(card.encoded, "código pessoal copiado")}><Copy size={14} /> copiar código pessoal</button>}
          </div>
          <form className="add-contact-form" onSubmit={addFriend}>
            <label>adicionar por código ou QR</label>
            <textarea value={contactText} onChange={(event) => setContactText(event.target.value)} rows={3} placeholder="cole teamscord://contact/v1/..." />
            <div className="inline-actions"><button type="submit" className="primary-button" disabled={busy || !contactText.trim()}><UserPlus size={14} /> enviar pedido</button><label className="file-button" title="Importar QR"><Upload size={14} /><input type="file" accept="image/*" onChange={(event) => void importQr(event.target.files?.[0])} /> QR</label></div>
          </form>
          {pending.length > 0 && <div className="pending-requests"><div className="friends-section-title"><span>pedidos pendentes</span><b>{pending.length}</b></div>{pending.map((friend) => <article className="friend-row pending" key={friend.peer_id}><span className="avatar small avatar-purple">{friend.display_name.slice(0, 2).toUpperCase()}</span><div><strong>{friend.display_name}</strong><small>{shortPeer(friend.peer_id)}</small></div><div className="friend-actions"><button onClick={() => void respond(friend.peer_id, "accept")} aria-label="Aceitar amizade"><Check size={14} /></button><button onClick={() => void respond(friend.peer_id, "reject")} aria-label="Recusar amizade"><UserRoundX size={14} /></button></div></article>)}</div>}
          <div className="friends-section-title accepted-title"><span>amigos</span><b>{accepted.length}</b></div>
          <div className="friends-list">{accepted.length ? accepted.map((friend) => <button className={`friend-row ${selectedPeer === friend.peer_id ? "active" : ""}`} key={friend.peer_id} onClick={() => setSelectedPeer(friend.peer_id)}><span className="avatar small avatar-purple"><span className={`presence ${friend.online ? "online" : "idle"}`} />{friend.display_name.slice(0, 2).toUpperCase()}</span><div><strong>{friend.display_name}</strong><small>{friend.online ? "online" : "offline"}</small></div><MessageCircle size={14} /></button>) : <div className="friends-empty">Nenhuma amizade aceita ainda.</div>}</div>
        </aside>
        <main className="direct-chat">
          {selected ? <><header className="direct-chat-header"><span className="avatar avatar-purple">{selected.display_name.slice(0, 2).toUpperCase()}</span><div><strong>{selected.display_name}</strong><small>{selected.online ? "online · conexão P2P disponível" : "offline · mensagens ficam na outbox"}</small></div><button className="icon-button danger-icon" onClick={() => void respond(selected.peer_id, "block")} title="Bloquear contato"><ShieldBan size={16} /></button></header><div className="direct-messages">{messages.length ? messages.map((message) => <article className={`direct-message ${message.mine ? "mine" : ""}`} key={message.id}><span className="avatar tiny avatar-purple">{message.author.slice(0, 2).toUpperCase()}</span><div><div className="direct-message-meta"><strong>{message.author}</strong><time>{new Date(message.created_at).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit" })}</time></div><p>{message.content}</p></div></article>) : <div className="direct-empty"><MessageCircle size={28} /><strong>Comece uma conversa privada</strong><span>As mensagens são cifradas para este contato e permanecem locais.</span></div>}</div><form className="direct-composer" onSubmit={send}><input value={draft} onChange={(event) => setDraft(event.target.value)} placeholder={`mensagem para ${selected.display_name}`} /><button className="send-button" disabled={busy || !draft.trim()} aria-label="Enviar mensagem"><Send size={16} /></button></form></> : <div className="direct-empty full"><MessageCircle size={34} /><strong>Escolha um amigo</strong><span>Adicione contatos por código ou QR para iniciar mensagens privadas.</span></div>}
        </main>
      </div>
    </section>;

  return mode === "page" ? <>{content}</> : <div className="modal-backdrop friends-backdrop" onClick={onClose}>{content}</div>;
}
