import { describe, expect, it, vi } from "vitest";
import { checkForUpdate, compareVersions, releaseToUpdateInfo } from "./update-checker";

describe("update checker", () => {
  it("compara versões semver simples", () => {
    expect(compareVersions("v0.3.0", "0.2.0")).toBe(1);
    expect(compareVersions("0.2.0", "0.2.0")).toBe(0);
    expect(compareVersions("0.1.9", "0.2.0")).toBe(-1);
  });

  it("seleciona o instalador Windows x64 de uma release válida", () => {
    const update = releaseToUpdateInfo({
      tag_name: "v0.3.0",
      name: "Teamscord 0.3.0",
      body: "Correções de rede",
      html_url: "https://github.com/PessoasBoas/Teamscord/releases/tag/v0.3.0",
      assets: [
        { name: "Teamscord_0.3.0_x64-setup.exe", browser_download_url: "https://github.com/PessoasBoas/Teamscord/releases/download/v0.3.0/Teamscord_0.3.0_x64-setup.exe" },
        { name: "Teamscord_0.3.0_x64-setup.exe.sha256", browser_download_url: "https://github.com/PessoasBoas/Teamscord/releases/download/v0.3.0/Teamscord_0.3.0_x64-setup.exe.sha256" },
      ],
    }, "0.2.0");

    expect(update).toMatchObject({ version: "0.3.0", hasInstaller: true, downloadUrl: "https://github.com/PessoasBoas/Teamscord/releases/download/v0.3.0/Teamscord_0.3.0_x64-setup.exe" });
  });

  it("rejeita release anterior, pré-release, rascunho ou URL adulterada", () => {
    const base = { tag_name: "v0.3.0", html_url: "https://github.com/PessoasBoas/Teamscord/releases/tag/v0.3.0", assets: [] };
    expect(releaseToUpdateInfo({ ...base, tag_name: "v0.2.0" }, "0.2.0")).toBeNull();
    expect(releaseToUpdateInfo({ ...base, prerelease: true }, "0.2.0")).toBeNull();
    expect(releaseToUpdateInfo({ ...base, draft: true }, "0.2.0")).toBeNull();
    expect(releaseToUpdateInfo({ ...base, html_url: "https://example.com/release" }, "0.2.0")).toBeNull();
  });

  it("consulta a release latest sem bloquear o aplicativo quando o GitHub falha", async () => {
    const fetcher = vi.fn<typeof fetch>().mockRejectedValue(new Error("offline"));
    await expect(checkForUpdate("0.2.0", fetcher)).resolves.toBeNull();
    expect(fetcher).toHaveBeenCalledOnce();
  });
});
