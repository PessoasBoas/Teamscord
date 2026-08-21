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
if (!edgePath) throw new Error("Microsoft Edge não foi encontrado para o smoke test WebRTC.");

const html = readFileSync(new URL("./webrtc-browser-smoke.html", import.meta.url));
const profilePath = join(tmpdir(), `teamscord-webrtc-smoke-${randomUUID()}`);
let resultResolve;
let resultReject;
const resultPromise = new Promise((resolve, reject) => { resultResolve = resolve; resultReject = reject; });

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
      try { resultResolve(JSON.parse(Buffer.concat(chunks).toString("utf8"))); }
      catch (error) { resultReject(error); }
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
const timer = setTimeout(() => resultReject(new Error("tempo esgotado aguardando o resultado do smoke WebRTC")), 45000);
try {
  const port = await listen();
  const edgeArgs = [
    "--headless=new",
    "--disable-gpu",
    "--no-first-run",
    "--disable-extensions",
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    "--autoplay-policy=no-user-gesture-required",
    `--user-data-dir=${profilePath}`,
    `http://127.0.0.1:${port}/`,
  ];
  edge = spawn(edgePath, edgeArgs, { stdio: "ignore", windowsHide: true });
  edge.once("error", resultReject);
  const result = await resultPromise;
  console.log(JSON.stringify(result));
  if (!result.ok) process.exitCode = 1;
} finally {
  clearTimeout(timer);
  if (edge?.pid) {
    if (process.platform === "win32") spawnSync("taskkill", ["/PID", String(edge.pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
    else if (!edge.killed) edge.kill("SIGTERM");
  }
  if (process.platform === "win32") {
    const escapedProfile = profilePath.replaceAll("'", "''");
    const cleanupCommand = `$profile = '${escapedProfile}'; Get-CimInstance Win32_Process -Filter \"Name = 'msedge.exe'\" | Where-Object { $_.CommandLine -like ('*' + $profile + '*') } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }`;
    spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", cleanupCommand], { stdio: "ignore", windowsHide: true });
  }
  server.close();
  await new Promise((resolve) => setTimeout(resolve, 800));
  let cleanupError;
  for (let attempt = 0; attempt < 8; attempt += 1) {
    try {
      rmSync(profilePath, { recursive: true, force: true, maxRetries: 3, retryDelay: 150 });
      cleanupError = undefined;
      break;
    } catch (error) {
      cleanupError = error;
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
  }
  if (cleanupError) console.warn(`aviso: não foi possível remover o perfil temporário do Edge: ${cleanupError}`);
}
