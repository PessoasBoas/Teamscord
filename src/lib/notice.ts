export type NoticeLevel = "success" | "warning" | "error" | "info";

export type FormattedNotice = {
  title: string;
  message: string;
  technicalDetails?: string;
  level: NoticeLevel;
  transient: boolean;
};

export function formatNotice(value: string): FormattedNotice {
  const technicalDetails = value.trim();
  const normalized = technicalDetails.toLowerCase();
  if (normalized.includes("presença não entregue") && /(eof|timeout|stream|connection|conexão)/i.test(technicalDetails)) {
    return { title: "Presença reconectando", message: "A conexão com um node foi interrompida. Tentando novamente…", technicalDetails, level: "warning", transient: true };
  }
  if (normalized.includes("sincronização falhou") && /(eof|timeout|stream|connection|conexão)/i.test(technicalDetails)) {
    return { title: "Sincronização aguardando conexão", message: "O histórico será recuperado assim que o peer voltar a responder.", technicalDetails, level: "warning", transient: true };
  }
  return { title: "Não foi possível concluir", message: technicalDetails, level: "error", transient: false };
}
