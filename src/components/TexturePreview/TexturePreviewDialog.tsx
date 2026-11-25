import { useEffect, useRef, useState } from "react";
import { useT } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { TEXTURE_COLOR_MAX, TEXTURE_COLOR_MIN, type TextureImportSession } from "../../hooks/useTextureImport";
import type { OriginalTexturePreview } from "../../types/texture";
import { TexturePreviewCanvas, type TextureComparisonMode, type TexturePreviewSide } from "./TexturePreviewCanvas";
import "./TexturePreviewDialog.css";

const COPY = {
  zh: {
    title: "纹理转涂色",
    original: "原始贴图",
    converted: "转换结果",
    comparison: "对比方式",
    split: "并排",
    single: "单窗",
    overlay: "滑条",
    visibleModel: "预览内容",
    reset: "重置视角",
    divider: "对比分界",
    canvas: "模型预览，拖动旋转，滚动缩放",
    settings: "转换设置",
    limit: "色板上限",
    result: "结果色板",
    before: "原网格",
    after: "转换后",
    warnings: {
      budget: "已达到细分面数预算。可应用当前预览，细小纹理可能简化。",
      precision: "坐标精度限制了进一步细分。可应用当前预览。",
      "level-limit": "已达到细分层数上限。可应用当前预览。",
    },
    progress: "正在转换纹理…",
    applying: "正在应用涂色…",
    prepareError: "无法转换纹理",
    applyError: "无法应用涂色",
    rendererError: "无法显示模型预览",
    imageLoading: "正在显示原始贴图…",
    discard: "应用后，当前模型和未保存的涂装将被替换。",
    retry: "重试",
    apply: "确认应用",
    replace: "应用并替换模型",
    cancel: "取消",
    faces: (count: number) => count.toLocaleString("zh") + " 面",
    colors: (count: number) => count.toLocaleString("zh") + " 色",
  },
  en: {
    title: "Texture to paint",
    original: "Original texture",
    converted: "Converted result",
    comparison: "Comparison mode",
    split: "Side by side",
    single: "Single view",
    overlay: "Slider",
    visibleModel: "Model to preview",
    reset: "Reset view",
    divider: "Comparison divider",
    canvas: "Model preview. Drag to rotate or scroll to zoom",
    settings: "Conversion settings",
    limit: "Palette limit",
    result: "Result palette",
    before: "Original mesh",
    after: "Converted mesh",
    warnings: {
      budget: "The subdivision face budget was reached. You can apply this preview. Fine details may be simplified.",
      precision: "Coordinate precision limits further subdivision. You can apply this preview.",
      "level-limit": "The subdivision level limit was reached. You can apply this preview.",
    },
    progress: "Converting texture…",
    applying: "Applying paint…",
    prepareError: "Could not convert the texture",
    applyError: "Could not apply the paint",
    rendererError: "Could not display the model preview",
    imageLoading: "Loading the original texture…",
    discard: "Applying will replace the current model and its unsaved paint.",
    retry: "Retry",
    apply: "Apply paint",
    replace: "Apply and replace model",
    cancel: "Cancel",
    faces: (count: number) => count.toLocaleString("en") + (count === 1 ? " face" : " faces"),
    colors: (count: number) => count.toLocaleString("en") + (count === 1 ? " color" : " colors"),
  },
};

interface PreviewDialogProps {
  session: TextureImportSession;
  hasModel: boolean;
  onApply: () => void;
  onCancel: () => void;
  onColorLimit: (count: number) => void;
  onRetry: () => void;
}

