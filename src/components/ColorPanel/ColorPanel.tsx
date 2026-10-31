import { useEffect, useState } from "react";
import { useAppStore } from "../../store/appStore";
import { AMS_PALETTE, type UsedColorEntry } from "../../types/mesh";
import { useT } from "../../i18n";
import { useTauriCommand } from "../../hooks/useTauriCommand";
import { AMS_PALETTE as AMS } from "../../types/mesh";
import { ReplaceColourSection } from "./ReplaceColourSection";
import { toHex } from "../../utils/colorFormat";
import { condenseUsedColours } from "../../utils/usedColours";

export function ColorPanel() {
  const t = useT();
  const currentColor = useAppStore((s) => s.currentColor);
  const setCurrentColor = useAppStore((s) => s.setCurrentColor);
  const setUsedColourHover = useAppStore((s) => s.setUsedColourHover);
  const { usedColors } = useTauriCommand();

  // Issue #23: the colours actually on the model, most faces first. The
  // refresh rides the same two signals the history panel uses —
  // meshData (import / labels rewrite) and historyVersion (every recorded
  // op, undo/redo, jump) — debounced 250 ms because a brush drag bumps the
  // version per pointermove and the backend pass is O(F).
  const modelFaceColors = useAppStore((s) => s.meshData?.faceColors);
  const historyVersion = useAppStore((s) => s.historyVersion);
  const [used, setUsed] = useState<UsedColorEntry[]>([]);
  // Issue #29 governance acceptance: paint provenance — the BRUSH colour a
  // stroke was painted with (not the blended per-face result). Selecting a
  // group offers a one-click "repaint the whole stroke" replacement.
  const { paintSourceGroups, replacePaintSource } = useTauriCommand();
  const historyVersionBump = useAppStore((s) => s.historyVersion);
  const [paintGroups, setPaintGroups] = useState<
    { source: [number, number, number, number]; faces: number }[]
  >([]);
  const [selectedSource, setSelectedSource] = useState<[number, number, number, number] | null>(null);
  const [repaintTo, setRepaintTo] = useState<[number, number, number, number] | null>(null);
  const [repainting, setRepainting] = useState(false);
  const setCurrentColorLocal = useAppStore((s) => s.setCurrentColor);

  useEffect(() => {
    let cancelled = false;
    if (!modelFaceColors) {
      setPaintGroups([]);
      setSelectedSource(null);
      return;
    }
    const timer = setTimeout(() => {
      void paintSourceGroups().then((res) => {
        if (!cancelled) setPaintGroups(res ?? []);
      });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modelFaceColors, historyVersionBump]);

  // Esc deselects the paint-source group (before the replace-collapse listener
  // gets a chance — this one is registered later, so it wins on the same key).
  useEffect(() => {
    if (selectedSource === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      setSelectedSource(null);
      setRepaintTo(null);
      e.stopImmediatePropagation();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedSource]);

  const applyRepaint = async () => {
    if (!selectedSource || !repaintTo) return;
    setRepainting(true);
    const n = await replacePaintSource(selectedSource, repaintTo);
    setRepainting(false);
    if (n > 0) {
      useAppStore.getState().markHistoryDirty();
      useAppStore.getState().setStatusMessage(t("color.replacedSource", n));
    }
    setSelectedSource(null);
    setRepaintTo(null);
  };
  // Issue #29 acceptance #6: the replace section starts collapsed and Esc
  // collapses it (dropping its internal picks with the unmount).
  const [replaceOpen, setReplaceOpen] = useState(false);
  useEffect(() => {
    if (!replaceOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA")) return;
      setReplaceOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [replaceOpen]);

  useEffect(() => {
    // Guards the async round-trip: a slow earlier fetch must never overwrite
    // the result of a newer one.
    let cancelled = false;
    if (!modelFaceColors) {
      setUsed([]);
      return;
    }
    const timer = setTimeout(() => {
      void usedColors().then((res) => {
        if (res && !cancelled) setUsed(condenseUsedColours(res));
      });
    }, 250);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
    // usedColors changes identity every render by design; the deps below are
    // the real triggers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modelFaceColors, historyVersion]);

  // The section is about colours the USER put on the model — a model with
  // only the never-painted default grey has nothing to show yet.
  const showUsed = used.length > 0 && !(used.length === 1 && used[0].isDefault);

  const handleColorChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const hex = e.target.value;
    const r = parseInt(hex.slice(1, 3), 16);
    const g = parseInt(hex.slice(3, 5), 16);
    const b = parseInt(hex.slice(5, 7), 16);
    setCurrentColor([r, g, b, 255]);
  };

  const hexColor = toHex(currentColor);

  // Compute perceived brightness for UI contrast (ITU-R BT.709 luma)
  const luma = (0.2126 * currentColor[0] + 0.7152 * currentColor[1] + 0.0722 * currentColor[2]) / 255;
  const isDark = luma < 0.25;

  return (
    <div style={styles.container}>
      <div style={styles.header}>{t("color.title")}</div>
      <div style={styles.currentRow}>
        <div
          style={{
            ...styles.preview,
            backgroundColor: hexColor,
            borderColor: isDark ? "var(--accent, #4a9eff)" : "var(--border, #555555)",
            borderWidth: isDark ? 2 : 1,
            boxShadow: isDark ? "0 0 0 1px var(--bg-panel, #2d2d2d) inset" : "none",
          }}
          title={`RGB(${currentColor[0]}, ${currentColor[1]}, ${currentColor[2]})`}
        />
        <span style={{
          fontSize: 11,
          fontFamily: "monospace",
          color: "var(--text-2, #aaaaaa)",
          minWidth: 62,
        }}>
          {hexColor.toUpperCase()}
        </span>
        <input
          type="color"
          value={hexColor}
          onChange={handleColorChange}
          style={styles.colorInput}
        />
      </div>

      <div style={styles.subheader}>{t("color.amsPalette")}</div>
      <div style={styles.palette}>
        {AMS_PALETTE.map((entry) => {
          const hex = `#${entry.color[0].toString(16).padStart(2, "0")}${entry.color[1].toString(16).padStart(2, "0")}${entry.color[2].toString(16).padStart(2, "0")}`;
          const isActive =
            currentColor[0] === entry.color[0] &&
            currentColor[1] === entry.color[1] &&
            currentColor[2] === entry.color[2];
          return (
            <button
              key={entry.name}
              title={entry.name}
              onClick={() =>
                setCurrentColor([entry.color[0], entry.color[1], entry.color[2], 255])
              }
              className="cym-btn"
              style={{
                ...styles.swatch,
                backgroundColor: hex,
                border: isActive ? "2px solid var(--accent, #4a9eff)" : "1px solid var(--border, #555555)",
              }}
            />
          );
        })}
      </div>

      {/* Issue #29 governance: paint provenance — each swatch is the BRUSH
          colour a stroke was painted with (icon-marked); selecting one offers
          a one-click repaint of the whole stroke (acceptance round 3, #3). */}
      {showUsed && paintGroups.length > 0 && (
        <>
          <div style={styles.subheader}>{t("color.paintColours")}</div>
          <div style={styles.palette}>
            {paintGroups.map((g) => {
              const hex = toHex(g.source);
              const key = `${g.source[0]},${g.source[1]},${g.source[2]},${g.source[3]}`;
              const isSelected =
                selectedSource !== null &&
                selectedSource[0] === g.source[0] &&
                selectedSource[1] === g.source[1] &&
                selectedSource[2] === g.source[2] &&
                selectedSource[3] === g.source[3];
              return (
                <button
                  key={key}
                  title={t("color.paintTooltip", hex.toUpperCase(), g.faces.toLocaleString())}
                  onClick={() => {
                    setSelectedSource(isSelected ? null : g.source);
                    setRepaintTo(null);
                  }}
                  className="cym-btn"
                  style={{
                    ...styles.swatch,
                    position: "relative",
                    backgroundColor: hex,
                    border: isSelected
                      ? "2px solid var(--accent, #4a9eff)"
                      : "1px solid var(--border, #555555)",
                  }}
                >
                  <span
                    style={{
                      position: "absolute",
                      top: -4,
                      right: -3,
                      fontSize: 9,
                      pointerEvents: "none",
                    }}
                  >
                    🖌
                  </span>
                </button>
              );
            })}
          </div>
          {selectedSource && (
            <div style={styles.replaceBox}>
              <div style={styles.replaceHint}>{t("color.repaintHint")}</div>
              <div style={styles.palette}>
                {AMS.map((entry) => {
                  const hex = toHex(entry.color);
                  const isTo =
                    repaintTo !== null &&
                    repaintTo[0] === entry.color[0] &&
                    repaintTo[1] === entry.color[1] &&
                    repaintTo[2] === entry.color[2];
                  return (
                    <button
                      key={entry.name}
                      title={entry.name}
                      onClick={() =>
                        setRepaintTo([entry.color[0], entry.color[1], entry.color[2], 255])
                      }
                      className="cym-btn"
                      style={{
                        ...styles.swatch,
                        backgroundColor: hex,
                        border: isTo
                          ? "2px solid var(--accent, #4a9eff)"
                          : "1px solid var(--border, #555555)",
                      }}
                    />
                  );
                })}
              </div>
              <button
                className="cym-btn"
                style={styles.replaceApply}
                disabled={!repaintTo || repainting}
                onClick={() => void applyRepaint()}
              >
                {repainting ? "…" : t("color.repaintApply")}
              </button>
              <div style={styles.replaceNote}>{t("color.repaintNote")}</div>
            </div>
          )}
        </>
      )}

      {/* Issue #23: the colours actually on the model, click = pick up. */}
      {showUsed && (
        <>
          <div style={styles.subheader}>{t("color.usedColors")}</div>
          <div style={styles.palette}>
            {used.map((u) => {
              const hex = toHex(u.color);
              const isActive =
                currentColor[0] === u.color[0] &&
                currentColor[1] === u.color[1] &&
                currentColor[2] === u.color[2];
              const tooltip = u.isDefault
                ? t("color.usedDefaultTooltip", hex.toUpperCase(), u.faces.toLocaleString(), u.regions)
                : t("color.usedTooltip", hex.toUpperCase(), u.faces.toLocaleString(), u.regions);
              return (
                <button
                  key={hex}
                  title={tooltip}
                  onMouseEnter={() => setUsedColourHover([u.color[0], u.color[1], u.color[2]])}
                  onMouseLeave={() => setUsedColourHover(null)}
                  onClick={() => setCurrentColor([u.color[0], u.color[1], u.color[2], 255])}
                  className="cym-btn"
                  style={{
                    ...styles.swatch,
                    backgroundColor: hex,
                    border: isActive
                      ? "2px solid var(--accent, #4a9eff)"
                      : u.isDefault
                        ? "1px dashed var(--text-2, #aaaaaa)"
                        : "1px solid var(--border, #555555)",
                  }}
                />
              );
            })}
          </div>
        </>
      )}

      {/* Issue #29: colour find & replace — only meaningful once a model is
          loaded (the used list is empty before that, but replace does not
          require any USED colour: the model may still be all-default).
          Acceptance round 2 feedback #6: the section is COLLAPSIBLE and
          starts collapsed — a picker the user is not using must not be a
          permanent fixture. Esc or the × collapses it (state inside is
          dropped, so picks never outlive the collapse). */}
      {modelFaceColors && (
        <div>
          <button
            className="cym-btn"
            style={styles.replaceToggle}
            onClick={() => setReplaceOpen((v) => !v)}
            title={t("color.replaceToggle")}
          >
            {replaceOpen ? "▾" : "▸"} {t("replace.title")}
            {replaceOpen && (
              <span
                role="button"
                aria-label={t("color.replaceClose")}
                title={t("color.replaceClose")}
                onClick={(e) => {
                  e.stopPropagation();
                  setReplaceOpen(false);
                }}
                style={{ marginLeft: 6, opacity: 0.7 }}
              >
                ✕
              </span>
            )}
          </button>
          {replaceOpen && modelFaceColors && (
            <ReplaceColourSection used={used} />
          )}
        </div>
      )}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    padding: 10,
    background: "var(--bg-panel, #2d2d2d)",
    borderRadius: 8,
  },
  header: {
    color: "var(--text-1, #dddddd)",
    fontSize: 14,
    fontWeight: 600,
    marginBottom: 8,
  },
  subheader: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    marginTop: 8,
    marginBottom: 4,
  },
  currentRow: {
    display: "flex",
    alignItems: "center",
    gap: 8,
  },
  preview: {
    width: 32,
    height: 32,
    borderRadius: 6,
    border: "1px solid var(--border, #555555)",
  },
  colorInput: {
    width: 40,
    height: 32,
    border: "none",
    padding: 0,
    cursor: "pointer",
    background: "transparent",
  },
  replaceBox: {
    marginTop: 6,
    padding: 6,
    border: "1px solid var(--border, #555555)",
    borderRadius: 6,
  },
  replaceHint: {
    fontSize: 11,
    color: "var(--text-2, #aaaaaa)",
    marginBottom: 4,
    lineHeight: 1.5,
  },
  replaceApply: {
    width: "100%",
    marginTop: 6,
    padding: "5px 0",
  },
  replaceNote: {
    fontSize: 10,
    color: "var(--text-3, #777777)",
    marginTop: 4,
    lineHeight: 1.5,
  },
  replaceToggle: {
    width: "100%",
    textAlign: "left",
    padding: "4px 6px",
    marginBottom: 4,
    background: "transparent",
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    border: "1px solid var(--border, #555555)",
    borderRadius: 6,
    cursor: "pointer",
  },
  palette: {
    display: "grid",
    gridTemplateColumns: "repeat(5, 1fr)",
    gap: 4,
  },
  swatch: {
    width: 28,
    height: 28,
    borderRadius: 4,
    cursor: "pointer",
  },
};
