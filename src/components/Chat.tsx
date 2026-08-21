import type { FormEvent, MouseEvent as ReactMouseEvent } from "react";
import { Hash, Plus, Send, ShieldCheck, Smile, Volume2 } from "lucide-react";
import type { Channel, ChatMessage } from "../lib/tauri";
import { isDesktop } from "../lib/tauri";

export function ChatPanel({ channel, messages, loading, draft, busy, onDraft, onSubmit, onMessageContextMenu }: {
  channel: Channel;
  messages: ChatMessage[];
  loading: boolean;
  draft: string;
  busy: boolean;
  onDraft: (value: string) => void;
  onSubmit: (event: FormEvent) => void;
  onMessageContextMenu: (event: ReactMouseEvent<HTMLElement>, message: ChatMessage) => void;
}) {
  return <div className="chat-column"><div className="chat-content"><div className="welcome-block"><div className="welcome-hash">{channel.kind === "voice" ? <Volume2 size={33} /> : <Hash size={33} />}</div><h2>Bem-vindo ao {channel.kind === "voice" ? "canal " : "#"}{channel.name}</h2><p>{channel.kind === "voice" ? "Converse por texto, entre no áudio e compartilhe sua tela sem trocar de canal." : "As mensagens deste canal são cifradas e sincronizadas entre os nodes do servidor."}</p></div><div className="message-list">{loading ? <div className="loading-state">sincronizando histórico…</div> : messages.length ? messages.map((message) => <MessageRow key={message.id} message={message} onContextMenu={(event) => onMessageContextMenu(event, message)} />) : <div className="empty-state">Nenhuma mensagem ainda. Comece a conversa.</div>}</div></div><form className="composer" onSubmit={onSubmit}><button type="button" className="composer-add" title="Anexo"><Plus size={20} /></button><input value={draft} onChange={(event) => onDraft(event.target.value)} placeholder={`Conversar em #${channel.name}`} disabled={busy || !isDesktop()} /><button type="button" className="composer-tool" title="Emoji"><Smile size={19} /></button><button className="send-button" type="submit" disabled={busy || !draft.trim() || !isDesktop()}><Send size={17} /></button></form></div>;
}

export function MessageRow({ message, onContextMenu }: { message: ChatMessage; onContextMenu: (event: ReactMouseEvent<HTMLElement>) => void }) {
  return <article className={`message-row ${message.mine ? "mine" : ""}`} onContextMenu={onContextMenu}><span className="avatar avatar-message" style={{ background: message.color }}>{message.initials}</span><div className="message-copy"><div className="message-meta"><strong>{message.author}</strong>{message.mine && <span className="you-tag">você</span>}<time>{message.timestamp}</time></div><p>{message.content}</p></div></article>;
}

export function EmptyWorkspace({ onCreate, onJoin }: { onCreate: () => void; onJoin: () => void }) {
  return <div className="empty-workspace"><div className="empty-workspace-icon"><ShieldCheck size={30} /></div><span className="eyebrow">REDE PRIVADA</span><h2>Seu espaço começa aqui</h2><p>Crie um servidor privado ou entre usando um convite assinado.</p><div><button className="primary-button" onClick={onCreate}><Plus size={17} /> criar servidor</button><button className="secondary-button" onClick={onJoin}>usar convite</button></div></div>;
}
