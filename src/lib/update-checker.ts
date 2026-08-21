export const CURRENT_VERSION = "0.2.0";
export const RELEASES_ENDPOINT = "https://api.github.com/repos/PessoasBoas/Teamscord/releases/latest";
export const DISMISSED_UPDATE_STORAGE = "teamscord.dismissed-update";

export type UpdateInfo = {
  version: string;
  title: string;
  notes: string;
  releaseUrl: string;
  downloadUrl: string;
  hasInstaller: boolean;
};

type ReleaseAsset = {
  name?: unknown;
  browser_download_url?: unknown;
};

type GitHubRelease = {
  tag_name?: unknown;
  name?: unknown;
  body?: unknown;
  html_url?: unknown;
  draft?: unknown;
  prerelease?: unknown;
  assets?: unknown;
};

type Semver = [number, number, number];

function parseVersion(value: string): Semver | null {
  const match = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(value.trim());
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

export function normalizeVersion(value: string): string | null {
  const parsed = parseVersion(value);
  return parsed ? parsed.join(".") : null;
}

export function compareVersions(left: string, right: string): number {
  const a = parseVersion(left);
  const b = parseVersion(right);
  if (!a || !b) return 0;
  for (let index = 0; index < a.length; index += 1) {
    if (a[index] !== b[index]) return a[index] > b[index] ? 1 : -1;
  }
  return 0;
}

function isAllowedGitHubUrl(value: unknown, prefix: string): value is string {
  if (typeof value !== "string" || !value.trim()) return false;
  try {
    const url = new URL(value);
    return url.protocol === "https:" && url.hostname.toLowerCase() === "github.com" && url.pathname.toLowerCase().startsWith(prefix.toLowerCase());
  } catch {
    return false;
  }
}

export function releaseToUpdateInfo(release: GitHubRelease, currentVersion = CURRENT_VERSION): UpdateInfo | null {
  if (release.draft === true || release.prerelease === true) return null;
  if (typeof release.tag_name !== "string") return null;

  const version = normalizeVersion(release.tag_name);
  if (!version || compareVersions(version, currentVersion) <= 0) return null;

  const releaseUrl = isAllowedGitHubUrl(release.html_url, "/pessoasboas/teamscord/releases/") ? release.html_url : null;
  if (!releaseUrl) return null;

  const assets = Array.isArray(release.assets) ? release.assets : [];
  const installer = assets
    .filter((asset): asset is ReleaseAsset => typeof asset === "object" && asset !== null)
    .map((asset) => ({
      name: typeof asset.name === "string" ? asset.name : "",
      url: isAllowedGitHubUrl(asset.browser_download_url, "/pessoasboas/teamscord/releases/download/") ? asset.browser_download_url : "",
    }))
    .filter((asset) => asset.url && /_x64-setup\.exe$/i.test(asset.name))
    [0];

  return {
    version,
    title: typeof release.name === "string" && release.name.trim() ? release.name.trim() : `Teamscord ${version}`,
    notes: typeof release.body === "string" ? release.body.trim() : "",
    releaseUrl,
    downloadUrl: installer?.url || releaseUrl,
    hasInstaller: Boolean(installer?.url),
  };
}

export async function checkForUpdate(currentVersion = CURRENT_VERSION, fetcher: typeof fetch = globalThis.fetch.bind(globalThis)): Promise<UpdateInfo | null> {
  if (typeof fetcher !== "function") return null;
  const controller = new AbortController();
  const timeout = globalThis.setTimeout(() => controller.abort(), 8_000);
  try {
    const response = await fetcher(RELEASES_ENDPOINT, {
      headers: { Accept: "application/vnd.github+json" },
      signal: controller.signal,
    });
    if (!response.ok) return null;
    const release = await response.json() as GitHubRelease;
    return releaseToUpdateInfo(release, currentVersion);
  } catch {
    return null;
  } finally {
    globalThis.clearTimeout(timeout);
  }
}
