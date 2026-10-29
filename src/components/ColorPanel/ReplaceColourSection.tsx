import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../../store/appStore";
import { AMS_PALETTE, type UsedColorEntry } from "../../types/mesh";
import { useT } from "../../i18n";
import { planReplaceColour, replaceGuard } from "../../utils/replaceColour";
import { toHex } from "../../utils/colorFormat";

type RGBA = [number, number, number, number];

/**
 * Colour find & replace (issue #29): pick a source from the colours actually
 * on the model, a target from anywhere, preview the face/region count live
 * over the frontend faceColors mirror (utils/replaceColour), then apply —
 * one backend command, one undo entry. The swatch grids re-use the used
 * colours (#23) and the AMS palette; picking never touches the panel's own
 * click-to-set-current behaviour.
 */
export function ReplaceColourSection({ used }: { used: UsedColorEntry[] }) {
  const t = useT();
  const faceColors = useAppStore((s) => s.meshData?.faceColors);
  const segmentLabels = useAppStore((s) => s.meshData?.segmentLabels);
  const viewSelectedRegions = useAppStore((s) => s.viewSelectedRegions);
  const currentColor = useAppStore((s) => s.currentColor);
  const setPendingPaintResult = useAppStore((s) => s.setPendingPaintResult);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);

  const [from, setFrom] = useState<RGBA | null>(null);
  const [to, setTo] = useState<RGBA | null>(null);
  const [scope, setScope] = useState<"model" | "regions">("model");
  const [picking, setPicking] = useState<"from" | "to" | null>(null);
  const [applying, setApplying] = useState(false);

  // Esc closes the picker — same contract as the app's other popovers.
  useEffect(() => {
    if (!picking) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setPicking(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [picking]);

  // A new model (or a full-payload undo) invalidates the picks — the source
  // colour may not even exist on the new face buffer. Incremental paints
  // mutate the array in place, so brushing never clears the section.
  useEffect(() => {
    setFrom(null);
    setTo(null);
    setPicking(null);
    setScope("model");
  }, [faceColors]);

  // The EFFECTIVE scope: choosing 仅选中分区 while the viewport selection is
  // empty must not render a checked radio over a whole-model apply (the
  // viewport's own Esc clears viewSelectedRegions — seen live with the
  // picker's Esc on the same keypress).
  const regionsScopeActive = scope === "regions" && viewSelectedRegions.length > 0;
  const scopeArg = regionsScopeActive
    ? { kind: "regions" as const, labels: viewSelectedRegions }
    : { kind: "model" as const };

  const plan =
    from && faceColors && segmentLabels
      ? planReplaceColour(faceColors, segmentLabels, from, scopeArg)
      : null;
  const guard = from && to ? replaceGuard(from, to) : null;
  const canApply = !!from && !!to && guard === null && !!plan && plan.faces > 0 && !applying;

  const apply = async () => {
    if (!from || !to) return;
    setApplying(true);
    const hexFrom = toHex(from).toUpperCase();
    const hexTo = toHex(to).toUpperCase();
    try {
      const result = await invoke<{
        updatedFaces: number[];
        updatedColors: number[][];
        faces: number;
        regions: number;
      }>("replace_face_color", {
        from,
        to,
        labels: scopeArg.kind === "regions" ? scopeArg.labels : null,
      });
      // The backend mirrored the guards (defence in depth): a 0-face result
      // means mirror drift or a no-op — nothing to hand to the GPU bridge,
      // and no fake "已替换 0 面" success.
      if (result.faces === 0) {
        setStatusMessage(t("replace.none", hexFrom));
        return;
      }
      // GPU + faceColors mirror + historyVersion bump all ride the pending
      // bridge — the used-colours histogram refreshes itself from that.
      setPendingPaintResult({ faces: result.updatedFaces, colors: result.updatedColors });
      setStatusMessage(t("replace.done", result.faces, result.regions, hexFrom, hexTo));
      setFrom(null);
      setTo(null);
      setPicking(null);
    } catch (e) {
      setStatusMessage(t("replace.failed", String(e)));
    } finally {
      setApplying(false);
    }
  };

  const pickerGrid = (target: "from" | "to") => {
    const entries: { colour: RGBA; dashed?: boolean }[] = used.map((u) => ({
      colour: u.color,
      dashed: u.isDefault,
    }));
    if (target === "to") {
      // Skip AMS entries the used list already shows — the same RGBA would
      // render as two identical swatches (e.g. AMS Brown = a painted brown).
      const seen = new Set(entries.map((e) => e.colour.join(",")));
      for (const p of AMS_PALETTE) {
        const colour: RGBA = [p.color[0], p.color[1], p.color[2], 255];
        if (!seen.has(colour.join(","))) {
          seen.add(colour.join(","));
          entries.push({ colour });
        }
      }
    }
    return (
      <div style={styles.picker}>
        <div style={styles.pickerHint}>
          {target === "from" ? t("replace.pickSource") : t("replace.pickTarget")}
        </div>
        <div style={styles.palette}>
          {entries.map((e, i) => (
            <button
              key={`${target}-${i}`}
              className="cym-btn"
              title={toHex(e.colour).toUpperCase()}
              onClick={() => {
                if (target === "from") setFrom(e.colour);
                else setTo(e.colour);
                setPicking(null);
              }}
              style={{
                ...styles.swatch,
                backgroundColor: toHex(e.colour),
                border: e.dashed ? "1px dashed var(--text-2, #aaaaaa)" : "1px solid var(--border, #555555)",
              }}
            />
          ))}
          <button
            className="cym-btn"
            style={styles.useCurrent}
            onClick={() => {
              if (target === "from") setFrom([...currentColor] as RGBA);
              else setTo([...currentColor] as RGBA);
              setPicking(null);
            }}
          >
            {t("replace.useCurrent")}
          </button>
        </div>
      </div>
    );
  };

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("replace.title")}</div>
      <div style={styles.row}>
        <span style={styles.sideLabel}>{t("replace.from")}</span>
        <button
          className="cym-btn"
          aria-label={t("replace.from")}
          title={from ? toHex(from).toUpperCase() : t("replace.pickSource")}
          onClick={() => setPicking(picking === "from" ? null : "from")}
          style={{
            ...styles.bigSwatch,
            backgroundColor: from ? toHex(from) : "transparent",
            border: picking === "from" ? "2px solid var(--accent, #4a9eff)" : from ? "1px solid var(--border, #555555)" : "1px dashed var(--text-2, #aaaaaa)",
          }}
        />
        <span style={styles.arrow}>→</span>
        <span style={styles.sideLabel}>{t("replace.to")}</span>
        <button
          className="cym-btn"
          aria-label={t("replace.to")}
          title={to ? toHex(to).toUpperCase() : t("replace.pickTarget")}
          onClick={() => setPicking(picking === "to" ? null : "to")}
          style={{
            ...styles.bigSwatch,
            backgroundColor: to ? toHex(to) : "transparent",
            border: picking === "to" ? "2px solid var(--accent, #4a9eff)" : to ? "1px solid var(--border, #555555)" : "1px dashed var(--text-2, #aaaaaa)",
          }}
        />
      </div>

      {picking && pickerGrid(picking)}

      <div style={styles.scopeRow}>
        <label style={styles.scopeLabel}>
          <input
            type="radio"
            name="replace-scope"
            checked={!regionsScopeActive}
            onChange={() => setScope("model")}
          />
          {t("replace.scopeModel")}
        </label>
        <label
          style={{ ...styles.scopeLabel, opacity: viewSelectedRegions.length > 0 ? 1 : 0.5 }}
          title={viewSelectedRegions.length > 0 ? undefined : t("replace.scopeSelectedTitle")}
        >
          <input
            type="radio"
            name="replace-scope"
            checked={regionsScopeActive}
            disabled={viewSelectedRegions.length === 0}
            title={viewSelectedRegions.length > 0 ? undefined : t("replace.scopeSelectedTitle")}
            onChange={() => setScope("regions")}
          />
          {t("replace.scopeSelected")}
          {regionsScopeActive ? ` (${viewSelectedRegions.length})` : ""}
        </label>
      </div>

      {guard ? (
        <div style={styles.guard}>
          {guard === "same" ? t("replace.guardSame") : t("replace.guardDefault")}
        </div>
      ) : plan && from && to ? (
        plan.faces > 0 ? (
          <div style={styles.preview}>
            {t("replace.preview", plan.faces, plan.regions, toHex(from).toUpperCase(), toHex(to).toUpperCase())}
          </div>
        ) : (
          <div style={styles.guard}>{t("replace.none", toHex(from).toUpperCase())}</div>
        )
      ) : null}

      <button className="cym-btn" style={styles.apply} onClick={() => void apply()} disabled={!canApply}>
        {applying ? "…" : t("replace.apply")}
      </button>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    marginTop: 10,
    padding: 8,
    border: "1px solid var(--border, #555555)",
    borderRadius: 8,
  },
  header: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    marginBottom: 6,
  },
  row: {
    display: "flex",
    alignItems: "center",
    gap: 6,
  },
  sideLabel: {
    fontSize: 11,
    color: "var(--text-2, #aaaaaa)",
  },
  arrow: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 13,
    margin: "0 2px",
  },
  bigSwatch: {
    width: 28,
    height: 28,
    borderRadius: 4,
    cursor: "pointer",
    padding: 0,
  },
  picker: {
    marginTop: 6,
  },
  pickerHint: {
    fontSize: 11,
    color: "var(--text-2, #aaaaaa)",
    marginBottom: 4,
  },
  palette: {
    display: "flex",
    flexWrap: "wrap",
    gap: 4,
    alignItems: "center",
  },
  swatch: {
    width: 22,
    height: 22,
    borderRadius: 4,
    cursor: "pointer",
    padding: 0,
  },
  useCurrent: {
    fontSize: 11,
    padding: "2px 6px",
    cursor: "pointer",
    border: "1px solid var(--border, #555555)",
    borderRadius: 4,
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #dddddd)",
  },
  scopeRow: {
    display: "flex",
    gap: 12,
    marginTop: 8,
    alignItems: "center",
  },
  scopeLabel: {
    fontSize: 11,
    color: "var(--text-1, #dddddd)",
    display: "flex",
    alignItems: "center",
    gap: 4,
    cursor: "pointer",
  },
  preview: {
    marginTop: 6,
    fontSize: 11,
    color: "var(--text-1, #dddddd)",
    lineHeight: 1.5,
  },
  guard: {
    marginTop: 6,
    fontSize: 11,
    color: "var(--danger, #e5484d)",
    lineHeight: 1.5,
  },
  apply: {
    marginTop: 8,
    width: "100%",
    padding: "5px 0",
    fontSize: 12,
    fontWeight: 600,
    cursor: "pointer",
    border: "1px solid var(--accent-border, #2f6fdb)",
    background: "var(--accent, #3b82f6)",
    color: "#fff",
    borderRadius: 6,
  },
};
