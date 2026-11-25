# Multi-plate acceptance implementation plan

**Goal:** Complete the existing approved 3MF design's P1 multi-plate tabs and whole-project save, then repeat adversarial and native UI acceptance.
**Spec:** ../specs/2026-11-15-3mf-project-format-design.md; user explicitly confirmed P1 on 2026-11-17.
**Architecture:** One backend workspace owns stable monotonically increasing tab IDs. All mesh IPCs require explicit tabId and atomically reject stale IDs; one lock owns workspace and each tab's mesh/history/hidden labels. Clean inactive tabs retain source references and load on demand; edited tabs remain resident (P2 dirty swapping is excluded). UI retains lightweight per-tab metadata; replacing mesh DTO resets transient drawing queues and releases old geometry. Export gathers all tabs, quantizes a joint palette, writes one object and matching engineering parts per plate using world-coordinate fingerprints.

## Global constraints
- Preserve all existing uncommitted P0 acceptance fixes; no commit/push without user request.
- Every mesh command carries tabId; stale responses and unsent painting cannot apply to another tab.
- STL appends a tab. 3MF replaces the project only after confirming any dirty tab. Close dirty tabs confirms; closing last tab clears viewport.
- One source configuration/palette per project; active plate, names, exact RGBA, labels, region names and undo/redo survive whole-project save/reopen.
- Plate geometry is local in UI; export uses ceil(sqrt(N)) grid, origins (col*bed_width*1.2,-row*bed_depth*1.2). Face identity hashes written world coordinates.
- No P2 dirty-memory swapping; clean inactive source tabs must be lazy. Real inputs plus malformed project parts still degrade per P0 rules.

## Contract
`TabInfo = { id:number, name:string, faceCount:number, dirty:boolean }`.
MeshData adds `tabId:number`, `tabs:TabInfo[]`; base to_dto defaults only used by backend tests. Frontend uses live tabId captured at operation initiation.
`load_model(path) -> MeshData`; STL append; 3MF atomic replace, restored active plate selected.
`switch_tab(tabId) -> MeshData`; `close_tab(tabId) -> MeshData|null`; `rename_tab(tabId,name) -> TabInfo[]`; `project_tabs() -> TabInfo[]`.
All existing mesh/paint/segment/history IPC add required `tabId`; whole-project export and joint palette preview also accept active tabId as stale-request guard.

## Tasks
- [x] 1. Backend workspace, lazy plate reader, explicit command guards, lifecycle commands and regression tests (stable IDs, atomic failed replacement/switch, per-tab hidden/history, STL append, empty/last close).
- [x] 2. Frontend workspace metadata/tab bar and IPC routing; dirty confirmation across all tabs; switch/close/rename with transient reset; drawing queue invalidation and stale-result rejection; tests.
- [x] 3. Whole-project joint palette/export, plate metadata/grid placement, per-plate engineering state/history and active/name roundtrip; real two/three-plate tests and saved source preset retention.
- [x] 4. Independent adversarial review/fix loops; full Rust/frontend/build checks; native GUI opening/switching/editing/close/cancel/save/reopen and drag-drop; update guide and unified loop journal.

## Review focus
1. Queued paint/segmentation/history result crosses a tab switch.
2. Save after editing an inactive tab drops its edits, history or source presets.
3. Failed lazy load or cancelled close discards the current tab or dirty flags.
4. Source plate metadata contradicts world positions, or plate origin/fingerprint differs on roundtrip.
5. Deleting/adding/renaming tabs changes plate order, identity or active plate unexpectedly.

## Rulings and evidence

- P1 follows the approved existing design; user explicitly included multi-plate tabs in acceptance. No new approval gate or separate feature branch is needed.
- Stable tab ID plus frontend view generation protects both old in-flight results and unsent work, including A→B→A switches. A final application check is required after Promise continuations.
- Closing the final tab clears both save destination and source presets. Closing another tab preserves current hidden regions and marks project structure unsaved.
- Negative/centered STL geometry must be placed on a slicer plate for world-position membership. Preserve valid placement; otherwise record an additive engineering placement offset and reverse it only after fingerprint-matched engineering restore. Hash written world coordinates throughout.
- Task1: Rust339passed/20ignored; workspace11/11; realreader2/2. Task2: initial independent review found asynchronous application races, stale history rows, hidden-state reset, final-close save path; these were fixed with regressions and are being re-reviewed.

## Final acceptance checkpoint

