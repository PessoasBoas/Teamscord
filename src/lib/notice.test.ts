import { describe, expect, it } from "vitest";
import { formatNotice } from "./notice";

describe("formatNotice", () => {
  it("humaniza EOF transitório de presença e preserva o detalhe técnico", () => {
    const notice = formatNotice("presença não entregue: IO error on outbound stream: EOF while parsing a value at line 1 column 0");
    expect(notice).toMatchObject({ level: "warning", transient: true, title: "Presença reconectando" });
    expect(notice.message).not.toContain("EOF");
    expect(notice.technicalDetails).toContain("EOF");
  });

  it("trata timeout de sincronização como estado temporário", () => {
    const notice = formatNotice("sincronização falhou: timeout while waiting for a response");
    expect(notice).toMatchObject({ level: "warning", transient: true, title: "Sincronização aguardando conexão" });
  });

  it("mantém erros de domínio visíveis e não os classifica como reconexão", () => {
    const notice = formatNotice("convite expirado");
    expect(notice).toMatchObject({ level: "error", transient: false, message: "convite expirado" });
    expect(notice.technicalDetails).toBeUndefined();
  });
});
