import { createServer } from "node:http";
import { accessSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";

const edgeCandidates = [
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
];
const edgePath = edgeCandidates.find((candidate) => {
  try { accessSync(candidate); return true; } catch { return false; }
});
if (!edgePath) throw new Error("Microsoft Edge não foi encontrado.");

const html = readFileSync(new URL("./webrtc-display-permission-smoke.html", import.meta.url));
const profilePath = join(tmpdir(), `teamscord-native-display-${randomUUID()}`);
let resolveResult;
let rejectResult;
const resultPromise = new Promise((resolve, reject) => { resolveResult = resolve; rejectResult = reject; });
const server = createServer((request, response) => {
  if (request.method === "GET" && request.url === "/") {
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end(html);
    return;
  }
  if (request.method === "POST" && request.url === "/result") {
    const chunks = [];
    request.on("data", (chunk) => chunks.push(chunk));
    request.on("end", () => {
      try { resolveResult(JSON.parse(Buffer.concat(chunks).toString("utf8"))); } catch (error) { rejectResult(error); }
      response.writeHead(204);
      response.end();
    });
    return;
  }
  response.writeHead(404);
  response.end();
});
const listen = () => new Promise((resolve, reject) => {
  server.once("error", reject);
  server.listen(0, "127.0.0.1", () => resolve(server.address().port));
});

let edge;
const timer = setTimeout(() => rejectResult(new Error("tempo esgotado aguardando o picker nativo de tela")), 12000);
try {
  const port = await listen();
  const args = [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--disable-extensions",
    "--use-fake-ui-for-media-stream",
    "--enable-usermedia-screen-capturing",
    "--allow-http-screen-capture",
    "--auto-select-desktop-capture-source=Entire screen",
    `--user-data-dir=${profilePath}`,
    `http://127.0.0.1:${port}/`,
  ];
  edge = spawn(edgePath, args, { stdio: "ignore", windowsHide: true });
  edge.once("error", rejectResult);
  const result = await resultPromise;
  console.log(JSON.stringify(result));
  if (!result.ok) process.exitCode = 1;
} finally {
  clearTimeout(timer);
  if (edge?.pid && process.platform === "win32") spawnSync("taskkill", ["/PID", String(edge.pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
  if (process.platform === "win32") {
    const escapedProfile = profilePath.replaceAll("'", "''");
    const cleanup = `$profile = '${escapedProfile}'; Get-CimInstance Win32_Process -Filter \"Name = 'msedge.exe'\" | Where-Object { $_.CommandLine -like ('*' + $profile + '*') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`;
    spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", cleanup], { stdio: "ignore", windowsHide: true });
  }
  server.close();
  await new Promise((resolve) => setTimeout(resolve, 500));
  rmSync(profilePath, { recursive: true, force: true, maxRetries: 3, retryDelay: 150 });
}
