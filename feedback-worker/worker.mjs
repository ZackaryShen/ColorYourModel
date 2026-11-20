import { renderFeedback } from "./page.mjs";

const repository = "ZackaryShen/ColorYourModel";
const githubApi = "https://api.github.com";
const turnstileApi = "https://challenges.cloudflare.com/turnstile/v0/siteverify";
const issueList = `https://github.com/${repository}/issues`;
const maxBodyBytes = 96 * 1024;
const limits = { title: 120, description: 4000, steps: 4000, version: 80 };
const systems = ["Windows", "macOS", "Linux", "Other"];

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const theme = url.searchParams.get("theme");
    const form = {
      lang: url.searchParams.get("lang"),
      version: (url.searchParams.get("version") ?? "").slice(0, limits.version),
      theme: theme === "light" || theme === "dark" ? theme : "",
    };
    const page = (status, error, issueNumber) => renderFeedback({
      form, status, error, issueNumber, issueList, limits, systems, siteKey: env.TURNSTILE_SITE_KEY,
    });

    if (url.pathname !== "/" && url.pathname !== "/submit") return new Response(null, { status: 404 });
    const method = url.pathname === "/" ? "GET" : "POST";
    if (request.method !== method) return new Response(null, { status: 405, headers: { Allow: method } });
    if (!env.TURNSTILE_SITE_KEY || !env.TURNSTILE_SECRET || !env.GITHUB_APP_CLIENT_ID ||
        !env.GITHUB_PRIVATE_KEY || !/^\d+$/.test(env.GITHUB_INSTALLATION_ID) ||
        !env.FEEDBACK_RATE_LIMIT || !env.ISSUE_RATE_LIMIT) return page(503, "unavailable");
    if (method === "GET") return page(200);

    if (request.headers.get("Origin") !== url.origin) return page(403, "invalid");
    if (request.headers.get("Content-Type")?.split(";")[0] !== "application/x-www-form-urlencoded") {
      return page(415, "invalid");
    }
    if (Number(request.headers.get("Content-Length")) > maxBodyBytes) return page(413, "invalid");

    try {
      const payload = await readForm(request);
      if (!payload) return page(413, "invalid");
      form.lang = payload.get("lang");
      form.os = systems.includes(payload.get("os")) ? payload.get("os") : "";
      form.kind = payload.get("kind") === "feature" ? "feature" : "bug";
      form.consent = payload.get("consent") === "yes";

      let tooLong = false;
      for (const [field, max] of Object.entries(limits)) {
        const value = (payload.get(field) ?? "").trim();
        tooLong ||= value.length > max;
        form[field] = value.slice(0, max);
      }
      form.title = form.title.replace(/[\u0000-\u001f\u007f-\u009f]+/g, " ").trim();
      if (tooLong || !form.title || !form.description || !form.consent) return page(400, "invalid");

      const token = payload.get("cf-turnstile-response");
      if (!token || token.length > 2048) return page(400, "challenge");
      const client = request.headers.get("CF-Connecting-IP") ?? "unknown";
      if (!(await env.FEEDBACK_RATE_LIMIT.limit({ key: client })).success) return page(429, "limited");

      const verified = await postJson(turnstileApi, { secret: env.TURNSTILE_SECRET, response: token });
      if (!verified.ok) return page(502, "verificationUnavailable");
      const result = await verified.json();
      if (result.success !== true || result.hostname !== url.hostname || result.action !== "feedback") {
        return page(400, "challenge");
      }
      if (!(await env.ISSUE_RATE_LIMIT.limit({ key: repository })).success) return page(429, "limited");

      const issueNumber = await createIssue(env, form);
      return page(201, undefined, issueNumber);
    } catch {
      return page(502, "failed");
    }
  },
};

async function readForm(request) {
  if (!request.body) return new URLSearchParams();
  const reader = request.body.getReader();
  const bytes = new Uint8Array(maxBodyBytes);
  let size = 0;
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    if (size + value.byteLength > bytes.length) {
      await reader.cancel();
      return null;
    }
    bytes.set(value, size);
    size += value.byteLength;
  }
  return new URLSearchParams(new TextDecoder().decode(bytes.subarray(0, size)));
}

function postJson(url, body, token) {
  const headers = { "Content-Type": "application/json" };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
    headers.Accept = "application/vnd.github+json";
    headers["User-Agent"] = "ColorYourModel-Feedback";
    headers["X-GitHub-Api-Version"] = "2026-03-10";
  }
  // Workers only supports follow and manual. Callers reject non-success statuses.
  return fetch(url, { method: "POST", headers, body: JSON.stringify(body), signal: AbortSignal.timeout(8000), redirect: "manual" });
}

function base64url(bytes) {
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function appJwt(env) {
  const pem = env.GITHUB_PRIVATE_KEY.replace(/-----BEGIN PRIVATE KEY-----|-----END PRIVATE KEY-----|\s/g, "");
  const algorithm = { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" };
  const key = await crypto.subtle.importKey("pkcs8", Uint8Array.from(atob(pem), c => c.charCodeAt(0)), algorithm, false, ["sign"]);
  const encoder = new TextEncoder();
  const now = Math.floor(Date.now() / 1000);
  const header = base64url(encoder.encode(JSON.stringify({ alg: "RS256", typ: "JWT" })));
  const claims = base64url(encoder.encode(JSON.stringify({ iat: now - 60, exp: now + 540, iss: env.GITHUB_APP_CLIENT_ID })));
  const unsigned = `${header}.${claims}`;
  const signature = await crypto.subtle.sign(algorithm.name, key, encoder.encode(unsigned));
  return `${unsigned}.${base64url(new Uint8Array(signature))}`;
}

function issueSection(heading, value) {
  // Indent every user-authored line so mentions, HTML and Markdown display as text.
  const plainText = value.replace(/\r\n?/g, "\n").split("\n").map(line => `    ${line}`).join("\n");
  return `### ${heading}\n\n${plainText}`;
}

async function createIssue(env, form) {
  const jwt = await appJwt(env);
  const installation = await postJson(`${githubApi}/app/installations/${env.GITHUB_INSTALLATION_ID}/access_tokens`, {
    repositories: ["ColorYourModel"], permissions: { issues: "write" },
  }, jwt);
  if (installation.status !== 201) throw new Error("GitHub App authentication failed");
  const { token } = await installation.json();
  if (typeof token !== "string" || !token) throw new Error("Missing GitHub App installation token");

  const feature = form.kind === "feature";
  const sections = [issueSection("Description", form.description)];
  if (form.steps) sections.push(issueSection(feature ? "Additional details" : "Steps to reproduce", form.steps));
  if (form.version) sections.push(issueSection("ColorYourModel version", form.version));
  if (form.os) sections.push(issueSection("Operating system", form.os));
  sections.push("Submitted via the ColorYourModel feedback form.");

  const issue = await postJson(`${githubApi}/repos/${repository}/issues`, {
    title: form.title, body: sections.join("\n\n"), labels: [feature ? "enhancement" : "bug"],
  }, token);
  if (issue.status !== 201) throw new Error("GitHub issue creation failed");
  const { number } = await issue.json();
  if (!Number.isSafeInteger(number) || number < 1) throw new Error("Invalid GitHub issue number");
  return number;
}
