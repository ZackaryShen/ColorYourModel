#!/usr/bin/env node
//! Build the static HTML docs site from markdown sources.
//!
//! Markdown files remain the source of truth (GitHub renders them, including
//! fenced ```mermaid blocks). This script renders every doc into a modern
//! static page under `docs/site/` — browsable by double-click (file://, no
//! fetch, classic scripts only) and deployable to GitHub Pages as-is.
//!
//! Determinism: output contains no timestamps, no absolute paths, so
//! `git status docs/site` after a rebuild is a valid freshness check (CI).
//!
//! Usage: npm run docs:build

import { readFileSync, writeFileSync, mkdirSync, rmSync, existsSync, readdirSync, statSync } from "node:fs";
import { join, dirname, relative, resolve, sep, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import { execFileSync } from "node:child_process";
import { Marked } from "marked";
import GithubSlugger from "github-slugger";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SITE = join(ROOT, "docs", "site");
const SITE_REL_POSIX = "docs/site";

// ── Source collection ────────────────────────────────────────────────────────

/** Walk `docs/` for .md sources, skipping the generated site itself. */
function collectDocs(dir) {
  const out = [];
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      if (full === SITE) continue;
      out.push(...collectDocs(full));
    } else if (entry.endsWith(".md")) {
      out.push(full);
    }
  }
  return out;
}

const SOURCES = [
  join(ROOT, "README.md"),
  join(ROOT, "README.zh-CN.md"),
  join(ROOT, "CHANGELOG.md"),
  ...collectDocs(join(ROOT, "docs")),
];

// Output path (posix, relative to SITE) for each source. README.md becomes the
// site index; every other file keeps its relative path with a .html suffix.
const outRel = (src) => {
  const relFromRoot = relative(ROOT, src).split(sep).join("/");
  if (relFromRoot === "README.md") return "index.html";
  return relFromRoot.replace(/\.md$/, ".html");
};

// Map: absolute source path -> output posix path. Directory entries map to
// their README page so sidebar/inline directory links stay inside the site.
const OUT_MAP = new Map(SOURCES.map((src) => [src, outRel(src)]));
for (const src of SOURCES) {
  const dir = dirname(src);
  if (dir !== ROOT && !OUT_MAP.has(dir)) OUT_MAP.set(dir, outRel(join(dir, "README.md")));
}

// ── Markdown rendering ───────────────────────────────────────────────────────

const SLUG_STRIP = /[`*_~[\]]/g;

/** Render one markdown file to article HTML. Headings get github-slugger ids
 *  (CJK-safe); TOC is extracted from the rendered ids afterwards so the two
 *  can never diverge. */
function renderArticle(src) {
  const raw = readFileSync(src, "utf8");
  const slugger = new GithubSlugger();

  const marked = new Marked({ gfm: true });
  marked.use({
    renderer: {
      heading(token) {
        const text = this.parser.parseInline(token.tokens);
        const id = slugger.slug(String(token.text).replace(SLUG_STRIP, ""));
        return `<h${token.depth} id="${id}">${text}</h${token.depth}>\n`;
      },
    },
  });

  let html = marked.parse(raw);

  // Fenced ```mermaid blocks → <pre class="mermaid"> (keep marked's HTML
  // escaping; mermaid reads textContent, which unescapes it back).
  html = html.replace(
    /<pre><code class="language-mermaid">([\s\S]*?)<\/code><\/pre>/g,
    '<pre class="mermaid">$1</pre>',
  );
  return html;
}

/** Extract TOC entries ([id, text, depth]) from rendered h2/h3. Inner text is
 *  entity-decoded here; templates re-escape on output (avoids &amp;amp;). */
function extractToc(html) {
  const toc = [];
  const re = /<h([23]) id="([^"]*)">(.*?)<\/h\1>/g;
  const decode = (s) => s
    .replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"').replace(/&#39;/g, "'");
  let m;
  while ((m = re.exec(html)) !== null) {
    toc.push([m[1], m[2], decode(m[3].replace(/<[^>]+>/g, ""))]);
  }
  return toc;
}

// ── Link rewriting ───────────────────────────────────────────────────────────

const REPO_URL = repoUrl();

function repoUrl() {
  const url = execFileSync("git", ["remote", "get-url", "origin"], { cwd: ROOT })
    .toString()
    .trim()
    .replace(/\.git$/, "");
  const ssh = url.match(/^git@github\.com:(.+)$/);
  return ssh ? `https://github.com/${ssh[1]}` : url;
}

const blob = (repoPath) => `${REPO_URL}/blob/main/${repoPath.replace(/\\/g, "/")}`;

