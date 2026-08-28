# Development Guide

> Part of the CYM wiki — how the repo is organized and how we work on it.

## Stack & layout

Tauri 2 desktop app: React 18 + TypeScript + Three.js frontend (`src/`), Rust backend (`src-tauri/`).
The full annotated tree lives in the [README](../../README.md#project-structure); deep-dive stubs live under [algorithms/](../algorithms/) and [technical/](../technical/).

## Everyday commands

```bash
npm run tauri dev                  # dev app (vite HMR + debug exe)
npx tsc --noEmit                   # TypeScript check
npx vitest run                     # frontend unit tests
cd src-tauri && cargo test --lib   # backend unit + regression tests
npm run docs:build                 # regenerate HTML archives after editing docs
```

Pre-commit convention: if tests fail, compare against a stash baseline to prove the failure pre-dates your change.

## Docs workflow (this wiki)

Markdown sources live in `docs/bak/` — a mirror of the `docs/` tree — plus the repo-root trio (`README.md`, `README.zh-CN.md`, `CHANGELOG.md`, which GitHub renders directly). The committed `*.html` pages under `docs/` (and the root `README.html` etc.) are generated from them and are the official, readable archive. After editing any md:

```bash
npm run docs:build     # regenerates archives, prunes stale ones, asserts links
git add -A '*.html' docs/   # include regenerated archives
```

- Never edit `*.html` by hand — the next build overwrites it; the md is the single source.
- `.github/workflows/docs-check.yml` fails CI when archives are stale.
- `.github/workflows/deploy-docs.yml` publishes the HTML to GitHub Pages (manual trigger; enable Pages with Source = "GitHub Actions" first).
- Diagrams are Mermaid fenced blocks — rendered by GitHub and by the HTML archives alike.
- UI-language note: the app UI defaults to **Chinese** with English available (`src/i18n.ts`); docs skeletons are English, content may be written in either language.

## Tools

| Script | Purpose |
|--------|---------|
| `tools/build_docs.mjs` | docs → HTML archive generator (marked + github-slugger) |
| `tools/extract_orca_presets.py` | regenerate `src-tauri/resources/presets/snapmaker_u1.json` from an OrcaSlicer vendor tree (whitelisted vendors, `inherits` chain flattening) |
| `tools/verify_3mf.py` | structural verification of an exported 3MF |
| `tools/verify_fill_routing.cjs` | fill-routing regression helper |

## Conventions

- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/), English, atomic; commit messages carry test evidence (pass counts).
- **Branches**: `feature_*` / `bugfix_*`; PRs squash-merged to `main`.
- **Adversarial development loop**: every non-trivial change passes `PLAN → REFUTE → REVISE → IMPLEMENT → TEST → RETROSPECT`; per-round records live in [CHANGELOG.md](../../CHANGELOG.md).
- **Region authority principle**: region **labels** are the truth, colours are output — algorithms must never use colour to decide region identity (see [Fill Routing](../technical/fill-routing.md)).
- **Thresholds carry provenance**: every magic number in a patch needs a source comment or an explicit "v1 default" marker.

## Release status

No release tags yet; v0.1 core loop is functionally complete with 3MF export verified end-to-end (2026-08). The defect tracker with P0/P1/P2 states lives in [docs/04](../04-缺陷与遗留问题清单.md) (中文).
