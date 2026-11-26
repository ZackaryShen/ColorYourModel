import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { captureTabOwnership, invokeTab, isCurrentTabOwnership } from "../../utils/tabCommand";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { useT } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import type {
  ExportPalettePreview,
  ExportSelection,
  MachineSummary,
  Rgb,
  TextureMappingPreview,
  TextureMappingRequest,
  TextureMappingSuggestions,
} from "../../types/export";
import { log } from "../../utils/logger";
import {
  fromHex,
  rgbHex,
  TextureMappingTable,
  TextureMappingViewport,
} from "./TextureMappingPanel";

const DEFAULT_TARGET = "snapmaker_orca";

function gradientPalette(
  preview: ExportPalettePreview | null,
  machine: MachineSummary | undefined,
  profile: ExportSelection | null,
) {
  if (profile?.targetSlicer !== DEFAULT_TARGET || !machine) return undefined;
  return preview?.gradientBySlotCount[machine.slotsByNozzle[profile.nozzleDiameter]];
}

function exportColors(
  preview: ExportPalettePreview | null,
  machine: MachineSummary | undefined,
  profile: ExportSelection | null,
) {
  return gradientPalette(preview, machine, profile)?.colors ?? preview?.flat ?? [];
}

function selectProfile(
  machine: MachineSummary,
  nozzleDiameter: string,
  slotCount: number,
  saved?: ExportSelection,
): ExportSelection {
  const processes = machine.processes.filter((process) => process.nozzles.includes(nozzleDiameter));
  const filaments = machine.filaments.filter((filament) => filament.nozzles.includes(nozzleDiameter));
  return {
    machineId: machine.id,
    nozzleDiameter,
    processName: processes.find(({ name }) => name === saved?.processName)?.name ?? processes[0]?.name ?? "",
    filamentNames: Array.from({ length: slotCount }, (_, i) =>
      filaments.find(({ name }) => name === saved?.filamentNames[i])?.name ?? filaments[0]?.name ?? ""),
    targetSlicer: saved && machine.targetSlicers.includes(saved.targetSlicer)
      ? saved.targetSlicer
      : machine.targetSlicers.find((target) => target === DEFAULT_TARGET) ?? machine.targetSlicers[0] ?? "",
  };
}

