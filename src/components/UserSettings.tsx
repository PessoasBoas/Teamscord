import { useState } from "react";
import { Palette, UserRound, Volume2, X } from "lucide-react";
import type { UserPreferences } from "../lib/tauri";

type UserSettingsProps = {
  preferences: UserPreferences;
  onChange: (preferences: UserPreferences) => void;
  onClose: () => void;
};

type Tab = "appearance" | "profile" | "audio";

export function UserSettings({ preferences, onChange, onClose }: UserSettingsProps) {
  const [tab, setTab] = useState<Tab>("appearance");
  const [draftName, setDraftName] = useState(preferences.display_name);

  function update<K extends keyof UserPreferences>(key: K, value: UserPreferences[K]) {
    onChange({ ...preferences, [key]: value });
  }

  function saveName() {
    const displayName = draftName.trim().slice(0, 40) || "Você";
    update("display_name", displayName);
    setDraftName(displayName);
  }

  return <div className="modal-backdrop" onClick={onClose}>
    <section className="settings-modal user-settings-modal" onClick={(event) => event.stopPropagation()}>
      <header className="settings-header"><div><span className="eyebrow">PREFERÊNCIAS LOCAIS</span><h2>Configurações do usuário</h2><p>Estas opções ficam somente neste dispositivo.</p></div><button className="icon-button" onClick={onClose} aria-label="Fechar configurações"><X size={19} /></button></header>
      <div className="settings-layout">
        <nav className="settings-tabs" aria-label="Configurações do usuário">
          <button className={tab === "appearance" ? "active" : ""} onClick={() => setTab("appearance")}><Palette size={14} /> Aparência</button>
          <button className={tab === "profile" ? "active" : ""} onClick={() => setTab("profile")}><UserRound size={14} /> Perfil local</button>
          <button className={tab === "audio" ? "active" : ""} onClick={() => setTab("audio")}><Volume2 size={14} /> Áudio</button>
        </nav>
        <div className="settings-content">
          {tab === "appearance" && <div className="preference-form">
            <div className="preference-section"><h3>Tema</h3><p>Escolha como o Teamscord deve aparecer.</p><div className="preference-options">{(["dark", "light", "system"] as const).map((theme) => <button key={theme} className={preferences.theme === theme ? "selected" : ""} onClick={() => update("theme", theme)}>{theme === "dark" ? "Escuro" : theme === "light" ? "Claro" : "Sistema"}</button>)}</div></div>
            <div className="preference-section"><h3>Fonte</h3><p>A fonte afeta somente a interface deste computador.</p><div className="preference-options">{(["manrope", "system", "mono"] as const).map((font) => <button key={font} className={preferences.font === font ? "selected" : ""} onClick={() => update("font", font)}>{font === "manrope" ? "Manrope" : font === "system" ? "Sistema" : "Monoespaçada"}</button>)}</div></div>
            <div className="preference-section"><h3>Densidade</h3><p>Ajuste o tamanho dos canais e mensagens.</p><div className="preference-options">{(["compact", "comfortable", "large"] as const).map((scale) => <button key={scale} className={preferences.scale === scale ? "selected" : ""} onClick={() => update("scale", scale)}>{scale === "compact" ? "Compacta" : scale === "comfortable" ? "Confortável" : "Grande"}</button>)}</div></div>
          </div>}
          {tab === "profile" && <div className="preference-form"><div className="preference-section"><h3>Nome exibido</h3><p>Esse nome será usado nas mensagens e nas calls deste node.</p><div className="profile-edit"><span className="avatar avatar-purple">{(draftName || "VC").slice(0, 2).toUpperCase()}</span><input value={draftName} maxLength={40} onChange={(event) => setDraftName(event.target.value)} onBlur={saveName} onKeyDown={(event) => { if (event.key === "Enter") saveName(); }} /><button className="connect-button" onClick={saveName}>salvar</button></div></div><div className="settings-hero"><UserRound size={24} /><div><strong>Identidade P2P protegida</strong><span>A identidade criptográfica do node não é alterada ao trocar o nome.</span></div></div></div>}
          {tab === "audio" && <div className="preference-form"><div className="preference-section"><h3>Dispositivos</h3><p>Os dispositivos são escolhidos dentro da call quando o WebView2 libera acesso ao microfone.</p></div><div className="settings-hero"><Volume2 size={24} /><div><strong>WebRTC P2P</strong><span>Áudio usa DTLS-SRTP e não é salvo no banco local.</span></div></div></div>}
        </div>
      </div>
    </section>
  </div>;
}
