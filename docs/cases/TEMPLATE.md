# Case: <short title>

> Copy this file to `<name>.md` and fill it. Write in English or 中文.

## Meta

| Field | Value |
|-------|-------|
| Date | YYYY-MM-DD |
| Model | file name + source + **license/provenance** (must be documented before publishing screenshots) |
| Face count | e.g. 1.5M |
| CYM version / commit | git sha |
| Slicer used | e.g. Snapmaker Orca (U1), version |
| Evidence | screenshot paths under `samples/` or `examples/` |

## Goal

What question this case answers (e.g. "does a fused seed segmentation survive 3MF export with correct filament slots?").

## Pipeline & Settings

Each step with its exact parameters (algorithm, thresholds, brush settings). Enough for someone else to reproduce.

## Result

What happened, with screenshots. Include the slicer-side view, not just the CYM side.

## Verification checklist

- [ ] Regions as expected (count / boundaries)
- [ ] Colours match palette slots in slicer
- [ ] Machine & process preset recognized by slicer
- [ ] Printed output (if applicable): photo

## Problems found

Anything that didn't work → file to the defect tracker (`../04`) and link it here.
