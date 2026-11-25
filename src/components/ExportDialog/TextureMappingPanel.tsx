import { useMemo, useState } from "react";
import { useT } from "../../i18n";
import type { MeshData } from "../../types/mesh";
import type { Rgb, TextureMappingPreview, TextureMappingSuggestions } from "../../types/export";
import { TexturePreviewCanvas } from "../TexturePreview/TexturePreviewCanvas";
import "./textureMapping.css";

export function rgbHex(rgb: Rgb) {
  return "#" + rgb.map((component) => component.toString(16).padStart(2, "0")).join("").toUpperCase();
}

export function fromHex(hex: string): Rgb {
  return [1, 3, 5].map((index) => Number.parseInt(hex.slice(index, index + 2), 16)) as Rgb;
}

interface MappingState {
  assigned: ReadonlyMap<string, number>;
  slots: TextureMappingSuggestions["slots"];
}

interface ViewportProps extends MappingState {
  mesh: MeshData;
  preview: TextureMappingPreview;
}

export function TextureMappingViewport({ mesh, preview, assigned, slots }: ViewportProps) {
  const t = useT();
  const [resetKey, setResetKey] = useState(0);
  const [failed, setFailed] = useState(false);

  const mappedMesh = useMemo(() => {
    if (assigned.size === 0 && !preview.grouping) return mesh;
    const faceColors = [...mesh.faceColors];
    const displayColors = new Map(slots.map((slot) => [slot.slot, slot.displayColor]));
    if (preview.grouping) {
      const displays = preview.rows.map(({ rgb }) => displayColors.get(assigned.get(rgbHex(rgb)) ?? 0) ?? rgb);
      preview.grouping.faceRows.forEach((row, face) => {
        const [r, g, b] = displays[row];
        const i = face * 4;
        faceColors[i] = r;
        faceColors[i + 1] = g;
        faceColors[i + 2] = b;
        faceColors[i + 3] = 255;
      });
      return { ...mesh, faceColors };
    }
    // Resolve each RGB once, then use a numeric key in the per-face loop.
    const displayByRgb = new Map<number, Rgb>();
    for (const [color, slot] of assigned) {
      const display = displayColors.get(slot);
      if (display) displayByRgb.set(Number.parseInt(color.slice(1), 16), display);
    }
    for (let i = 0; i < faceColors.length; i += 4) {
      const key = (faceColors[i] << 16) | (faceColors[i + 1] << 8) | faceColors[i + 2];
      const display = displayByRgb.get(key);
      if (display) {
        faceColors[i] = display[0];
        faceColors[i + 1] = display[1];
        faceColors[i + 2] = display[2];
        faceColors[i + 3] = 255;
      }
    }
    return { ...mesh, faceColors };
  }, [mesh, preview, assigned, slots]);

  return (
    <section className="texture-export-viewport">
      <div className="texture-export-canvas">
        {failed ? (
          <p role="status">{t("export.texturePreviewError")}</p>
        ) : (
          <>
            <TexturePreviewCanvas
              converted={mappedMesh}
              label={t("export.texturePreview")}
              resetKey={resetKey}
              onError={() => setFailed(true)}
            />
            <span className="texture-export-viewport-caption">{t("export.texturePreview")}</span>
          </>
        )}
      </div>
      <button className="cym-btn texture-export-action" onClick={() => setResetKey(resetKey + 1)} disabled={failed}>
        {t("export.textureResetView")}
      </button>
    </section>
  );
}

interface TableProps extends MappingState {
  preview: TextureMappingPreview;
  onAssign: (rgb: Rgb, slot: number) => void;
  onSuggest: () => void;
  canSuggest: boolean;
}

export function TextureMappingTable({ preview, slots, assigned, onAssign, onSuggest, canSuggest }: TableProps) {
  const t = useT();
  const { rows, grouping } = preview;
  const numAssigned = rows.filter(({ rgb }) => assigned.has(rgbHex(rgb))).length;
  return (
    <section className="texture-export-mapping">
      <h3>{t("export.textureMapping")}</h3>
      {grouping && <p>{t("export.textureGrouped", grouping.sourceColorCount, rows.length)}</p>}
      <p>{rows.length > slots.length
        ? t("export.textureMerge", rows.length, slots.length)
        : t("export.textureSuggestionHint")}</p>
      <div className="texture-export-table-wrap">
        <table aria-label={t("export.textureMapping")}>
          <thead>
            <tr><th scope="col">{t("export.textureModelColour")}</th><th scope="col">{t("export.filament")}</th></tr>
          </thead>
          <tbody>
            {rows.map(({ rgb, faceCount }) => {
              const hex = rgbHex(rgb);
              const value = assigned.get(hex) ?? 0;
              const slot = slots.find(({ slot }) => slot === value);
              return (
                <tr key={hex}>
                  <th scope="row">
                    <span className="texture-export-swatch" style={{ backgroundColor: hex }} />
                    {hex}
                    <span className="texture-export-face-count" title={t("export.textureFaceCount", faceCount)}>
                      {faceCount.toLocaleString()}
                    </span>
                  </th>
                  <td>
                    <div className="texture-export-slot">
                      {slot && (
                        <span className="texture-export-swatch" style={{ backgroundColor: rgbHex(slot.displayColor) }}
                          title={slot.nominalColor ? rgbHex(slot.nominalColor) : t("export.textureUnknown")} />
                      )}
                      <select value={value || ""} aria-label={t("export.textureSlotFor", hex)}
                        onChange={(event) => onAssign(rgb, Number(event.target.value))}>
                        <option value="">{t("export.textureUnassigned")}</option>
                        {slots.map(({ slot, nominalColor }) => (
                          <option key={slot} value={slot}>
                            {nominalColor ? t("export.textureSlot", slot) : t("export.textureSlotUnknown", slot)}
                          </option>
                        ))}
                      </select>
                    </div>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <div className="texture-export-mapping-status">
        <button className="cym-btn texture-export-action" disabled={!canSuggest} onClick={onSuggest}>
          {t("export.textureSuggest")}
        </button>
        <span role="status">{t("export.textureAssignmentCount", numAssigned, rows.length)}</span>
      </div>
    </section>
  );
}