export function TexturePreviewDialog({ session, hasModel, onApply, onCancel, onColorLimit, onRetry }: PreviewDialogProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  const language = useAppStore((state) => state.language);
  const theme = useAppStore((state) => state.theme);
  const setTheme = useAppStore((state) => state.setTheme);
  const busy = useAppStore((state) => state.isLoading);
  const dirty = useAppStore((state) => state.paintDirty);
  const copy = COPY[language];
  const t = useT();

  const [mode, setMode] = useState<TextureComparisonMode>("split");
  const [single, setSingle] = useState<TexturePreviewSide>("converted");
  const [divider, setDivider] = useState(50);
  const [resetKey, setResetKey] = useState(0);
  const [renderedSource, setRenderedSource] = useState<OriginalTexturePreview | null>(null);
  const [renderError, setRenderError] = useState(false);

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    element.showModal();
    return () => { if (element.open) element.close(); };
  }, []);

  const { phase, path, colorLimit, preview, error } = session;
  const applying = phase === "applying";
  const preparing = phase === "preparing";
  const ready = !!preview && renderedSource === preview.original && !renderError;
  const canApply = phase === "ready" && ready && !busy;
  const fileName = path.split(/[\\/]/).pop();
  const showOriginal = mode === "split" || (mode === "overlay" && divider > 0) || (mode === "single" && single === "original");
  const showConverted = mode === "split" || (mode === "overlay" && divider < 100) || (mode === "single" && single === "converted");
  const currentError = error ? (
    <div className="texture-preview-error" role="alert">
      <strong>{phase === "apply-error" ? copy.applyError : copy.prepareError}</strong>
      <div>{error}</div>
    </div>
  ) : null;

  return (
    <dialog
      ref={dialog}
      className="texture-preview-dialog"
      aria-labelledby="texture-preview-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!applying) onCancel();
      }}
      onKeyDown={(event) => event.stopPropagation()}
    >
      <header className="texture-preview-header">
        <div className="texture-preview-source">
          <h1 id="texture-preview-title">{copy.title}</h1>
          <p><span>GLB</span><code title={path}>{fileName}</code></p>
        </div>
        <button className="texture-preview-button cym-btn" onClick={() => setTheme(theme === "dark" ? "light" : "dark")}>
          {t(theme === "dark" ? "menu.themeToLight" : "menu.themeToDark")}
        </button>
      </header>

      <div className="texture-preview-body">
        <section className="texture-preview-visual">
          <div className="texture-preview-stage">
            {preview ? (
              <>
                <TexturePreviewCanvas
                  original={preview.original}
                  converted={preview.converted}
                  mode={mode}
                  single={single}
                  divider={divider}
                  resetKey={resetKey}
                  label={copy.canvas}
                  onReady={() => { setRenderedSource(preview.original); setRenderError(false); }}
                  onError={() => setRenderError(true)}
                />
                {showOriginal && <span className="texture-preview-tag">{copy.original}</span>}
                {showConverted && (
                  <span className={"texture-preview-tag texture-preview-tag-result texture-preview-tag-" + mode}>
                    {copy.converted}
                  </span>
                )}
                {mode === "split" && <span className="texture-preview-divider" style={{ left: "50%" }} />}
                {mode === "overlay" && divider > 0 && divider < 100 && (
                  <span className="texture-preview-divider" style={{ left: divider + "%" }} />
                )}
                {renderError ? (
                  <div className="texture-preview-center" role="alert">{copy.rendererError}</div>
                ) : !ready && !preparing ? (
                  <div className="texture-preview-center" role="status">{copy.imageLoading}</div>
                ) : null}
              </>
            ) : (
              <div className="texture-preview-center">
                {currentError ?? <span role="status">{copy.progress}</span>}
              </div>
            )}
          </div>

          {preview && (
            <>
              <div className="texture-preview-controls">
                <div className="texture-preview-group" role="group" aria-label={copy.comparison}>
                  {(["split", "single", "overlay"] as const).map((choice) => (
                    <button key={choice} className="cym-btn" aria-pressed={choice === mode} onClick={() => setMode(choice)}>
                      {copy[choice]}
                    </button>
                  ))}
                </div>
                {mode === "single" && (
                  <div className="texture-preview-group" role="group" aria-label={copy.visibleModel}>
                    {(["original", "converted"] as const).map((choice) => (
                      <button key={choice} className="cym-btn" aria-pressed={choice === single} onClick={() => setSingle(choice)}>
                        {copy[choice]}
                      </button>
                    ))}
                  </div>
                )}
                <button className="texture-preview-reset cym-btn" onClick={() => setResetKey((count) => count + 1)}>
                  {copy.reset}
                </button>
              </div>
              {mode === "overlay" && (
                <label className="texture-preview-wipe">
                  {copy.divider}
                  <input type="range" min={0} max={100} value={divider} onChange={(event) => setDivider(Number(event.currentTarget.value))} />
                </label>
              )}
            </>
          )}
        </section>

        <aside className="texture-preview-settings" aria-labelledby="texture-preview-settings-title">
          <h2 id="texture-preview-settings-title">{copy.settings}</h2>
          <label className="texture-preview-limit" htmlFor="texture-palette-limit">
            {copy.limit}<output htmlFor="texture-palette-limit">{colorLimit}</output>
          </label>
          <input id="texture-palette-limit" type="range" min={TEXTURE_COLOR_MIN} max={TEXTURE_COLOR_MAX}
            value={colorLimit} disabled={applying} onChange={(event) => onColorLimit(Number(event.currentTarget.value))} />
          <div className="texture-preview-ticks" aria-hidden="true"><span>{TEXTURE_COLOR_MIN}</span><span>{TEXTURE_COLOR_MAX}</span></div>

          <h3>{copy.result}{preview && " · " + copy.colors(preview.palette.length)}</h3>
          {preview && (
            <ul className="texture-preview-swatches" aria-label={copy.result}>
              {preview.palette.map(([r, g, b], index) => {
                const rgb = "#" + [r, g, b].map((byte) => byte.toString(16).padStart(2, "0")).join("").toUpperCase();
                return <li key={rgb + "-" + index} style={{ backgroundColor: rgb }} title={rgb} aria-label={rgb} />;
              })}
            </ul>
          )}
          <dl>
            <div><dt>{copy.before}</dt><dd>{preview ? copy.faces(preview.original.faces.length / 3) : "—"}</dd></div>
            <div><dt>{copy.after}</dt><dd>{preview ? copy.faces(preview.converted.faceCount) : "—"}</dd></div>
          </dl>
          {phase === "ready" && preview?.warning && (
            <p className="texture-preview-warning" role="status">{copy.warnings[preview.warning]}</p>
          )}
        </aside>
      </div>

      {preview && currentError}
      <footer className="texture-preview-footer">
        <div className="texture-preview-note">
          {applying || (preparing && preview) ? (
            <span role="status">{applying ? copy.applying : copy.progress}</span>
          ) : hasModel && dirty ? copy.discard : null}
        </div>
        <button className="texture-preview-button cym-btn" disabled={applying} onClick={onCancel}>{copy.cancel}</button>
        {error ? (
          <button className="texture-preview-button texture-preview-primary cym-btn" onClick={onRetry}>{copy.retry}</button>
        ) : (
          <button className="texture-preview-button texture-preview-primary cym-btn" disabled={!canApply} onClick={onApply}>
            {hasModel ? copy.replace : copy.apply}
          </button>
        )}
      </footer>
    </dialog>
  );
}
