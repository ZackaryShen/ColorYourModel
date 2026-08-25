import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { save } from "@tauri-apps/plugin-dialog";
import { useT } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import {
  ExportSelection,
  FilamentSummary,
  MachineSummary,
} from "../../types/export";
import { log } from "../../utils/logger";

/**
 * Modal "pick the machine / nozzle / process / filament" dialog shown before
 * the 3MF save dialog. Lets the user tell the slicer what hardware the file
 * was painted for, instead of re-picking it by hand after every export.
 *
 * Data comes from the backend preset library (`list_export_presets`) and the
 * actual quantised palette of the loaded mesh (`export_palette_preview`), so
 * the dialog never drifts from what the writer will produce.
 */
export function ExportDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const { export3mf, exportObj } = useTauriCommand();
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const lastSelection = useAppStore((s) => s.lastExportSelection);
  const setLastExportSelection = useAppStore((s) => s.setLastExportSelection);

  // 3MF carries colours through a slicer profile; OBJ writes RGB directly and
  // needs none of the machine / process / filament pickers.
  const [format, setFormat] = useState<"3mf" | "obj">("3mf");
  const [progress, setProgress] = useState(0);

  const [machines, setMachines] = useState<MachineSummary[]>([]);
  const [palette, setPalette] = useState<string[]>([]);
  const [machineId, setMachineId] = useState("");
  const [nozzle, setNozzle] = useState("");
  const [processName, setProcessName] = useState("");
  const [filamentNames, setFilamentNames] = useState<string[]>([]);
  const [targetSlicer, setTargetSlicer] = useState("snapmaker_orca");
  const [exporting, setExporting] = useState(false);

  // Live progress from the backend's `export-progress` events. Mounted once so
  // the listener survives the whole export; the command runs off the webview
  // thread, so these arrive in real time instead of being batched at the end.
  useEffect(() => {
    const unlisten = listen<{ progress: number; stage: string }>(
      "export-progress",
      (e) => {
        setProgress(e.payload.progress);
      }
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [ms, pal] = await Promise.all([
          invoke<MachineSummary[]>("list_export_presets"),
          invoke<string[]>("export_palette_preview"),
        ]);
        if (cancelled) return;
        setMachines(ms);
        setPalette(pal.length ? pal : ["#8A8A8A"]);
        if (ms.length > 0) {
          // Prefer the persisted selection when it still matches a known
          // machine; otherwise fall back to the first machine's defaults.
          const persisted = lastSelection;
          const known = persisted
            ? ms.find((m) => m.id === persisted.machineId)
            : undefined;
          setMachineId(known ? known.id : ms[0].id);
          setTargetSlicer(
            known && persisted && persisted.targetSlicer
              ? persisted.targetSlicer
              : ms[0].targetSlicers.includes("snapmaker_orca")
                ? "snapmaker_orca"
                : ms[0].targetSlicers[0] ?? "orcaslicer"
          );
        }
      } catch (e) {
        log.error("ExportDialog", "failed to load presets", { error: String(e) });
        setStatusMessage(`导出配置加载失败：${e}`);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [setStatusMessage]); // eslint-disable-line react-hooks/exhaustive-deps

  const machine = useMemo(
    () => machines.find((m) => m.id === machineId) ?? null,
    [machines, machineId]
  );

  const nozzles = machine?.nozzles ?? [];

  // Reset nozzle / process / filament whenever the machine changes. When a
  // persisted selection matches, restore its nozzle / process / filaments.
  useEffect(() => {
    const persisted = lastSelection;
    const matches = persisted && persisted.machineId === machineId;
    setNozzle(matches ? persisted!.nozzleDiameter : (nozzles[0] ?? ""));
    setProcessName(matches ? persisted!.processName : "");
    if (matches) {
      const names = persisted!.filamentNames;
      if (names.length > 0) setFilamentNames(names.slice(0, palette.length || 1));
    }
  }, [machineId]); // eslint-disable-line react-hooks/exhaustive-deps

  const processes = useMemo(
    () => (machine?.processes ?? []).filter((p) => p.nozzles.includes(nozzle)),
    [machine, nozzle]
  );

  const filaments = useMemo(
    () =>
      (machine?.filaments ?? []).filter((f) => f.nozzles.includes(nozzle)),
    [machine, nozzle]
  );

  // Reset process when nozzle changes (a process may not exist for the new nozzle).
  useEffect(() => {
    const first = processes[0];
    setProcessName(first ? first.name : "");
  }, [nozzle, processes.length]); // eslint-disable-line react-hooks/exhaustive-deps

  // Keep one filament picker per palette slot; shorter arrays repeat the last
  // entry on the backend, so a single choice covers every slot.
  useEffect(() => {
    setFilamentNames((prev) => {
      const next = [...prev];
      while (next.length < palette.length) {
        const last = next[next.length - 1] ?? filaments[0]?.name ?? "";
        next.push(last);
      }
      return next.slice(0, palette.length);
    });
  }, [palette.length, filaments]); // eslint-disable-line react-hooks/exhaustive-deps

  const doExport = async () => {
    if (format === "3mf" && (!machine || !processName)) return;
    const selection: ExportSelection = {
      machineId,
      nozzleDiameter: nozzle,
      processName,
      filamentNames,
      targetSlicer,
    };
    setExporting(true);
    setProgress(0);
    try {
      const ext = format === "obj" ? "obj" : "3mf";
      const path = await save({
        filters: [{ name: format === "obj" ? "OBJ" : "3MF", extensions: [ext] }],
        defaultPath: `model.${ext}`,
      });
      if (!path) return; // user cancelled the save dialog

      // Tauri's save dialog does NOT auto-append the extension, so a user who
      // types "mymodel" would otherwise get a suffix-less file — invisible
      // when filtering for *.3mf / *.obj. Force it.
      const outPath = path.toLowerCase().endsWith(`.${ext}`)
        ? path
        : `${path}.${ext}`;

      if (format === "obj") {
        await exportObj(outPath);
      } else {
        await export3mf(outPath, selection);
        setLastExportSelection({
          machineId,
          nozzleDiameter: nozzle,
          processName,
          filamentNames,
          targetSlicer,
        });
      }
      onClose();
    } catch (e) {
      log.error("ExportDialog", "export failed", { error: String(e) });
      setStatusMessage(`导出失败：${e}`);
    } finally {
      setExporting(false);
    }
  };

  return (
    <div style={styles.overlay} onClick={onClose}>
      <div
        style={styles.dialog}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
      >
        <div style={styles.header}>{t("export.title")}</div>

        {/* Format toggle — 3MF needs a slicer profile; OBJ writes colour
            directly and hides the machine / process / filament pickers. */}
        <label style={styles.label}>{t("export.format")}</label>
        <div style={styles.row}>
          <button
            className="cym-btn"
            style={{
              ...styles.fmtBtn,
              ...(format === "3mf" ? styles.fmtActive : {}),
            }}
            onClick={() => setFormat("3mf")}
          >
            {t("export.format3mf")}
          </button>
          <button
            className="cym-btn"
            style={{
              ...styles.fmtBtn,
              ...(format === "obj" ? styles.fmtActive : {}),
            }}
            onClick={() => setFormat("obj")}
          >
            {t("export.formatObj")}
          </button>
        </div>

        {format === "3mf" ? (
          <>
            {/* Machine */}
            <label style={styles.label}>{t("export.machine")}</label>
            <select
              style={styles.select}
              value={machineId}
              onChange={(e) => setMachineId(e.target.value)}
            >
              {machines.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.vendor} — {m.name}
                </option>
              ))}
            </select>

            {/* Nozzle */}
            {nozzles.length > 1 && (
              <>
                <label style={styles.label}>{t("export.nozzle")}</label>
                <div style={styles.row}>
                  {nozzles.map((n) => (
                    <button
                      key={n}
                      className="cym-btn"
                      style={{
                        ...styles.nozzleBtn,
                        ...(n === nozzle ? styles.nozzleActive : {}),
                      }}
                      onClick={() => setNozzle(n)}
                    >
                      {n}mm
                    </button>
                  ))}
                </div>
              </>
            )}

            {/* Process */}
            <label style={styles.label}>{t("export.process")}</label>
            <select
              style={styles.select}
              value={processName}
              onChange={(e) => setProcessName(e.target.value)}
              disabled={processes.length === 0}
            >
              {processes.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                  {p.layerHeight ? ` (${p.layerHeight}mm)` : ""}
                </option>
              ))}
            </select>

            {/* Filament per slot */}
            <label style={styles.label}>
              {t("export.filament")} ({palette.length}{t("export.slots")})
            </label>
            {palette.map((colour, i) => {
              const slotFilament = filamentNames[i] ?? filaments[0]?.name ?? "";
              return (
                <div key={i} style={styles.slotRow}>
                  <span
                    style={{ ...styles.swatch, background: colour }}
                    title={colour}
                  />
                  <select
                    style={styles.select}
                    value={slotFilament}
                    onChange={(e) => {
                      setFilamentNames((prev) => {
                        const next = [...prev];
                        next[i] = e.target.value;
                        return next;
                      });
                    }}
                  >
                    {filaments.map((f) => (
                      <option key={f.name} value={f.name}>
                        {f.name}
                      </option>
                    ))}
                  </select>
                </div>
              );
            })}

            {/* Target slicer */}
            <label style={styles.label}>{t("export.target")}</label>
            <select
              style={styles.select}
              value={targetSlicer}
              onChange={(e) => setTargetSlicer(e.target.value)}
            >
              <option value="snapmaker_orca">{t("export.targetSnapmaker")}</option>
              <option value="orcaslicer">{t("export.targetOrca")}</option>
            </select>
          </>
        ) : (
          <div style={styles.note}>{t("export.objNote")}</div>
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
            disabled={exporting || (format === "3mf" && (!machine || !processName))}
          >
            {exporting ? t("export.exporting") : t("export.confirm")}
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
    background: "rgba(0,0,0,0.55)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 1000,
  },
  dialog: {
    width: 460,
    maxHeight: "85vh",
    overflowY: "auto",
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border, #555)",
    borderRadius: 10,
    padding: 16,
    color: "var(--text-1, #eee)",
    fontFamily: 'inherit',
  },
  header: {
    fontSize: 15,
    fontWeight: 600,
    marginBottom: 12,
  },
  label: {
    display: "block",
    fontSize: 12,
    color: "var(--text-2, #aaa)",
    margin: "10px 0 4px",
  },
  select: {
    width: "100%",
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
    borderColor: "var(--accent, #4a9eff)",
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
    borderColor: "var(--accent, #4a9eff)",
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
