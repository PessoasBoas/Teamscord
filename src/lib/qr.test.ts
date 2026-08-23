import { describe, expect, it } from "vitest";
import { qrDataUrl } from "./qr";

describe("QR de compartilhamento", () => {
  it("gera uma imagem autocontida para códigos de contato e convite", async () => {
    const dataUrl = await qrDataUrl("teamscord://contact/v1/test", 160);
    expect(dataUrl.startsWith("data:image/png;base64,")).toBe(true);
    expect(dataUrl.length).toBeGreaterThan(200);
  });
});
