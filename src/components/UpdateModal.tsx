import type { MouseEvent } from "react";
import { Download, ExternalLink, Sparkles, X } from "lucide-react";
import type { UpdateInfo } from "../lib/update-checker";
import { nodeApi } from "../lib/tauri";

export function UpdateModal({ update, onLater }: { update: UpdateInfo; onLater: () => void }) {
  async function openUrl(event: MouseEvent<HTMLAnchorElement>, url: string) {
    event.preventDefault();
    try {
      await nodeApi.openExternalUrl(url);
    } catch {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  }

  return <div className="modal-backdrop update-backdrop" onClick={onLater}>
    <section className="network-modal update-modal" role="dialog" aria-modal="true" aria-labelledby="update-title" onClick={(event) => event.stopPropagation()}>
      <button className="icon-button update-close" onClick={onLater} aria-label="Fechar aviso de atualização"><X size={18} /></button>
      <div className="update-icon"><Sparkles size={23} /></div>
      <span className="eyebrow">NOVA VERSÃO DISPONÍVEL</span>
      <h2 id="update-title">Atualize o Teamscord</h2>
      <p className="update-copy">A versão <strong>{update.version}</strong> já está disponível no GitHub.</p>
      {update.notes && <div className="update-notes">{update.notes}</div>}
      <div className="update-actions">
        <a className="primary-button update-download" href={update.downloadUrl} target="_blank" rel="noreferrer" onClick={(event) => void openUrl(event, update.downloadUrl)}><Download size={15} /> {update.hasInstaller ? "Baixar instalador" : "Abrir release"}</a>
        <a className="update-release-link" href={update.releaseUrl} target="_blank" rel="noreferrer" onClick={(event) => void openUrl(event, update.releaseUrl)}><ExternalLink size={13} /> ver detalhes da versão</a>
      </div>
      <p className="modal-note update-note">A atualização é manual: baixe o instalador oficial e confirme a instalação. O Teamscord não instala uma versão nova silenciosamente.</p>
      <button className="secondary-button update-later" onClick={onLater}>lembrar depois</button>
    </section>
  </div>;
}
