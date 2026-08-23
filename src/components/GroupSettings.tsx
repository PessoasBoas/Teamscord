import { useEffect, useState } from "react";
import { ArrowDown, ArrowUp, Copy, Crown, Palette, Pencil, Plus, Save, ShieldCheck, Trash2, X } from "lucide-react";
import {
  nodeApi,
  type AuditEvent,
  type Channel,
  type ChannelPermission,
  type Group,
  type GroupMember,
  type Permission,
  type Role,
  type RoleView,
} from "../lib/tauri";

type GroupSettingsProps = {
  group: Group;
  localPeerId: string;
  onClose: () => void;
  onChanged: () => Promise<void>;
  onDeleted: () => Promise<void>;
  onProfileChange: (group: Group) => void;
  onFeedback: (message: string) => void;
  onError: (message: string) => void;
};
type Tab = "overview" | "personalization" | "members" | "roles" | "channels" | "invites" | "audit";

export function GroupSettings({ group, localPeerId, onClose, onChanged, onDeleted, onProfileChange, onFeedback, onError }: GroupSettingsProps) {
  const [tab, setTab] = useState<Tab>("overview");
  const [members, setMembers] = useState<GroupMember[]>([]);
  const [roles, setRoles] = useState<RoleView[]>([]);
  const [audit, setAudit] = useState<AuditEvent[]>([]);
  const [channelPermissions, setChannelPermissions] = useState<Record<string, ChannelPermission[]>>({});
  const [channelName, setChannelName] = useState("");
  const [channelKind, setChannelKind] = useState<Channel["kind"]>("text");
  const [editingChannelId, setEditingChannelId] = useState<string | null>(null);
  const [editingChannelName, setEditingChannelName] = useState("");
  const [invite, setInvite] = useState("");
  const [loading, setLoading] = useState(true);
  const [profileName, setProfileName] = useState(group.name);
  const [profileInitials, setProfileInitials] = useState(group.initials);
  const [profileColor, setProfileColor] = useState(group.color);

  async function load() {
    setLoading(true);
    try {
      const [nextMembers, nextRoles, nextAudit, nextPermissions] = await Promise.all([
        nodeApi.listGroupMembers(group.id),
        nodeApi.getGroupRoles(),
        nodeApi.getAuditLog(group.id),
        Promise.all(group.channels.filter((channel) => channel.kind === "voice").map(async (channel) => [channel.id, await nodeApi.getChannelPermissions(group.id, channel.id)] as const)),
      ]);
      setMembers(nextMembers);
      setRoles(nextRoles);
      setAudit(nextAudit);
      setChannelPermissions(Object.fromEntries(nextPermissions));
    } catch (reason) {
      onError(String(reason));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => { void load(); }, [group.id, group.channels]);
  useEffect(() => {
    try {
      const saved = JSON.parse(localStorage.getItem(`teamscord.group-profile:${group.id}`) ?? "null") as { name?: string; initials?: string; color?: string } | null;
      setProfileName(saved?.name ?? group.name);
      setProfileInitials(saved?.initials ?? group.initials);
      setProfileColor(saved?.color ?? group.color);
    } catch {
      setProfileName(group.name);
      setProfileInitials(group.initials);
      setProfileColor(group.color);
    }
  }, [group.id, group.name, group.initials, group.color]);

  function saveProfile() {
    const name = profileName.trim().slice(0, 80) || group.name;
    const initials = profileInitials.trim().slice(0, 3).toUpperCase() || group.initials;
    const color = /^#[0-9a-f]{6}$/i.test(profileColor) ? profileColor : group.color;
    const next = { ...group, name, initials, color };
    localStorage.setItem(`teamscord.group-profile:${group.id}`, JSON.stringify({ name, initials, color }));
    onProfileChange(next);
    onFeedback("aparência do grupo salva neste dispositivo");
  }

  async function action(run: () => Promise<unknown>, success: string) {
    try {
      await run();
      await load();
      await onChanged();
      onFeedback(success);
    } catch (reason) {
      onError(String(reason));
    }
  }

  async function changeRole(member: GroupMember, role: Role) {
    if (member.role === role || !window.confirm(`Alterar o cargo de ${member.display_name}?`)) return;
    await action(() => nodeApi.updateMemberRole(group.id, member.peer_id, role), "cargo atualizado");
  }

  async function transferOwnership(member: GroupMember) {
    if (!window.confirm(`Transferir a propriedade de ${group.name} para ${member.display_name}? Você ficará como Admin.`)) return;
    await action(() => nodeApi.transferGroupOwnership(group.id, member.peer_id), "propriedade transferida");
  }

  async function deleteGroup() {
    if (!window.confirm(`Excluir ${group.name} permanentemente em todos os nodes?`)) return;
    try {
      await nodeApi.deleteGroup(group.id);
      await onDeleted();
      onFeedback("grupo excluído");
    } catch (reason) {
      onError(String(reason));
    }
  }

  async function moderate(member: GroupMember, kind: "kick" | "ban" | "timeout" | "unban") {
    const label = kind === "kick" ? "expulsar" : kind === "ban" ? "banir" : kind === "unban" ? "desbanir" : "aplicar timeout";
    if (!window.confirm(`${label} ${member.display_name}?`)) return;
    const run = kind === "kick" ? () => nodeApi.kickMember(group.id, member.peer_id)
      : kind === "ban" ? () => nodeApi.banMember(group.id, member.peer_id)
        : kind === "unban" ? () => nodeApi.unbanMember(group.id, member.peer_id)
          : () => nodeApi.timeoutMember(group.id, member.peer_id, 300);
    await action(run, `${label} aplicado`);
  }

  async function createChannel() {
    if (!channelName.trim()) return;
    await action(() => nodeApi.createChannel(group.id, channelName.trim(), channelKind), `${channelKind === "voice" ? "canal de voz" : "canal de texto"} criado`);
    setChannelName("");
  }

  async function deleteChannel(channel: Channel) {
    if (!window.confirm(`Excluir #${channel.name}? Esta ação será registrada na auditoria.`)) return;
    await action(() => nodeApi.deleteChannel(group.id, channel.id), "canal excluído");
  }

  async function updateChannel(channel: Channel) {
    const name = editingChannelName.trim();
    if (!name || name === channel.name) { setEditingChannelId(null); return; }
    await action(() => nodeApi.updateChannel({ ...channel, name }), "canal atualizado");
    setEditingChannelId(null);
  }

  async function moveChannel(index: number, direction: -1 | 1) {
    const nextIndex = index + direction;
    if (nextIndex < 0 || nextIndex >= group.channels.length) return;
    const ids = group.channels.map((channel) => channel.id);
    [ids[index], ids[nextIndex]] = [ids[nextIndex], ids[index]];
    await action(() => nodeApi.reorderChannels(group.id, ids), "canais reorganizados");
  }

  async function createInvite() {
    try {
      setInvite(await nodeApi.createInvite(group.id));
      onFeedback("convite criado");
    } catch (reason) {
      onError(String(reason));
    }
  }

  async function updateChannelPermission(permission: ChannelPermission, field: "join" | "screen", value: boolean) {
    const next = { ...permission, can_join_voice: field === "join" ? value : permission.can_join_voice, can_share_screen: field === "screen" ? value : permission.can_share_screen };
    setChannelPermissions((current) => ({ ...current, [permission.channel_id]: (current[permission.channel_id] ?? []).map((item) => item.role === permission.role ? next : item) }));
    await action(() => nodeApi.setChannelPermission(group.id, permission.channel_id, permission.role, next.can_join_voice, next.can_share_screen), "permissão do canal atualizada");
  }

  const labels: Record<Tab, string> = {
    overview: "Visão geral", personalization: "Personalização", members: "Membros", roles: "Cargos e permissões",
    channels: "Canais", invites: "Convites", audit: "Log de auditoria",
  };
  const localMember = members.find((member) => member.peer_id === localPeerId);
  const localRole = localMember?.role;
  const canManageRoles = localRole === "owner" || localRole === "admin";
  const canManageChannels = canManageRoles;
  const canManageMembers = localRole === "owner" || localRole === "admin";
  const canModerate = localRole === "owner" || localRole === "admin" || localRole === "mod";
  const canManageTarget = (targetRole: Role) => localRole === "owner"
    ? targetRole !== "owner"
    : localRole === "admin"
      ? targetRole === "mod" || targetRole === "member"
      : localRole === "mod" && targetRole === "member";
  const roleOptions = (targetRole: Role) => localRole === "owner"
    ? (["admin", "mod", "member"] as Role[])
    : localRole === "admin" && (targetRole === "mod" || targetRole === "member")
      ? (["mod", "member"] as Role[])
      : [];

  return <div className="modal-backdrop" onClick={onClose}>
    <section className="settings-modal" onClick={(event) => event.stopPropagation()}>
      <header className="settings-header"><div><span className="eyebrow">CONTROLE DO GRUPO</span><h2>{group.name}</h2><p>Owner, Admin, Mod e Member · eventos assinados</p></div><button className="icon-button" onClick={onClose} aria-label="Fechar configurações"><X size={19} /></button></header>
      <div className="settings-layout">
        <nav className="settings-tabs" aria-label="Configurações do grupo">{(Object.keys(labels) as Tab[]).map((item) => <button key={item} className={tab === item ? "active" : ""} onClick={() => setTab(item)}>{item === "personalization" && <Palette size={14} />}{labels[item]}</button>)}</nav>
        <div className="settings-content">
          {loading ? <div className="loading-state">carregando controles…</div> : tab === "overview" ? <Overview members={members} channels={group.channels.length} audit={audit.length} isOwner={localPeerId === group.owner_peer_id} onDelete={deleteGroup} /> : tab === "personalization" ? <div className="group-personalization"><div className="group-personalization-preview" style={{ background: profileColor }}><strong>{profileInitials || "TC"}</strong><span>{profileName || group.name}</span></div><div className="preference-section"><h3>Perfil do grupo</h3><p>Essa aparência é salva neste dispositivo para este grupo.</p><label>nome exibido<input value={profileName} maxLength={80} onChange={(event) => setProfileName(event.target.value)} /></label><label>iniciais<input value={profileInitials} maxLength={3} onChange={(event) => setProfileInitials(event.target.value.toUpperCase())} /></label><label>cor principal<input type="color" value={profileColor} onChange={(event) => setProfileColor(event.target.value)} /></label><button className="primary-button" onClick={saveProfile}>salvar aparência</button></div></div> : tab === "members" ? <div className="settings-list">{members.map((member) => <article className="member-admin-row" key={member.peer_id}><span className="avatar small avatar-purple">{member.display_name.slice(0, 2).toUpperCase()}</span><div><strong>{member.display_name}</strong><small>{member.peer_id.slice(0, 18)}… · {member.status}</small></div>{member.role === "owner" ? <span className="role-badge">owner</span> : canManageRoles && canManageTarget(member.role) ? <select value={member.role} onChange={(event) => void changeRole(member, event.target.value as Role)}>{roleOptions(member.role).map((role) => <option key={role} value={role}>{role}</option>)}</select> : <span className="role-badge">{member.role}</span>}<div className="member-actions">{localPeerId === group.owner_peer_id && member.status === "active" && member.role !== "owner" && <button onClick={() => void transferOwnership(member)}><Crown size={12} /> Owner</button>}{member.status === "banned" ? canManageMembers && canManageTarget(member.role) && <button onClick={() => void moderate(member, "unban")}>desbanir</button> : member.status === "active" && canModerate && canManageTarget(member.role) ? <><button onClick={() => void moderate(member, "timeout")}>timeout</button><button onClick={() => void moderate(member, "kick")}>expulsar</button><button className="danger-text" onClick={() => void moderate(member, "ban")}>banir</button></> : null}</div></article>)}{!members.length && <div className="empty-state">Nenhum membro sincronizado.</div>}</div> : tab === "roles" ? <div className="roles-list">{roles.map((role) => <article key={role.role}><div><strong>{role.role}</strong><span>{role.permissions.length} permissões</span></div><p>{role.permissions.map((permission: Permission) => permission.replace(/_/g, " ")).join(" · ")}</p></article>)}</div> : tab === "channels" ? <div className="settings-list">{canManageChannels && <div className="inline-form"><input value={channelName} onChange={(event) => setChannelName(event.target.value)} placeholder="nome do novo canal" /><select value={channelKind} onChange={(event) => setChannelKind(event.target.value as Channel["kind"])} aria-label="Tipo do novo canal"><option value="text">texto</option><option value="voice">voz</option></select><button className="connect-button" onClick={() => void createChannel()}><Plus size={15} /> criar</button></div>}{group.channels.map((channel, index) => <article className="channel-admin-row" key={channel.id}>{editingChannelId === channel.id ? <input className="channel-name-edit" autoFocus value={editingChannelName} onChange={(event) => setEditingChannelName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") void updateChannel(channel); if (event.key === "Escape") setEditingChannelId(null); }} /> : <span>{channel.kind === "text" ? "#" : "◉"} {channel.name}</span>}<small>{channel.kind === "voice" ? "voz + chat + tela" : "texto"}</small>{canManageChannels && (editingChannelId === channel.id ? <button className="icon-button" onClick={() => void updateChannel(channel)} aria-label={`Salvar ${channel.name}`}><Save size={16} /></button> : <button className="icon-button" onClick={() => { setEditingChannelId(channel.id); setEditingChannelName(channel.name); }} aria-label={`Editar ${channel.name}`}><Pencil size={16} /></button>)}{canManageChannels && <><button className="icon-button" disabled={index === 0} onClick={() => void moveChannel(index, -1)} aria-label={`Mover ${channel.name} para cima`}><ArrowUp size={15} /></button><button className="icon-button" disabled={index === group.channels.length - 1} onClick={() => void moveChannel(index, 1)} aria-label={`Mover ${channel.name} para baixo`}><ArrowDown size={15} /></button><button className="icon-button danger-text" onClick={() => void deleteChannel(channel)} aria-label={`Excluir ${channel.name}`}><Trash2 size={16} /></button></>}</article>)}</div> : tab === "invites" ? <div className="invite-settings"><p>Convites assinados expiram e não carregam credenciais TURN.</p>{canManageMembers && <button className="primary-button" onClick={() => void createInvite()}>criar convite</button>}{!canManageMembers && <div className="empty-state">Somente Owner e Admin podem criar convites.</div>}{invite && <div className="invite-box"><code>{invite}</code><button className="connect-button" onClick={() => { void navigator.clipboard?.writeText(invite); onFeedback("convite copiado"); }}><Copy size={15} /> copiar</button></div>}</div> : <div className="audit-list">{audit.map((event) => <article key={event.event_id}><span className="audit-kind">{event.kind}</span><span>{event.issuer_peer_id.slice(0, 12)}…</span><time>{new Date(event.logical_timestamp).toLocaleString("pt-BR")}</time></article>)}{!audit.length && <div className="empty-state">Nenhum evento administrativo.</div>}</div>}
          {!loading && tab === "channels" && <div className="channel-permission-stack"><p className="modal-note">Permissões por canal; desative a entrada ou o compartilhamento para qualquer cargo.</p>{group.channels.filter((channel) => channel.kind === "voice").map((channel) => <ChannelPermissionEditor key={channel.id} permissions={channelPermissions[channel.id] ?? []} disabled={!canManageChannels} onChange={(permission, field, value) => void updateChannelPermission(permission, field, value)} />)}</div>}
        </div>
      </div>
    </section>
  </div>;
}

function ChannelPermissionEditor({ permissions, disabled, onChange }: { permissions: ChannelPermission[]; disabled: boolean; onChange: (permission: ChannelPermission, field: "join" | "screen", value: boolean) => void }) {
  return <div className="channel-permission-editor" aria-label="Permissões deste canal">
    <span className="channel-permission-title">permissões da call</span>
    {permissions.map((permission) => <div className="channel-permission-row" key={permission.role}><strong>{permission.role}</strong><label><input type="checkbox" checked={permission.can_join_voice} disabled={disabled} onChange={(event) => onChange(permission, "join", event.target.checked)} /> entrar</label><label><input type="checkbox" checked={permission.can_share_screen} disabled={disabled} onChange={(event) => onChange(permission, "screen", event.target.checked)} /> compartilhar tela</label></div>)}
  </div>;
}

function Overview({ members, channels, audit, isOwner, onDelete }: { members: GroupMember[]; channels: number; audit: number; isOwner: boolean; onDelete: () => Promise<void> }) {
  return <div className="settings-overview"><div className="settings-hero"><ShieldCheck size={28} /><div><strong>Grupo privado e cifrado</strong><span>A autoridade do Owner e a época atual da chave são validadas em cada node.</span></div></div><div className="settings-stat-grid"><div><span>membros</span><strong>{members.length}</strong></div><div><span>canais</span><strong>{channels}</strong></div><div><span>auditoria</span><strong>{audit}</strong></div></div>{isOwner && <button className="danger-button" onClick={() => void onDelete()}><Trash2 size={15} /> excluir grupo permanentemente</button>}</div>;
}