export function ExportDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { export3mf, exportObj } = useTauriCommand();
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setLastExportSelection = useAppStore((s) => s.setLastExportSelection);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const meshData = useAppStore((s) => s.meshData);
  const activeTabId = useAppStore((s) => s.activeTabId);
  const tabEpoch = useAppStore((s) => s.tabEpoch);
  const [owner] = useState(captureTabOwnership);
  const [sourceMesh] = useState(meshData);
  const isTexture = sourceMesh?.texturePainting === true;
  const [format, setFormat] = useState<"3mf" | "obj">("3mf");
  const [progress, setProgress] = useState(0);
  const [machines, setMachines] = useState<MachineSummary[]>([]);
  const [paintPreview, setPaintPreview] = useState<ExportPalettePreview | null>(null);
  const [selection, setSelection] = useState<ExportSelection | null>(null);
  const [exporting, setExporting] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  const [selectedSlots, setSelectedSlots] = useState<Set<number>>(new Set());
  const [bulkFilament, setBulkFilament] = useState("");

  const [mappingPreview, setMappingPreview] = useState<TextureMappingPreview | null>(null);
  const [assigned, setAssigned] = useState<ReadonlyMap<string, number>>(new Map());
  const [nominalColors, setNominalColors] = useState<(Rgb | null)[]>([]);
  const [suggestions, setSuggestions] = useState<{
    selection: ExportSelection;
    nominalColors: (Rgb | null)[];
    data?: TextureMappingSuggestions;
    error?: string;
  } | null>(null);
  const [suggestionAttempt, setSuggestionAttempt] = useState(0);

  // Export progress is delivered while the native writer is running.
  useEffect(() => {
    const unlisten = listen<{ progress: number; stage: string }>("export-progress", (e) => {
      setProgress(e.payload.progress);
    });
    return () => { void unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    if (!sourceMesh) return;
    let cancelled = false;
    const persisted = useAppStore.getState().lastExportSelection;
    const loadPreview = isTexture
      ? invokeTab<TextureMappingPreview>("texture_mapping_preview", {}, owner.tabId)
      : invokeTab<ExportPalettePreview>("export_palette_preview", {}, owner.tabId);

    void Promise.all([invoke<MachineSummary[]>("list_export_presets"), loadPreview]).then(([presets, preview]) => {
      if (cancelled) return;
      setMachines(presets);
      const accepted = "accepted" in preview ? preview.accepted : null;
      const preferred = accepted?.selection ?? persisted ?? undefined;
      const machine = presets.find(({ id }) => id === preferred?.machineId) ?? presets[0];
      if (!machine) {
        setLoadError("");
        return;
      }
      const saved = preferred?.machineId === machine.id ? preferred : undefined;
      const nozzle = machine.nozzles.find((n) => n === saved?.nozzleDiameter) ?? machine.nozzles[0] ?? "";
      if ("flat" in preview) {
        const profile = selectProfile(machine, nozzle, 0, saved);
        const slotCount = exportColors(preview, machine, profile).length;
        setSelection(selectProfile(machine, nozzle, slotCount, saved));
        setPaintPreview(preview);
        return;
      }
      const slotCount = machine.slotsByNozzle[nozzle] ?? 0;
      setSelection(selectProfile(machine, nozzle, slotCount, saved));
      setMappingPreview(preview);
      const matching = accepted?.selection.machineId === machine.id ? accepted : null;
      setNominalColors(Array.from({ length: slotCount }, (_, i) => matching?.nominalColors[i] ?? null));
      setAssigned(new Map(matching?.assignments
        .filter(({ slot }) => slot >= 1 && slot <= slotCount)
        .map(({ rgb, slot }) => [rgbHex(rgb), slot])));
    }).catch((error: unknown) => {
      if (cancelled) return;
      log.error("ExportDialog", "failed to load export settings", { error: String(error) });
      setLoadError(String(error));
    });
    return () => { cancelled = true; };
  }, [isTexture, owner, sourceMesh]);

  const machine = machines.find(({ id }) => id === selection?.machineId);
  const palette = exportColors(paintPreview, machine, selection);
  const gradient = gradientPalette(paintPreview, machine, selection);
  const nozzle = selection?.nozzleDiameter ?? "";
  const processes = machine?.processes.filter((process) => process.nozzles.includes(nozzle)) ?? [];
  const filaments = machine?.filaments.filter((filament) => filament.nozzles.includes(nozzle)) ?? [];
  const slotCount = isTexture ? machine?.slotsByNozzle[nozzle] ?? 0 : palette.length;
  const validProfile = !!selection?.processName
    && slotCount > 0
    && (!isTexture || (!!selection.targetSlicer && selection.filamentNames.every(Boolean)));

  useEffect(() => {
    if (!mappingPreview || !selection || !validProfile) return;
    let cancelled = false;
    void invoke<TextureMappingSuggestions>("suggest_texture_mapping", {
      selection,
      nominalColors,
      colors: mappingPreview.rows.map(({ rgb }) => rgb),
    }).then((data) => {
      if (!cancelled) setSuggestions({ selection, nominalColors, data });
    }).catch((error: unknown) => {
      if (!cancelled) setSuggestions({ selection, nominalColors, error: String(error) });
    });
    return () => { cancelled = true; };
  }, [mappingPreview, selection, nominalColors, validProfile, suggestionAttempt]);

  // A delayed response belongs to the spool settings that produced it.
  const currentSuggestions = suggestions?.selection === selection && suggestions.nominalColors === nominalColors
    ? suggestions : null;
  const slots = useMemo<TextureMappingSuggestions["slots"]>(() => currentSuggestions?.data?.slots
    ?? nominalColors.map((color, i) => ({ slot: i + 1, nominalColor: color, displayColor: color ?? [138, 138, 138] })),
  [currentSuggestions, nominalColors]);
  const sourceReplaced = !sourceMesh || !meshData || owner.tabId !== activeTabId || owner.epoch !== tabEpoch
    || meshData.vertices !== sourceMesh.vertices || meshData.faces !== sourceMesh.faces;
  const textureReady = !!mappingPreview?.rows.length && !!currentSuggestions?.data
    && mappingPreview.rows.every(({ rgb }) => assigned.has(rgbHex(rgb)));
  const canExport = !exporting && !sourceReplaced && (format === "obj" || (!!validProfile && (!isTexture || textureReady)));

  const chooseProfile = (next: MachineSummary, nextNozzle: string) => {
    if (next.id === selection?.machineId && nextNozzle === nozzle) return;
    const sameMachine = next.id === selection?.machineId;
    const prior = sameMachine ? selection ?? undefined : undefined;
    const profile = selectProfile(next, nextNozzle, 0, prior);
    const nextColors = exportColors(paintPreview, next, profile);
    const count = isTexture ? next.slotsByNozzle[nextNozzle] ?? 0 : nextColors.length;
    const saved = prior && !isTexture ? {
      ...prior,
      filamentNames: nextColors.map((color) => prior.filamentNames[palette.indexOf(color)] ?? ""),
    } : prior;
    setSelection(selectProfile(next, nextNozzle, count, saved));
    setSelectedSlots(new Set());
    setBulkFilament("");
    if (isTexture) {
      setNominalColors((previous) => Array.from({ length: count }, (_, i) => sameMachine ? previous[i] ?? null : null));
      setAssigned((previous) => new Map(sameMachine ? [...previous].filter(([, slot]) => slot <= count) : []));
    }
  };

  const chooseTarget = (targetSlicer: string) => {
    if (!selection || !machine) return;
    const profile = { ...selection, targetSlicer };
    if (isTexture) {
      setSelection(profile);
      return;
    }
    const colors = exportColors(paintPreview, machine, profile);
    const fallback = selectProfile(machine, nozzle, colors.length, profile).filamentNames[0];
    setSelection({
      ...profile,
      filamentNames: colors.map((color) => selection.filamentNames[palette.indexOf(color)] ?? fallback),
    });
    setSelectedSlots(new Set());
  };

  const setFilament = (slotIndex: number, name: string) => {
    setSelection((previous) => previous && {
      ...previous,
      filamentNames: previous.filamentNames.map((filament, i) => i === slotIndex ? name : filament),
    });
  };

  const bulkTarget = filaments.find(({ name }) => name === bulkFilament)?.name ?? filaments[0]?.name ?? "";
  const applyBulk = () => {
    if (!bulkTarget) return;
    setSelection((previous) => previous && {
      ...previous,
      filamentNames: previous.filamentNames.map((name, i) => !selectedSlots.size || selectedSlots.has(i) ? bulkTarget : name),
    });
  };

  const toggleSlot = (index: number) => {
    setSelectedSlots((previous) => {
      const next = new Set(previous);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  const assignSlot = (rgb: Rgb, slot: number) => {
    setAssigned((previous) => {
      const next = new Map(previous);
      if (slot === 0) next.delete(rgbHex(rgb));
      else if (slot >= 1 && slot <= slotCount) next.set(rgbHex(rgb), slot);
      return next;
    });
  };

  const suggestedAssignments = currentSuggestions?.data?.assignments;
  const applySuggestions = () => {
    if (!suggestedAssignments) return;
    setAssigned((previous) => {
      const next = new Map(previous);
      for (const { rgb, slot } of suggestedAssignments) {
        const key = rgbHex(rgb);
        if (!next.has(key)) next.set(key, slot);
      }
      return next;
    });
  };

  const doExport = async () => {
    const { importBusy, saveBusy, isLoading, pendingImportPath, pendingCloseTabId } = useAppStore.getState();
    if (!canExport || importBusy || saveBusy || isLoading || pendingImportPath || pendingCloseTabId != null) return;
    // Capture the confirmed revision and exact RGB mapping before the save dialog opens.
    const mappingRequest: TextureMappingRequest | undefined = format === "3mf" && selection && mappingPreview ? {
      revision: mappingPreview.revision,
      mapping: {
        selection,
        nominalColors,
        assignments: mappingPreview.rows.flatMap(({ rgb }) => {
          const slot = assigned.get(rgbHex(rgb));
          return slot === undefined ? [] : [{ rgb, slot }];
        }),
      },
    } : undefined;
    setExportError(null);
    useAppStore.setState({ saveBusy: true });
    setExporting(true);
    setProgress(0);
    try {
      const ext = format;
      const path = await save({
        filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
        defaultPath: `model.${ext}`,
      });
      if (!path) return;
      if (!isCurrentTabOwnership(owner)) throw new Error(t("export.modelChanged"));

      if (mappingRequest) {
        // The native overwrite prompt covers this exact path.
        await export3mf(path, mappingRequest.mapping.selection, false, mappingRequest);
      } else {
        if (format === "obj") await exportObj(path);
        else if (selection) await export3mf(path, selection);
      }
      if (format === "3mf" && selection) {
        setLastExportSelection(selection);
      }
      onClose();
    } catch (e) {
      log.error("ExportDialog", "export failed", { error: String(e) });
      setExportError(String(e));
      setStatusMessage(t("export.failure", String(e)));
    } finally {
      useAppStore.setState({ saveBusy: false });
      setExporting(false);
    }
  };

  // No model loaded: nothing to export. The Toolbar button and File menu both
  // disable their export entries, but this guard makes the empty state
  // explicit for any path that still opens the dialog (e.g. a model unloaded
  // while the dialog was already queued) instead of showing empty pickers.
  if (!sourceMesh) {
    return (
      <div style={styles.overlay} onClick={onClose}>
        <div
          style={styles.dialog}
          onClick={(e) => e.stopPropagation()}
          role="dialog"
          aria-modal="true"
        >
          <div style={styles.header}>{t("export.noModel")}</div>
          <div style={styles.body}>
            <div style={styles.note}>{t("export.noModelHint")}</div>
          </div>
          <div style={styles.footer}>
            <button className="cym-btn" style={styles.btn} onClick={onClose}>
              {t("export.cancel")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  const showMapping = format === "3mf" && isTexture;

  return (
    <div style={styles.overlay} onClick={() => { if (!exporting) onClose(); }}>
      <div
        style={{ ...styles.dialog, ...(showMapping ? styles.textureDialog : {}) }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t("export.title")}
      >
        <div style={styles.header}>
          {t("export.title")}
          {showMapping && (
            <button
              className="cym-btn texture-export-action"
              style={styles.themeButton}
              onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            >
              {t(theme === "dark" ? "menu.themeToLight" : "menu.themeToDark")}
            </button>
          )}
        </div>

        <div style={styles.body}>
          <fieldset className="export-dialog-fields" disabled={exporting}>
            <div style={styles.label}>{t("export.format")}</div>
            <div style={styles.row}>
              <button
                className="cym-btn"
                style={{ ...styles.fmtBtn, ...(format === "3mf" ? styles.fmtActive : {}) }}
                aria-pressed={format === "3mf"}
                onClick={() => { setFormat("3mf"); setExportError(null); }}
              >
                {t("export.format3mf")}
              </button>
              <button
                className="cym-btn"
                style={{ ...styles.fmtBtn, ...(format === "obj" ? styles.fmtActive : {}) }}
                aria-pressed={format === "obj"}
                onClick={() => { setFormat("obj"); setExportError(null); }}
              >
                {t("export.formatObj")}
              </button>
            </div>

            {showMapping && <p style={styles.note}>{t("export.textureScope")}</p>}
            {format === "obj" ? (
              <p style={styles.note}>{t("export.objNote")}</p>
            ) : (
              <div className={showMapping ? "texture-export-layout" : undefined}>
                {showMapping && mappingPreview && (
                  <TextureMappingViewport mesh={sourceMesh} preview={mappingPreview} assigned={assigned} slots={slots} />
                )}
                <div className={showMapping ? "texture-export-settings" : undefined}>
                  <label style={styles.label}>
                    {t("export.machine")}
                    <select
                      style={styles.select}
                      value={machine?.id ?? ""}
                      disabled={!machines.length}
                      onChange={(e) => {
                        const next = machines.find(({ id }) => id === e.target.value);
                        if (next) chooseProfile(next, next.nozzles[0] ?? "");
                      }}
                    >
                      {machines.map((m) => <option key={m.id} value={m.id}>{m.vendor} — {m.name}</option>)}
                    </select>
                  </label>

                  {machine && machine.nozzles.length > 1 && (
                    <>
                      <div style={styles.label}>{t("export.nozzle")}</div>
                      <div style={styles.row}>
                        {machine.nozzles.map((n) => (
                          <button
                            key={n}
                            className="cym-btn"
                            style={{ ...styles.nozzleBtn, ...(n === nozzle ? styles.nozzleActive : {}) }}
                            aria-pressed={n === nozzle}
                            onClick={() => chooseProfile(machine, n)}
                          >
                            {n}mm
                          </button>
                        ))}
                      </div>
                    </>
                  )}

                  <label style={styles.label}>
                    {t("export.process")}
                    <select
                      style={styles.select}
                      value={selection?.processName ?? ""}
                      onChange={(e) => selection && setSelection({ ...selection, processName: e.target.value })}
                      disabled={!processes.length}
                    >
                      {processes.map((process) => (
                        <option key={process.name} value={process.name}>
                          {process.name}{process.layerHeight ? ` (${process.layerHeight}mm)` : ""}
                        </option>
                      ))}
                    </select>
                  </label>

                  {showMapping ? (
                    <>
                      {mappingPreview && (
                        <TextureMappingTable
                          preview={mappingPreview}
                          slots={slots}
                          assigned={assigned}
                          onAssign={assignSlot}
                          onSuggest={applySuggestions}
                          canSuggest={suggestedAssignments?.some(({ rgb }) => !assigned.has(rgbHex(rgb))) ?? false}
                        />
                      )}
                      {slotCount > 0 && (
                        <details className="texture-export-spools">
                          <summary>{t("export.filament")} ({slotCount}{t("export.slots")})</summary>
                          <p>{t("export.textureNominalHint")}</p>
                          {slots.map(({ slot, nominalColor, displayColor }, i) => (
                            <div key={slot} className="texture-export-spool">
                              <label>
                                {t("export.textureSlot", slot)}
                                <select
                                  style={styles.select}
                                  value={selection?.filamentNames[i] ?? ""}
                                  onChange={(e) => setFilament(i, e.target.value)}
                                >
                                  {filaments.map(({ name }) => <option key={name} value={name}>{name}</option>)}
                                </select>
                              </label>
                              <label className="texture-export-nominal-colour" title={t("export.textureNominalColour", slot)}>
                                <input
                                  type="color"
                                  aria-label={t("export.textureNominalColour", slot)}
                                  value={rgbHex(displayColor)}
                                  onChange={(e) => {
                                    const rgb = fromHex(e.target.value);
                                    setNominalColors((previous) => previous.map((color, j) => i === j ? rgb : color));
                                  }}
                                />
                                {nominalColor ? rgbHex(nominalColor) : t("export.textureUnknown")}
                              </label>
                              {nominalColors[i] && (
                                <button
                                  className="cym-btn texture-export-action"
                                  aria-label={t("export.textureClearColour", slot)}
                                  onClick={() => setNominalColors((previous) => previous.map((color, j) => i === j ? null : color))}
                                >
                                  {t("export.textureResetColour")}
                                </button>
                              )}
                            </div>
                          ))}
                        </details>
                      )}
                      {loadError === null && (!mappingPreview || (validProfile && !currentSuggestions)) && (
                        <p role="status" className="texture-export-feedback">{t("export.textureLoading")}</p>
                      )}
                      {currentSuggestions?.error && (
                        <div className="texture-export-feedback" role="alert">
                          {t("export.textureSuggestionsFailed", currentSuggestions.error)}
                          <button className="cym-btn texture-export-action" onClick={() => {
                            setSuggestions(null);
                            setSuggestionAttempt((attempt) => attempt + 1);
                          }}>
                            {t("export.textureRetry")}
                          </button>
                        </div>
                      )}
                      {mappingPreview && !validProfile && (
                        <p role="alert" className="texture-export-feedback">{t("export.textureInvalidProfile")}</p>
                      )}
                    </>
                  ) : (
                    <>
                      <div style={styles.label}>{t("export.filament")} ({palette.length}{t("export.slots")})</div>
                      {gradient && <p style={styles.note} role="status">{t(
                        gradient.approximated ? "export.gradientApproximation" : "export.gradientRecipes"
                      )}</p>}
                      <div style={styles.bulkRow}>
                        <button
                          className="cym-btn"
                          style={styles.bulkBtn}
                          onClick={() => setSelectedSlots(selectedSlots.size === palette.length
                            ? new Set() : new Set(palette.map((_, i) => i)))}
                        >
                          {selectedSlots.size === palette.length ? t("export.deselectAll") : t("export.selectAll")}
                        </button>
                        <select
                          style={{ ...styles.select, flex: 1 }}
                          aria-label={t("export.filament")}
                          value={bulkTarget}
                          onChange={(e) => setBulkFilament(e.target.value)}
                        >
                          {filaments.map(({ name }) => <option key={name} value={name}>{name}</option>)}
                        </select>
                        <button className="cym-btn" style={{ ...styles.bulkBtn, ...styles.bulkApply }} onClick={applyBulk}>
                          {selectedSlots.size > 0 ? t("export.applyToSelected", selectedSlots.size) : t("export.applyToAll")}
                        </button>
                      </div>
                      {palette.map((colour, i) => (
                        <div key={i} style={styles.slotRow}>
                          <input
                            type="checkbox"
                            style={styles.slotCheck}
                            checked={selectedSlots.has(i)}
                            onChange={() => toggleSlot(i)}
                            aria-label={t("export.slotSelect", i + 1)}
                          />
                          <span style={{ ...styles.swatch, background: colour }} title={colour} />
                          <select
                            style={styles.select}
                            aria-label={t("export.textureSlot", i + 1)}
                            value={selection?.filamentNames[i] ?? ""}
                            onChange={(e) => setFilament(i, e.target.value)}
                          >
                            {filaments.map(({ name }) => <option key={name} value={name}>{name}</option>)}
                          </select>
                        </div>
                      ))}
                    </>
                  )}

                  <label style={styles.label}>
                    {t("export.target")}
                    <select
                      style={styles.select}
                      value={selection?.targetSlicer ?? ""}
                      onChange={(e) => chooseTarget(e.target.value)}
                    >
                      {(!isTexture || machine?.targetSlicers.includes("snapmaker_orca")) && (
                        <option value="snapmaker_orca">{t("export.targetSnapmaker")}</option>
                      )}
                      {(!isTexture || machine?.targetSlicers.includes("orcaslicer")) && (
                        <option value="orcaslicer">{t("export.targetOrca")}</option>
                      )}
                    </select>
                  </label>
                </div>
              </div>
            )}
          </fieldset>
        </div>

        {format === "3mf" && loadError !== null && (
          <p role="alert" className="texture-export-feedback export-dialog-error">
            {loadError ? t("export.presetLoadFailed", loadError) : t("export.noPresets")}
          </p>
        )}
        {(exportError || sourceReplaced) && (
          <p role="alert" className="texture-export-feedback export-dialog-error">
            {sourceReplaced ? t("export.modelChanged") : t("export.failure", exportError ?? "")}
          </p>
        )}
        {exporting && (
          <div style={styles.progressWrap}>
            <div style={{ ...styles.progressBar, width: `${Math.round(progress * 100)}%` }} />
            <span style={styles.progressText}>{Math.round(progress * 100)}%</span>
          </div>
        )}

        <div style={styles.footer}>
          <button className="cym-btn" style={styles.btn} onClick={onClose} disabled={exporting}>
            {t("export.cancel")}
          </button>
          <button
            className="cym-btn"
            style={{ ...styles.btn, ...styles.btnPrimary }}
            onClick={doExport}
            disabled={!canExport}
          >
            {t(exporting ? "export.exporting" : showMapping ? "export.textureConfirm" : "export.confirm")}
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed",
    inset: 0,
    background: "var(--overlay-bg, rgba(0,0,0,0.55))",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1000,
  },
  dialog: {
    width: 460,
    maxWidth: "calc(100vw - 56px)",
    maxHeight: "85vh",
    // Flex column + scrollable body (styles.body): header/progress/footer stay
    // pinned while only the settings list scrolls. The dialog itself no longer
    // scrolls as a whole — that buried the Export button below the fold.
    display: "flex",
    flexDirection: "column",
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 16,
    color: "var(--text-1, #eee)",
    fontFamily: 'inherit',
  },
  textureDialog: {
    width: 920,
  },
  body: {
    overflowY: "auto",
    minHeight: 0,
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    fontSize: 15,
    fontWeight: 600,
    marginBottom: 12,
  },
  themeButton: {
    marginTop: 0,
  },
  label: {
    display: "block",
    fontSize: 12,
    color: "var(--text-2, #aaa)",
    margin: "10px 0 4px",
  },
  select: {
    width: "100%",
    boxSizing: "border-box",
    padding: "6px 8px",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    fontSize: 13,
  },
  row: {
    display: "flex",
    gap: 6,
    flexWrap: "wrap" as const,
  },
  nozzleBtn: {
    padding: "4px 10px",
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 12,
  },
  nozzleActive: {
    border: "1px solid var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
  },
  fmtBtn: {
    flex: 1,
    padding: "6px 8px",
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 13,
  },
  fmtActive: {
    border: "1px solid var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
  },
  note: {
    fontSize: 12,
    lineHeight: 1.5,
    color: "var(--text-2, #aaa)",
    background: "var(--bg-elevated, #3a3a3a)",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    padding: "8px 10px",
    margin: "8px 0",
  },
  progressWrap: {
    position: "relative",
    height: 18,
    background: "var(--bg-elevated, #3a3a3a)",
    border: "1px solid var(--border, #555)",
    borderRadius: 6,
    overflow: "hidden",
    margin: "12px 0 4px",
  },
  progressBar: {
    height: "100%",
    background: "var(--accent, #4a9eff)",
    transition: "width 0.15s linear",
  },
  progressText: {
    position: "absolute",
    inset: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    fontSize: 11,
    color: "var(--text-1, #eee)",
  },
  slotRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
    marginBottom: 4,
  },
  bulkRow: {
    display: "flex",
    alignItems: "center",
    gap: 6,
    marginBottom: 8,
    flexWrap: "wrap" as const,
  },
  bulkBtn: {
    padding: "5px 9px",
    border: "1px solid var(--border, #555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #eee)",
    borderRadius: 6,
    cursor: "pointer",
    fontSize: 12,
    whiteSpace: "nowrap" as const,
  },
  bulkApply: {
    border: "1px solid var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
  },
  slotCheck: {
    width: 15,
    height: 15,
    accentColor: "var(--accent, #4a9eff)",
    cursor: "pointer",
    flexShrink: 0,
  },
  swatch: {
    width: 18,
    height: 18,
    borderRadius: 4,
    border: "1px solid rgba(255,255,255,0.3)",
    flexShrink: 0,
  },
  footer: {
    display: "flex",
    justifyContent: "flex-end",
    gap: 8,
    marginTop: 16,
  },
  btn: {
    padding: "6px 14px",
    border: "none",
    borderRadius: 6,
    background: "var(--bg-hover, #444)",
    color: "var(--text-1, #eee)",
    cursor: "pointer",
    fontSize: 13,
  },
  btnPrimary: {
    background: "var(--accent, #4a9eff)",
    color: "#fff",
  },
};
