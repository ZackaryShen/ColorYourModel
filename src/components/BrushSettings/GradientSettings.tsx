import { useId } from "react";
import { useT } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { PaintTool } from "../../types/mesh";
import { toHex } from "../../utils/colorFormat";
import { MAX_GRADIENT_STOPS, MIN_GRADIENT_STOPS, type GradientColor } from "../../utils/gradientPaint";
import "./gradientSettings.css";

export function GradientSettings({ unavailable }: { unavailable: boolean }) {
  const t = useT();
  const stops = useAppStore(s => s.gradientStops);
  const setStops = useAppStore(s => s.setGradientStops);
  const enabled = useAppStore(s => s.gradientEnabled) && !unavailable;
  const setEnabled = useAppStore(s => s.setGradientEnabled);
  const context = useAppStore(s => s.gradientContext);
  const showSavedGuide = useAppStore(s => s.showSavedGradientGuide);
  const setSavedGuide = useAppStore(s => s.setSavedGradientGuide);
  const hintId = useId();

  function changeStop(index: number, hex: string) {
    const rgb = Number.parseInt(hex.slice(1), 16);
    const color: GradientColor = [(rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255, 255];
    setStops(stops.map((stop, i) => i === index ? color : stop));
  }

  function moveStop(index: number, delta: number) {
    const next = [...stops];
    const [stop] = next.splice(index, 1);
    next.splice(index + delta, 0, stop);
    setStops(next);
  }

  return (
    <div className="gradient-settings">
      <label className="gradient-settings__toggle">
        <input
          type="checkbox"
          checked={enabled}
          disabled={unavailable}
          aria-describedby={unavailable ? hintId : undefined}
          onChange={e => setEnabled(e.currentTarget.checked)}
        />
        <span>{t("gradient.mode")}</span>
      </label>
      {unavailable && <p id={hintId}>{t("gradient.textureUnavailable")}</p>}
      {enabled && (
        <fieldset className="gradient-settings__stops">
          <legend>{t("gradient.stops")}</legend>
          <div
            className="gradient-settings__preview"
            style={{ background: `linear-gradient(90deg, ${stops.map(toHex).join(", ")})` }}
          />
          <ol>
            {stops.map((color, index) => {
              const number = index + 1;
              const up = t("gradient.moveUp", number);
              const down = t("gradient.moveDown", number);
              const remove = t("gradient.remove", number);
              return (
                <li key={index}>
                  <label>
                    <input
                      type="color"
                      aria-label={t("gradient.stop", number)}
                      value={toHex(color)}
                      onChange={e => changeStop(index, e.currentTarget.value)}
                    />
                    <span>{toHex(color).toUpperCase()}</span>
                  </label>
                  <button type="button" className="cym-btn" title={up} aria-label={up} disabled={index === 0} onClick={() => moveStop(index, -1)}>↑</button>
                  <button type="button" className="cym-btn" title={down} aria-label={down} disabled={number === stops.length} onClick={() => moveStop(index, 1)}>↓</button>
                  <button type="button" className="cym-btn" title={remove} aria-label={remove} disabled={stops.length <= MIN_GRADIENT_STOPS} onClick={() => setStops(stops.filter((_, i) => i !== index))}>×</button>
                </li>
              );
            })}
          </ol>
          <button
            type="button"
            className="cym-btn gradient-settings__add"
            disabled={stops.length >= MAX_GRADIENT_STOPS}
            onClick={() => {
              const [r, g, b] = useAppStore.getState().currentColor;
              setStops([...stops, [r, g, b, 255]]);
            }}
          >
            {t("gradient.add")}
          </button>
          {context && context.points.length > 1 && (
            <button
              type="button"
              className="cym-btn gradient-settings__saved-guide"
              aria-pressed={showSavedGuide}
              onClick={() => setSavedGuide(!showSavedGuide)}
            >
              {context.tool === PaintTool.Fill ? t("gradient.savedDirection") : t("gradient.savedPath")}
            </button>
          )}
        </fieldset>
      )}
    </div>
  );
}