/** Rewrite relative hrefs in rendered HTML so navigation works inside the
 *  site: .md → mapped .html page, directories → their README page, stray
 *  repo files (LICENSE) → GitHub blob. Absolute/anchor/mailto untouched. */
function rewriteLinks(html, src) {
  const pageOut = OUT_MAP.get(src); // e.g. "docs/algorithms/segmentation.html"
  return html.replace(/href="([^"]*)"/g, (full, href) => {
    if (/^(https?:|mailto:|#|data:)/.test(href)) return full;

    let targetAbs; // absolute repo path of a source md or directory
    if (href.endsWith(".md")) {
      targetAbs = resolve(dirname(src), href);
    } else if (!/\.[a-zA-Z0-9]+$/.test(href.replace(/[/?].*$/, "")) || href.endsWith("/")) {
      // directory-style link (no file extension)
      targetAbs = resolve(dirname(src), href);
      if (!OUT_MAP.has(targetAbs) && !targetAbs.endsWith(".md")) {
        return `href="${blob(relative(ROOT, targetAbs))}"`;
      }
    } else {
      // non-markdown repo file (LICENSE, …) → GitHub blob
      return `href="${blob(relative(ROOT, resolve(dirname(src), href)))}"`;
    }

    const targetOut = OUT_MAP.get(targetAbs);
    if (!targetOut) {
      // Unknown markdown target — fall back to GitHub blob rather than a 404
      // inside the site.
      return `href="${blob(relative(ROOT, targetAbs))}"`;
    }
    const rel = relative(dirname(pageOut), targetOut).split(sep).join("/");
    return `href="${encodeURI(rel)}"`;
  });
}

// ── Page template ────────────────────────────────────────────────────────────

const NAV_GROUPS = [
  ["Overview", ["README.md", "README.zh-CN.md", "CHANGELOG.md"]],
  ["Algorithms", ["docs/algorithms/README.md", "docs/algorithms/segmentation.md", "docs/algorithms/seed-grow-fuse.md", "docs/algorithms/eye-detection.md"]],
  ["Technical", ["docs/technical/README.md", "docs/technical/bvh-face-picking.md", "docs/technical/shader-segment-highlight.md", "docs/technical/fill-routing.md", "docs/technical/undo-redo-history.md", "docs/technical/export-pipeline.md", "docs/technical/crash-diagnostics.md"]],
  ["Cases", ["docs/cases/README.md", "docs/cases/TEMPLATE.md"]],
  ["Working docs · 中文", SOURCES.filter((s) => /docs[/\\]0\d-/.test(s)).sort()],
  ["Internal", ["docs/loop-journal.md"]],
];

const PAGE_TITLES = new Map(
  SOURCES.map((src) => {
    const h1 = readFileSync(src, "utf8").match(/^#\s+(.+)$/m);
    const fallback = relative(ROOT, src).split(sep).join("/");
    let title = h1 ? h1[1].trim() : fallback;
    if (src.endsWith("TEMPLATE.md")) title += " (template)";
    return [src, title];
  }),
);

const navLabel = (src) => {
  const base = relative(ROOT, src).split(sep).join("/");
  if (base === "README.md") return "Home (EN)";
  if (base === "README.zh-CN.md") return "首页（中文）";
  let t = PAGE_TITLES.get(src);
  if (src.endsWith("TEMPLATE.md")) t = "Case template";
  return t.length > 42 ? t.slice(0, 41) + "…" : t;
};

/** NAV_GROUPS mixes absolute paths (filtered SOURCES) with repo-relative
 *  strings — normalize both to absolute source paths. */
const toAbs = (p) => (isAbsolute(p) ? p : join(ROOT, p));

function buildSidebar(currentSrc) {
  const groups = NAV_GROUPS.map(([label, srcs]) => {
    const items = srcs
      .map((rawSrc) => {
        const src = toAbs(rawSrc);
        const out = OUT_MAP.get(src);
        const rel = relative(dirname(OUT_MAP.get(currentSrc)), out).split(sep).join("/");
        const cur = src === currentSrc ? ' aria-current="page"' : "";
        return `<li><a href="${encodeURI(rel)}"${cur}>${escapeHtml(navLabel(src))}</a></li>`;
      })
      .join("\n");
    return `<div class="nav-group"><div class="nav-group-title">${label}</div><ul>${items}</ul></div>`;
  }).join("\n");
  return `<nav id="sidebar-nav">${groups}</nav>`;
}

const escapeHtml = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

const LANG_PAIR = new Map([
  [join(ROOT, "README.md"), "README.zh-CN.md"],
  [join(ROOT, "README.zh-CN.md"), "README.md"],
]);

function langToggle(src) {
  const other = LANG_PAIR.get(src);
  if (!other) return "";
  const rel = relative(dirname(OUT_MAP.get(src)), OUT_MAP.get(join(ROOT, other))).split(sep).join("/");
  const label = src.endsWith(".zh-CN.md") ? "English" : "简体中文";
  return `<a class="lang-toggle" href="${encodeURI(rel)}">${label}</a>`;
}

function pageCss() {
  return readFileSync(join(ROOT, "tools", "docs_site.css"), "utf8");
}

function renderPage(src, article, toc) {
  const title = PAGE_TITLES.get(src);
  const tocHtml = toc.length
    ? `<details class="toc" open><summary>On this page</summary><ul>${toc
        .map(([d, id, text]) => `<li class="toc-h${d}"><a href="#${id}">${escapeHtml(text)}</a></li>`)
        .join("")}</ul></details>`
    : "";
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)} · ColorYourModel Docs</title>
<link rel="icon" href="data:image/svg+xml,<svg xmlns=%22http://www.w3.org/2000/svg%22 viewBox=%220 0 100 100%22><text y=%22.9em%22 font-size=%2290%22>🎨</text></svg>">
<style>
${pageCss()}
</style>
</head>
<body>
<header id="topbar">
  <button id="menu-btn" aria-label="Toggle navigation">☰</button>
  <a class="brand" href="${encodeURI(relative(dirname(OUT_MAP.get(src)), "index.html"))}">🎨 <strong>ColorYourModel</strong> <span>Docs</span></a>
  <div class="topbar-right">
    ${langToggle(src)}
    <a class="gh-link" href="${REPO_URL}" title="GitHub repository">GitHub ↗</a>
  </div>
