import type { ReactNode } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";

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
