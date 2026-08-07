/**
 * Types shared between the export dialog and the 3MF export IPC boundary.
 *
 * Field shapes mirror the Rust side: `ExportSelection` is serialized with
 * camelCase (serde `rename_all`), and the summaries come from
 * `presets::summary()` which drops the heavy config payloads.
 */

/** A machine the user can export for (from `list_export_presets`). */
export interface MachineSummary {
  id: string;
  vendor: string;
  name: string;
  /** Nozzle diameters, e.g. ["0.2","0.4","0.6","0.8"]. */
  nozzles: string[];
  /** Slicer targets this machine can be exported for. */
  targetSlicers: string[];
  processes: ProcessSummary[];
  filaments: FilamentSummary[];
}

export interface ProcessSummary {
  name: string;
  /** Nozzle diameters this process is offered for. */
  nozzles: string[];
  layerHeight: string | null;
}

export interface FilamentSummary {
  name: string;
  nozzles: string[];
  /** PLA / PETG / ... (grouping label for the dialog). */
  material: string | null;
  vendor: string | null;
}

/**
 * The user's pick in the export dialog. Serialized to the `export_3mf_command`
 * IPC as camelCase; `None` on the Rust side falls back to the generic profile.
 */
export interface ExportSelection {
  machineId: string;
  nozzleDiameter: string;
  processName: string;
  /** One filament preset name per extruder slot, in slot order. */
  filamentNames: string[];
  /** `snapmaker_orca` | `orcaslicer` — decides which model id is written. */
  targetSlicer: string;
}

/** A validated export selection persisted between sessions. */
export interface PersistedExportSelection {
  machineId: string;
  nozzleDiameter: string;
  processName: string;
  /** Per-slot names; may be shorter than the palette (last one repeats). */
  filamentNames: string[];
  targetSlicer: string;
}
