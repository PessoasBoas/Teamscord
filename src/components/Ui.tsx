import type { ReactNode } from "react";
import { X } from "lucide-react";

export function ModalHeader({ eyebrow, title, onClose }: { eyebrow: string; title: string; onClose: () => void }) {
  return <div className="modal-header"><div><span className="eyebrow">{eyebrow}</span><h2>{title}</h2></div><button className="icon-button" onClick={onClose} aria-label="Fechar"><X size={19} /></button></div>;
}

export function MenuCard({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={`menu-card ${className ?? ""}`} onClick={(event) => event.stopPropagation()}>{children}</div>;
}

export function PopoverPanel({ className, title, onClose, children }: { className?: string; title: string; onClose: () => void; children: ReactNode }) {
  return <section className={`popover-panel ${className ?? ""}`}><header><strong>{title}</strong><button className="icon-button" onClick={onClose} aria-label="Fechar"><X size={14} /></button></header>{children}</section>;
}