</header>
<div id="layout">
  <aside id="sidebar">
    <input id="doc-filter" type="search" placeholder="Filter documents…" autocomplete="off">
    ${buildSidebar(src)}
    <div class="sidebar-foot">Generated from markdown ·<br><code>npm run docs:build</code></div>
  </aside>
  <main id="content">
    <article>
${article}
    </article>
    ${tocHtml}
  </main>
</div>
<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
<script>
window.addEventListener("DOMContentLoaded", function () {
  if (window.mermaid) {
    mermaid.initialize({ startOnLoad: true, theme: "base", securityLevel: "loose",
      themeVariables: { primaryColor: "#eef2ff", primaryTextColor: "#1e293b", primaryBorderColor: "#c7d2fe",
        lineColor: "#64748b", fontSize: "14px",
        fontFamily: "-apple-system, Segoe UI, PingFang SC, Microsoft YaHei, sans-serif" },
      flowchart: { curve: "basis", padding: 12, htmlLabels: true } });
  } else {
    document.querySelectorAll("pre.mermaid").forEach(function (el) { el.classList.add("mermaid-offline"); });
  }
  var filter = document.getElementById("doc-filter");
  if (filter) filter.addEventListener("input", function () {
    var q = this.value.toLowerCase();
    document.querySelectorAll("#sidebar-nav li").forEach(function (li) {
      li.style.display = li.textContent.toLowerCase().indexOf(q) >= 0 ? "" : "none";
    });
  });
  var btn = document.getElementById("menu-btn");
  if (btn) btn.addEventListener("click", function () {
    document.getElementById("sidebar").classList.toggle("open");
  });
});
</script>
</body>
</html>
`;
}

// ── Build & validate ─────────────────────────────────────────────────────────

rmSync(SITE, { recursive: true, force: true });
const pages = [];
for (const src of SOURCES) {
  const article = rewriteLinks(renderArticle(src), src);
  const outPath = join(SITE, OUT_MAP.get(src));
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, renderPage(src, article, extractToc(article)));
  pages.push(OUT_MAP.get(src));
}

// Existence check: every in-site relative href must resolve to a produced
// file (grep alone can't prove a rewritten link is alive).
let broken = 0;
for (const page of pages) {
  const html = readFileSync(join(SITE, page), "utf8");
  const pageDir = dirname(page);
  for (const m of html.matchAll(/href="([^"]*)"/g)) {
    const href = m[1];
    if (/^(https?:|mailto:|#|data:)/.test(href)) continue;
    const target = resolve(join(SITE, pageDir), decodeURIComponent(href));
    if (!existsSync(target)) {
      console.error(`BROKEN LINK: ${page} -> ${href}`);
      broken++;
    }
  }
}

const mermaidPages = pages.filter((p) =>
  readFileSync(join(SITE, p), "utf8").includes('class="mermaid"'),
).length;

if (broken) {
  console.error(`\n${broken} broken link(s) — site NOT valid.`);
  process.exit(1);
}
console.log(`OK: ${pages.length} pages, ${mermaidPages} pages with mermaid diagrams, 0 broken links.`);
console.log(`Open ${SITE_REL_POSIX}${sep}index.html in a browser.`);
