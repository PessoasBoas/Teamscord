import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { AlertTriangle, CheckCircle2, ChevronDown, Info, X } from "lucide-react";

export function ModalHeader({ eyebrow, title, onClose }: { eyebrow: string; title: string; onClose: () => void }) {
  return <div className="modal-header"><div><span className="eyebrow">{eyebrow}</span><h2>{title}</h2></div><button className="icon-button" onClick={onClose} aria-label="Fechar"><X size={19} /></button></div>;
}

export function MenuCard({ className, children }: { className?: string; children: ReactNode }) {
  const card = <div className={`menu-card ${className ?? ""}`} onClick={(event) => event.stopPropagation()}>{children}</div>;
  return typeof document === "undefined" ? card : createPortal(card, document.body);
}

export function PopoverPanel({ className, title, onClose, children }: { className?: string; title: string; onClose: () => void; children: ReactNode }) {
  const panel = <section className={`popover-panel ${className ?? ""}`} role="dialog" aria-label={title} onClick={(event) => event.stopPropagation()}><header><div><span className="popover-kicker">CENTRAL</span><strong>{title}</strong></div><button className="icon-button" onClick={onClose} aria-label="Fechar"><X size={16} /></button></header>{children}</section>;
  return typeof document === "undefined" ? panel : createPortal(panel, document.body);
}

export function ToastNotice({ title, message, technicalDetails, level = "success", onClose }: {
  title: string;
  message: string;
  technicalDetails?: string;
  level?: "success" | "warning" | "error" | "info";
  onClose: () => void;
}) {
  const icon = level === "success" ? <CheckCircle2 size={16} /> : level === "warning" ? <AlertTriangle size={16} /> : level === "error" ? <AlertTriangle size={16} /> : <Info size={16} />;
  return <aside className={`toast toast-${level}`} role={level === "error" ? "alert" : "status"} aria-live="polite">
    <span className="toast-icon" aria-hidden="true">{icon}</span>
    <div className="toast-copy"><strong>{title}</strong><span>{message}</span>{technicalDetails && <details><summary>ver detalhes técnicos <ChevronDown size={13} /></summary><code>{technicalDetails}</code></details>}</div>
    <button className="toast-close" onClick={onClose} aria-label="Fechar aviso"><X size={14} /></button>
  </aside>;
}
