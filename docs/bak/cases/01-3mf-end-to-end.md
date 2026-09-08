# Case: 3MF End-to-End Verification

> First verification record following [the template](TEMPLATE.md). Closes tracker P0-4.

## Meta

| Field | Value |
|-------|-------|
| Date | 2026-08-27 |
| Model | figure bust (shirt / skin / hair / glasses) — ⚠️ **provenance & license not yet documented** (tracked in `samples/` README note); replace with a self-made model before wider distribution if unresolved |
| Face count | TODO (read from CYM HUD on re-run) |
| CYM version / commit | `a7700df` era (fill-routing fix verified in the same session) |
| Slicer used | Snapmaker Orca, Snapmaker U1 profile |
| Evidence | `samples/liangsheng/` (CYM workflow shots; the slicer-import shots of the original run were retired in the 2026-09 samples reorganisation) |

## Goal

Does the full loop hold: CYM segmentation → painting → 3MF export → slicer shows the correct multi-filament, multi-colour, machine-configured job?

## Pipeline & Settings

1. Import STL; seed-based segmentation (Seed panel: recommend + grow + fuse, eye detection on)
2. Paint regions via the palette
3. Export 3MF with embedded machine preset (Snapmaker U1, 0.4 nozzle) — no vendor profiles pre-installed in the slicer

## Result

- Snapmaker Orca imported the 3MF with the model displayed in full colour.
- **Four filament slots** matched the painted palette (shirt / skin / hair / accent).
- The **process dropdown showed the embedded preset** `0.20 Standard @Snapmaker U1 (0.4 nozzle)` — proving the compiled-in `machine_settings_1.config` path works without any vendor installation.
- Maintainer's verdict on record: *"当前的3mf导出是合理的"* → [tracker P0-4 closed](../04-缺陷与遗留问题清单.md).

## Verification checklist

- [x] Regions as expected (seed panel output, 13 regions post tiny-merge in the same session)
- [x] Colours match palette slots in slicer
- [x] Machine & process preset recognized by slicer
- [ ] Printed output photo (pending a real print)

## Problems found

- None blocking. Written verification protocol (slicer version pinning, assertion checklist per run) is this page's TODO — future re-verifications should extend this case instead of starting from scratch.
