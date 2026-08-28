# Painting Tools

> Part of the CYM wiki — the colouring toolbox and everything around it.

## The tools

| Tool | What it does |
|------|--------------|
| **View** | orbit / inspect (painting disabled) |
| **Brush** | paint faces under the cursor with radius + strength |
| **Spray** | scattered dots inside the radius (density setting) — organic texture |
| **Smart brush** | like the brush, but respects region boundaries |
| **Fill** | paints the **whole region you click** (see below) |
| **Eyedropper** | pick the colour under the cursor into the palette |
| **Eraser** | strip colour back to the default |
| **Segment brush** | drag across faces, release to create a new region |
| **Lasso** | draw a closed loop of points; the enclosed area becomes a region |
| **Seed** | place seeds for the Seed panel (see [Seed Tools](seed-tools.md)) |

Tools are mutually exclusive — selecting one deactivates the others.

## Fill is region-authoritative

Clicking with Fill paints exactly the region the clicked face belongs to. Two consequences worth knowing:

- **Fast**: one click colours even a million-face region.
- **Safe with repeated colours**: region identity comes from the region **label**, never the colour — filling region B red while region A is already red does not touch A. This is regression-tested ([why](../technical/fill-routing.md)).

## Undo / redo

One unified timeline covers brush, spray, fill, erase and manual region edits — `Ctrl+Z` / `Ctrl+Y` (or the toolbar buttons) walk it back and forth. Backend-enforced, so it stays consistent across tools.

## Viewing and navigating

- **Segment view** — toggle region-coloured visualization with yellow boundary outlines.
- **Hover highlight** — hovering with fill/picker/segment tools tints the region under the cursor (GPU shader, instant even on 1.5M-face models). Highlight is display-only; it never changes what a tool targets.
- **Brush cursor** — a 3D ring follows the mouse showing brush radius and colour.
- **Space to pan** — hold `Space` to pan the camera, release to go back to orbit.

## Colours

Pick palette colours from the colour panel; the palette slots map to filament slots at export time (see [Exporting](exporting.md)).
