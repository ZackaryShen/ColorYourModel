# Auto Segmentation

> Part of the CYM wiki — how to use the automatic segmentation algorithms.
> For the math and pipeline internals, see [Algorithms: Mesh Segmentation](../algorithms/segmentation.md).

## Where

After importing a model (and at any time later) you can segment it:

- **On import**: a default dihedral-angle segmentation (30°) runs automatically.
- **Intelligent Segmentation panel**: choose an algorithm and tune its parameters.

## Algorithms in the panel

The panel exposes 3 of the 8 backend algorithms (`auto_segment_v2` IPC, all algorithms available to developers — see the [IPC Reference](../developer/ipc-reference.md)):

| Algorithm | Parameters (default) | Good for |
|-----------|----------------------|----------|
| **Dihedral** | angle threshold (30°) | mechanical parts, hard edges — splits where faces meet at steep angles |
| **Curvature K-Means** | clusters k (6), normal smoothing iterations (2), use SDF fusion (on), crease threshold (45°) | organic models with smooth patches |
| **Shape Diameter (SDF)** | k (0 = auto) | thickness-based separation (limbs, shells) |

```mermaid
flowchart LR
    P["Intelligent Segmentation panel"] -->|"select + tune"| A["auto_segment_v2<br/>(algorithm, params)"]
    A --> R["regions replace or join<br/>the current segmentation"]
```

## Behaviour notes

- **Manual regions survive re-runs** — segmentation preserves manually painted regions by default (`preserve_manual`, can be disabled).
- **Parameters persist per algorithm** — switching algorithms and coming back keeps the sliders where you left them (validated on restore; stored in `localStorage` via zustand persist).
- **Progress** — long segmentations emit progress events shown in the UI.
- **Single-region result?** If everything merges into one region, your thresholds are too permissive — raise the dihedral angle for hard-surface models or increase k for clustering algorithms.

## Region management afterwards

Any segmentation is a starting point: use the Segments panel and canvas tools to **merge / split / rename** regions, **resegment a single region** with a different algorithm, or refine with seeds (next page).
