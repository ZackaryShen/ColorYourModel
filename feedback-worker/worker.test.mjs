import assert from "node:assert/strict";
import { generateKeyPairSync, verify } from "node:crypto";
import { test } from "node:test";
import { JSDOM } from "jsdom";
import worker from "./worker.mjs";

const origin = "https://feedback.example.com";
const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const env = {
  TURNSTILE_SITE_KEY: "widget-key",
  TURNSTILE_SECRET: "widget-secret",
  GITHUB_APP_CLIENT_ID: "Iv1.example",
  GITHUB_INSTALLATION_ID: "456",
  GITHUB_PRIVATE_KEY: privateKey.export({ type: "pkcs8", format: "pem" }),
  FEEDBACK_RATE_LIMIT: { limit: async () => ({ success: true }) },
  ISSUE_RATE_LIMIT: { limit: async () => ({ success: true }) },
};
const fields = {
  lang: "zh",
  kind: "bug",
  title: "导出失败",
  description: "导出时报错\n@somebody\n<script>alert(1)</script>",
  steps: "导入模型后点击导出",
  version: "0.2.0",
  os: "macOS",
  consent: "yes",
  "cf-turnstile-response": "challenge-token",
};

function submit(overrides, bindings = env, requestOrigin = origin, path = "/submit") {
  return worker.fetch(new Request(new URL(path, origin), {
    method: "POST",
    headers: { Origin: requestOrigin },
    body: new URLSearchParams({ ...fields, ...overrides }),
  }), bindings);
}

function upstream(t, { challenge = {}, issueStatus = 201 } = {}) {
  const seen = [];
  t.mock.method(globalThis, "fetch", async (url, options) => {
    seen.push(url);
    assert.equal(options.method, "POST");
    assert.equal(options.redirect, "manual");
    if (url === "https://challenges.cloudflare.com/turnstile/v0/siteverify") {
      assert.deepEqual(JSON.parse(options.body), { secret: "widget-secret", response: "challenge-token" });
      return Response.json({ success: true, action: "feedback", hostname: "feedback.example.com", ...challenge });
    }
    if (url === "https://api.github.com/app/installations/456/access_tokens") {
      const [header, claims, signature] = options.headers.Authorization.slice(7).split(".");
      assert.equal(JSON.parse(Buffer.from(header, "base64url")).alg, "RS256");
      assert.equal(JSON.parse(Buffer.from(claims, "base64url")).iss, "Iv1.example");
      assert.ok(verify("sha256", Buffer.from(`${header}.${claims}`), publicKey, Buffer.from(signature, "base64url")));
      assert.deepEqual(JSON.parse(options.body), { repositories: ["ColorYourModel"], permissions: { issues: "write" } });
      return Response.json({ token: "test-installation-token" }, { status: 201 });
    }
    assert.equal(url, "https://api.github.com/repos/ZackaryShen/ColorYourModel/issues");
    assert.equal(options.headers.Authorization, "Bearer test-installation-token");
    const issue = JSON.parse(options.body);
    assert.equal(issue.title, "导出失败");
    assert.deepEqual(issue.labels, ["bug"]);
    assert.match(issue.body, /\n    @somebody\n    <script>alert\(1\)<\/script>/);
    assert.match(issue.body, /macOS/);
    assert.match(issue.body, /0\.2\.0/);
    return Response.json({ number: 42, html_url: "https://example.invalid/" }, { status: issueStatus });
  });
  return seen;
}

test("serves a localized public feedback form with the app version", async () => {
  const response = await worker.fetch(new Request(`${origin}/?lang=zh&version=0.2.0`), env);
  const html = await response.text();
  assert.equal(response.status, 200);
  assert.match(html, /<html lang="zh-CN">/);
  assert.match(html, /公开 Issue/);
  assert.match(html, /name="version"[^>]+value="0.2.0"/);
  assert.match(html, /data-sitekey="widget-key"/);
  assert.doesNotMatch(html, /widget-secret|PRIVATE KEY/);
});