- Final independent review: Spec passed and Quality passed after fixing lazy-plate state-loss warnings and Export→Save path/config divergence.
- GUI follow-up fixed duplicate sibling keys leaving empty sidebars and missing region cleanup after last close; scoped review passed.
- Native verified: three tabs; per-plate fill/undo/redo; Chinese tab rename; dirty inactive close cancel; full SaveAs and reopen active2 with three objects, exact colors and histories; hidden-region isolation; close-one then full Save to two plates; source temporarily missing preserves active model and recovery works; STL append; fresh STL first Save asks new filename; final close clears viewport/regions/path.
- Automated: frontend217/217; Rust353passed/21ignored; TypeScript/frontend and native dev builds passed; docs37pages/0brokenlinks.
- Acceptance completed under the user's revised criterion: file-picker import is sufficient; native cross-window drag is not a blocking requirement. The already verified native file-picker import of three plates is the accepted evidence. Native drag itself remains unverified and is not claimed to pass.
- No commit or push. P2 dirty-tab swapping, Windows/Linux and printer tests were not in this acceptance pass.

## User scope resolution

2026-11-17: User explicitly said “你可以使用导入的方式，不一定要拖拽”. Native import/three-tab editing/save-reopen acceptance is therefore complete; no native drag success is claimed. The temporary acceptance app and its Vite server were stopped after the handoff ended. All implementation changes remain uncommitted and unpushed.

## Continued adversarial acceptance — 2026-11-17

- Independent reviews found two additional counterexamples: a remounted plate view reused stroke IDs and coalesced separate drags into one undo entry; failed saves bypassed eviction of temporarily loaded clean plates.
- Fixed the stroke allocator to belong to the renderer session, and unified save cleanup across preset rejection, partial lazy loading failure and writer failure. Red/green regressions cover both.
- Delayed-response tests confirmed another isolation issue: detection catches returned unowned empty arrays after stale-result rejection. All five detection commands now propagate failure to existing SeedPanel catch handlers, preserving the current plate's overlays and status.
- Native GUI verified replacement cancel, corrupt 3MF replacement preserving all three tabs, failed-import recovery, inactive dirty tab exit confirmation, and A(first blue 8-face fill) → B → A(second yellow 2-face fill) → Undo restoring only the second fill while the first remains blue.
- Cross-review of frontend and backend fixes found no further confirmed production blocker. The renderer-session stroke counter also survives development hot-module reload, with a red/green module-reload regression.
- No commit/push; temporary acceptance app and its own Vite server stopped after visual verification.

- Final continued-round validation: frontend 224/224 (217 original + 7); Rust 355 passed / 21 ignored (353 original + 2); frontend and native dev builds passed; git diff --check passed.

## Practical product audit beyond Tabs and the spec checklist — 2026-11-17

User explicitly requested review of real usability/correctness issues the spec did not anticipate. This pass examined current product workflows, including existing defects exposed through the branch, rather than classifying every issue as newly introduced.

- Fixed unsafe filename rewriting: Save/Save As/Export use the exact current or native-confirmed path, preserving case, repeated extensions and extensionless filenames. A real-file regression proved the old implementation overwrote a neighbouring shorter filename.
- Fixed modal/text/IME shortcut leakage. Initial guards were themselves falsified by native GUI: after reset-modal cancellation, another listener still switched to View and cleared seeds. DialogShell now consumes Escape during capture; SeedPanel/ColorPanel and viewport model shortcuts share editing/modal guards. Real modal-unmount and SeedPanel tests cover listener ordering.
- Fixed export preset mismatch: changing nozzle validates per-slot and bulk materials; a saved non-default process is preserved. Delayed/stateful component tests demonstrated the old UI submitted PETG 0.4 after selecting nozzle 0.6.
- Fixed reset history flags so successful reset immediately clears Undo/Redo and refreshes timeline; corrected loaded-but-unsegmented empty guidance. Generic toast feedback no longer labels failures with a success checkmark.
- Fixed OBJ colour index overflow at palette size 256, companion MTL naming for exact paths, and writes that damaged an old MTL when OBJ failed. Both files stage before installation with backups/rollback; injected second-install failure restores both old files. This does not claim cross-file crash atomicity.
- Fixed engineering-save/print-fit coupling: 300mm models can save edit state without scaling or changing real bed settings; explicit print Export stays strict. Single/three-plate roundtrips restore geometry, labels, names, exact colours and histories. Non-finite extent/world-coordinate guards prevent malformed output.
- Native evidence: cancel reset retains one seed and current Seed tool; reset after painting immediately disables Undo; export/reopen retains non-default 0.20 Standard 0.4 process; switching/exporting 0.6 writes actual 0.6 machine/nozzle/material settings; 300x300x30 model saves as 本轮超床保存.3mf.3mf, reopens and undoes colour; neighbouring 本轮超床保存.3mf remains unchanged; strict print export rejects and creates no output.
- Cross-review passed for paths, preset state, reset history, OBJ transaction/colour mapping, keyboard final fixes and oversized engineering-save geometry. No confirmed unresolved blocker from the reviewed workflows.
- Automated final: frontend239/239 (224+15); Rust364passed/21ignored (355+9); frontend/native builds passed; docs37pages/0brokenlinks; diff check passed. macOS scope; no claim of Windows/Linux, printer validation or cross-file crash recovery.
- Temporary acceptance app and this pass's Vite server stopped; all changes remain uncommitted and unpushed.
