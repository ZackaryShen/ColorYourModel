import { useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../../store/appStore";
import { useT } from "../../i18n";
import { log } from "../../utils/logger";

/**
 * Image-projection painting (0.2.0-P2, req #9): pick a picture, project it
 * orthographically along one of the six STL-local axes, and bake it into the
 * per-face paint. Lives OUTSIDE the R3F tree — the PaintResult is parked in
 * the store's pendingPaintResult for MeshDisplay to apply on the GPU.
 */

const AXES: { id: string; labelKey: string }[] = [
  { id: "+z", labelKey: "projection.top" },
  { id: "-z", labelKey: "projection.bottom" },
  { id: "+y", labelKey: "projection.back" },
  { id: "-y", labelKey: "projection.front" },
  { id: "+x", labelKey: "projection.right" },
  { id: "-x", labelKey: "projection.left" },
];

export function ProjectionDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [imagePath, setImagePath] = useState<string | null>(null);
  const [axis, setAxis] = useState<string>("+z");
  const [busy, setBusy] = useState(false);
  const setStatusMessage = useAppStore((s) => s.setStatusMessage);
  const setPendingPaintResult = useAppStore((s) => s.setPendingPaintResult);

  const pickImage = async () => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Image", extensions: ["png", "jpg", "jpeg", "webp", "bmp"] }],
    });
    if (selected) setImagePath(selected);
  };

  const project = async () => {
    if (!imagePath || busy) return;
    setBusy(true);
    try {
      const result = await invoke<{ updatedFaces: number[]; updatedColors: number[] }>(
        "project_image_paint",
        { path: imagePath, axis, strokeId: null },
      );
      setPendingPaintResult({
        faces: result.updatedFaces,
        colors: result.updatedColors,
      });
      setStatusMessage(t("status.projected", result.updatedFaces.length));
      onClose();
    } catch (e) {
      log.error("ProjectionDialog", "projection failed", { error: String(e) });
      setStatusMessage(`${t("projection.failed")}: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div style={styles.overlay} onClick={onClose}>
      <div style={styles.dialog} onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
        <div style={styles.title}>{t("projection.title")}</div>
        <div style={styles.hint}>{t("projection.hint")}</div>

        <button className="cym-btn" style={styles.pickBtn} onClick={() => void pickImage()}>
          {imagePath ? `🖼 ${imagePath.split(/[\\/]/).pop()}` : t("projection.pickImage")}
        </button>

        <div style={styles.axisGrid}>
          {AXES.map((a) => (
            <button
              key={a.id}
              className="cym-btn"
              style={{ ...styles.axisBtn, ...(axis === a.id ? styles.axisActive : {}) }}
              onClick={() => setAxis(a.id)}
            >
              {t(a.labelKey)}
            </button>
          ))}
        </div>
        <div style={styles.hintSmall}>{t("projection.axisHint")}</div>

        <div style={styles.btnRow}>
          <button
            className="cym-btn"
            style={styles.primaryBtn}
            disabled={!imagePath || busy}
            onClick={() => void project()}
          >
            {busy ? "…" : t("projection.run")}
          </button>
          <button className="cym-btn" style={styles.btn} onClick={onClose}>
            {t("about.close")}
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
    background: "rgba(0,0,0,0.5)",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    zIndex: 100,
  },
  dialog: {
    background: "var(--bg-panel, #2d2d2d)",
    border: "1px solid var(--border-strong, #333333)",
    borderRadius: 10,
    padding: 18,
    width: 340,
  },
  title: {
    color: "var(--text-1, #dddddd)",
    fontSize: 15,
    fontWeight: 600,
    marginBottom: 6,
  },
  hint: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 12,
    marginBottom: 10,
  },
  hintSmall: {
    color: "var(--text-2, #aaaaaa)",
    fontSize: 11,
    margin: "4px 0 10px",
  },
  pickBtn: {
    width: "100%",
    padding: "8px 10px",
    marginBottom: 10,
    textAlign: "left" as const,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap" as const,
  },
  axisGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(3, 1fr)",
    gap: 4,
  },
  axisBtn: {
    padding: "6px 4px",
    borderRadius: 4,
    border: "1px solid var(--border, #555555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #dddddd)",
    fontSize: 12,
    cursor: "pointer",
  },
  axisActive: {
    borderColor: "var(--accent, #4a9eff)",
    background: "var(--bg-active, #3a5a7a)",
  },
  btnRow: {
    display: "flex",
    gap: 8,
    justifyContent: "flex-end",
    marginTop: 12,
  },
  primaryBtn: {
    padding: "6px 14px",
    borderRadius: 6,
    border: "none",
    background: "var(--accent, #4a9eff)",
    color: "#ffffff",
    fontSize: 13,
    cursor: "pointer",
  },
  btn: {
    padding: "6px 14px",
    borderRadius: 6,
    border: "1px solid var(--border, #555555)",
    background: "var(--bg-elevated, #3a3a3a)",
    color: "var(--text-1, #dddddd)",
    fontSize: 13,
    cursor: "pointer",
  },
};