test("keeps the desktop theme and draft when the user needs to retry", async () => {
  for (const theme of ["light", "dark"]) {
    const initial = await worker.fetch(new Request(`${origin}/?lang=zh&theme=${theme}`), env);
    const { window } = new JSDOM(await initial.text());
    try {
      const { document } = window;
      assert.equal(document.documentElement.dataset.theme, theme);
      assert.equal(document.querySelector(".cf-turnstile").dataset.theme, theme);
      const path = document.querySelector("form").getAttribute("action");
      const response = await submit({ "cf-turnstile-response": "" }, env, origin, path);
      const html = await response.text();
      assert.equal(response.status, 400);
      assert.match(html, new RegExp(`<html[^>]+data-theme="${theme}"`));
      assert.match(html, /value="导出失败"/);
    } finally {
      window.close();
    }
  }
});

test("switches guidance between bug reports and suggestions without replacing the draft", async () => {
  const response = await worker.fetch(new Request(`${origin}/?lang=zh&version=0.2.0`), env);
  const { window } = new JSDOM(await response.text(), { runScripts: "dangerously" });
  try {
    const form = window.document.querySelector("form");
    const description = form.elements.namedItem("description");
    description.value = "正在填写的反馈";
    const feature = form.querySelector('[type=radio][value=feature]');
    assert.ok(feature, "feature suggestions have a visible type choice");
    const bugPrompt = description.placeholder;
    feature.click();
    assert.notEqual(description.placeholder, bugPrompt);
    assert.match(description.placeholder, /希望/);
    assert.equal(description.value, "正在填写的反馈");
  } finally {
    window.close();
  }
});

test("asks for human verification in place and keeps the draft", async () => {
  const response = await worker.fetch(new Request(`${origin}/?lang=en`), env);
  const { window } = new JSDOM(await response.text(), { runScripts: "dangerously" });
  try {
    const form = window.document.querySelector("form");
    form.elements.namedItem("description").value = "My feedback draft";
    const submitted = form.dispatchEvent(new window.Event("submit", { cancelable: true }));
    assert.equal(submitted, false);
    assert.equal(form.elements.namedItem("description").value, "My feedback draft");
    assert.match(form.querySelector('[role="status"]').textContent, /verification/i);
    assert.equal(form.querySelector('button[type="submit"]').disabled, false);

    const token = window.document.createElement("input");
    token.type = "hidden";
    token.name = "cf-turnstile-response";
    token.value = "test-response";
    form.append(token);
    assert.equal(form.dispatchEvent(new window.Event("submit", { cancelable: true })), true);
    assert.equal(form.querySelector('button[type="submit"]').disabled, true);
    assert.equal(form.dispatchEvent(new window.Event("submit", { cancelable: true })), false);
  } finally {
    window.close();
  }
});

test("creates a public issue as the GitHub App and returns the canonical issue URL", async (t) => {
  const seen = upstream(t);
  const response = await submit({}, env, origin, "/submit?theme=dark");
  assert.equal(response.status, 201);
  const html = await response.text();
  assert.match(html, /href="https:\/\/github\.com\/ZackaryShen\/ColorYourModel\/issues\/42"/);
  assert.match(html, /<html[^>]+data-theme="dark"/);
  assert.match(html, /href="\/\?lang=zh&amp;version=0\.2\.0&amp;theme=dark"/);
  assert.equal(seen.length, 3);
});

test("rejects unconsented, oversized and cross-origin submissions before forwarding", async (t) => {
  const seen = upstream(t);
  assert.equal((await submit({ consent: "" })).status, 400);
  assert.equal((await submit({ description: "x".repeat(100_000) })).status, 413);
  assert.equal((await submit({}, env, "https://different.example.com")).status, 403);
  assert.equal(seen.length, 0);
});

test("rejects a verified token issued on a different hostname and retains the escaped draft", async (t) => {
  const seen = upstream(t, { challenge: { hostname: "different.example.com" } });
  const response = await submit();
  const html = await response.text();
  assert.equal(response.status, 400);
  assert.match(html, /导出失败/);
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.equal(seen.length, 1);
});

test("a per-client limit rejects the request before calling the verification service", async (t) => {
  const seen = upstream(t);
  const response = await submit({}, { ...env, FEEDBACK_RATE_LIMIT: { limit: async () => ({ success: false }) } });
  assert.equal(response.status, 429);
  assert.equal(seen.length, 0);
});

test("an upstream failure keeps the draft and does not retry creating the issue", async (t) => {
  const seen = upstream(t, { issueStatus: 500 });
  const response = await submit();
  assert.equal(response.status, 502);
  assert.match(await response.text(), /value="导出失败"/);
  assert.equal(seen.length, 3);
});
