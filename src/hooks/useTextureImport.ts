import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useAppStore } from "../store/appStore";
import type { MeshData } from "../types/mesh";
import type { TexturePreview } from "../types/texture";
import { log } from "../utils/logger";

export const TEXTURE_COLOR_MIN = 1;
export const TEXTURE_COLOR_MAX = 16;
const DEFAULT_COLOR_LIMIT = 8;
const PALETTE_DEBOUNCE_MS = 180;

export interface TextureImportSession {
  jobId: number;
  path: string;
  colorLimit: number;
  phase: "preparing" | "ready" | "applying" | "prepare-error" | "apply-error";
  preview: TexturePreview | null;
  error: string | null;
}

let lastJobId = Number(import.meta.hot?.data?.lastTextureJobId ?? 0);
import.meta.hot?.dispose?.((data) => { data.lastTextureJobId = lastJobId; });

function cancelJob(jobId: number) {
  void invoke("cancel_texture", { jobId }).catch((error) => {
    log.warn("textureImport", "Could not cancel texture job", { jobId, error: String(error) });
  });
}

export function useTextureImport() {
  const [session, setSession] = useState<TextureImportSession | null>(null);
  // Read synchronously from events and IPC callbacks, including before React
  // has committed a render for the latest job.
  const current = useRef<TextureImportSession | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const startedJobId = useRef<number | null>(null);

  const update = (next: TextureImportSession | null) => {
    current.current = next;
    setSession(next);
  };

  useEffect(() => () => {
    window.clearTimeout(timer.current);
    if (startedJobId.current !== null && current.current?.phase !== "applying") {
      cancelJob(startedJobId.current);
    }
    current.current = null;
    startedJobId.current = null;
  }, []);

  const prepare = (path: string, colorLimit: number, delay: number) => {
    window.clearTimeout(timer.current);
    const next: TextureImportSession = {
      jobId: lastJobId = Math.max(Date.now(), lastJobId + 1),
      path,
      colorLimit,
      phase: "preparing",
      preview: current.current?.preview ?? null,
      error: null,
    };
    update(next);

    if (startedJobId.current !== null) {
      cancelJob(startedJobId.current);
      startedJobId.current = null;
    }

    const run = async () => {
      if (current.current?.jobId !== next.jobId) return;
      const { jobId } = next;
      startedJobId.current = jobId;
      try {
        const preview = await invoke<TexturePreview>("prepare_texture", { path, jobId, colorLimit });
        if (current.current?.jobId === jobId) update({ ...next, preview, phase: "ready" });
      } catch (error) {
        if (current.current?.jobId === jobId) {
          update({ ...next, phase: "prepare-error", error: String(error) });
        }
      }
    };

    if (delay) timer.current = window.setTimeout(() => void run(), delay);
    else void run();
  };

  const open = (path: string) => {
    if (current.current || useAppStore.getState().isLoading) return;
    prepare(path, DEFAULT_COLOR_LIMIT, 0);
  };

  const setColorLimit = (colorLimit: number) => {
    const active = current.current;
    if (!active || active.phase === "applying" || active.colorLimit === colorLimit) return;
    if (!Number.isInteger(colorLimit) || colorLimit < TEXTURE_COLOR_MIN || colorLimit > TEXTURE_COLOR_MAX) return;
    prepare(active.path, colorLimit, PALETTE_DEBOUNCE_MS);
  };

  const retry = () => {
    const active = current.current;
    if (active?.error) prepare(active.path, active.colorLimit, 0);
  };

  const cancel = () => {
    if (current.current?.phase === "applying") return;
    window.clearTimeout(timer.current);
    update(null);
    if (startedJobId.current !== null) {
      cancelJob(startedJobId.current);
      startedJobId.current = null;
    }
  };

  const apply = async () => {
    const active = current.current;
    if (active?.phase !== "ready" || useAppStore.getState().isLoading) return;
    const { jobId } = active;
    update({ ...active, phase: "applying" });
    try {
      const data = await invoke<MeshData>("apply_texture", { jobId });
      if (current.current?.jobId !== jobId) return;
      const { setMeshData, markPaintDirty } = useAppStore.getState();
      setMeshData(data);
      markPaintDirty();
      startedJobId.current = null;
      update(null);
    } catch (error) {
      if (current.current?.jobId === jobId) {
        update({ ...active, phase: "apply-error", error: String(error) });
      }
    }
  };

  return {
    session,
    open,
    setColorLimit,
    retry,
    apply,
    cancel,
    isOpen: () => current.current !== null,
    isApplying: () => current.current?.phase === "applying",
  };
}
