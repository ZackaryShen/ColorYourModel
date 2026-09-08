# Contributing to ColorYourModel

Thanks for your interest in improving CYM! This page covers everything needed
to go from clone to merged PR.

## Development setup

Prerequisites:

- **Node.js** ≥ 18 (LTS recommended) + npm
- **Rust** (stable, with the MSVC toolchain on Windows)
- **Tauri 2 system dependencies** — see the
  [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.
  The project is developed primarily on Windows; macOS/Linux builds are
  expected to work but are not exercised daily.

```bash
npm install          # frontend deps
npm run tauri dev    # vite on :1420 + debug build of the Rust backend
```

The first `tauri dev` compiles the dependency tree — later launches are
incremental. Closing the app window ends the dev task.

## Tests and checks

Run these before opening a PR (CI runs the frontend/docs side):

```bash
cargo test --lib                    # Rust unit tests (src-tauri/)
npx tsc --noEmit                    # TypeScript type check
npx vitest run                      # frontend unit tests
npm run docs:build                  # regenerate docs HTML archives
```

Notes:

- `cargo test --release --lib <probe> -- --ignored --nocapture` runs the
  in-repo evidence probes (they read large STL fixtures from disk and are
  `#[ignore]` by default). If you touch a segmentation algorithm, please add
  or extend a probe so the effect is measurable, not vibes.
- If a test fails on `main` too, say so in the PR and reference the exact
  failure — a known-failure baseline (stash comparison) beats a mystery.

## How we work

- **Branches**: `feature_*` / `bugfix_*` off the default branch; PRs are
  squash-merged.
- **Commits**: [Conventional Commits](https://www.conventionalcommits.org/),
  English, atomic — one logical change per commit, with the test evidence in
  the body (pass counts, probe numbers).
- **Adversarial loop**: behavioural changes go through
  `PLAN → REFUTE → REVISE → IMPLEMENT → TEST → RETROSPECT`. In practice:
  write down the approach *before* implementing it, try to falsify it (or have
  a reviewer try), and keep the numbers that motivated the change in the
  commit message.
- **Performance claims need evidence**: mesh operations here run on up to
  1.5M-face models. A "faster" PR should include before/after timings from a
  probe, run in the profile users actually feel.

## Documentation

Docs are part of the repo, not an afterthought:

- Markdown sources live in [`docs/bak/`](docs/bak/README.md), mirroring the
  docs tree. After editing: `npm run docs:build` regenerates the official HTML
  archives, prunes stale pages and asserts every link — **commit both** the
  `.md` and the regenerated `.html`. CI fails when the HTML drifts.
- User-facing behaviour changes (a new slider, a renamed button, different
  defaults) should update the matching page in `docs/bak/user-guide/` in the
  same PR.
- Screenshots for docs live under `samples/` — the repo tracks
  `samples/**/*.png` only; models and data files stay out (`.gitignore`).

## Reporting bugs

Open a GitHub issue with the bug-report template: app version, OS, model
(face count matters — many bugs only reproduce at 1M+ faces), the exact steps,
and screenshots if visual. Crash logs land in the app's log (see the crash
diagnostics note in the docs).

## License

By contributing you agree that your contributions are licensed under
[AGPL-3.0](LICENSE), the same terms as the project.
