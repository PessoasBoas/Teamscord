import type { MouseEvent as ReactMouseEvent, ReactNode } from "react";
import { Hash, Volume2 } from "lucide-react";
import type { Channel, GroupMember } from "../lib/tauri";

export function channelIcon(kind: Channel["kind"]) {
  return kind === "voice" ? <Volume2 size={17} /> : <Hash size={17} />;
}

export function ChannelSection({ title, icon, action, children }: { title: string; icon: ReactNode; action?: ReactNode; children: ReactNode }) {
  return <section className="channel-section"><div className="section-heading"><span>{icon}{title}</span>{action}</div>{children}</section>;
}

export function ChannelRow({ channel, active, unread, onClick, onContextMenu }: { channel: Channel; active: boolean; unread: number; onClick: () => void; onContextMenu?: (event: ReactMouseEvent<HTMLButtonElement>) => void }) {
  return <button className={`channel-row ${active ? "active" : ""}`} onClick={onClick} onContextMenu={onContextMenu} title={channel.name}><span className="channel-row-icon">{channelIcon(channel.kind)}</span><span>{channel.name}</span>{unread > 0 && <b>{unread > 9 ? "9+" : unread}</b>}</button>;
}

export function MemberRow({ member, activeCall, online = false, onContextMenu }: { member: GroupMember; activeCall: boolean; online?: boolean; onContextMenu?: (event: ReactMouseEvent<HTMLDivElement>) => void }) {
  const isOnline = member.status === "active" && online;
  return <div className="member" onContextMenu={onContextMenu} title={isOnline ? "node conectado" : "membro não conectado neste momento"}><span className="avatar small avatar-purple">{member.display_name.slice(0, 2).toUpperCase()}<span className={`presence ${isOnline ? "online" : "idle"}`} /></span><span>{member.display_name}</span>{activeCall && <span className="member-call-icon" aria-label="em call"><Volume2 size={13} /></span>}<small className="member-role">{member.role}</small></div>;
}
