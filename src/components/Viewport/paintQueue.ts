/** Serial paint queue shared by every pointer-driven paint tool.
 *
 * A drag fires one async backend call per pointermove. Fire-and-forget invokes
 * returned OUT OF ORDER while `updateFaceColors` wrote them synchronously, so
 * overlapping brush strokes overwrote each other — the "color not following
 * the cursor" defect (iteration 15, req #2). Draining strictly in enqueue
 * order guarantees faces are colored in the order the cursor visited them.
 *
 * Latest-wins consumer throttle (iteration 16, REFUTE blocker-3): while a
 * Tauri paint IPC is in flight, only the MOST RECENT job is kept. The queue
 * never grows unbounded, so the visual paint lags by at most one round-trip,
 * and intermediate faces are covered by the brush's radius flood-fill.
 *
 * Every job carries the `strokeId` minted at pointerdown. The backend
 * coalesces consecutive paint calls sharing a `stroke_id` into ONE undo entry
 * (`mesh/history.rs`), so a single brush drag is one Ctrl+Z step. Dropping the
 * field on the way through this queue — exactly what v0.1.1 did (issue #20) —
 * made the backend see `stroke_id: None` for every sample and peeled each
 * stroke back one pointermove at a time. `paintQueue.test.ts` pins the
 * forwarding.
 */

/** One unit of paint work, as produced by a pointerdown/pointermove sample. */
export interface PaintJob {
  faceId: number;
  wholeRegion: boolean;
  hoveredSegment?: number | null;
  renderedHover?: number | null;
  /** Stroke id minted at pointerdown; the backend's undo-coalescing key. */
  strokeId?: number | null;
  /** 0.2.0-P2 gradient (path mode): precomputed sample color. */
  colorOverride?: [number, number, number, number];
  /** 0.2.0-P2 gradient (radial mode): one-shot graded disc. */
  gradientRadial?: boolean;
}

export interface SerialPaintQueue<Job> {
  enqueue(job: Job): void;
}

/** Create the drain-serially, latest-wins queue. `fn` is the backend call;
 *  a job whose `fn` rejects is swallowed — one failed face must not kill the
 *  queue, the next enqueue restarts the drain. */
export function createSerialPaintQueue<Job>(
  fn: (job: Job) => Promise<void>
): SerialPaintQueue<Job> {
  let pending: Job | null = null;
  let draining = false;
  return {
    enqueue(job: Job) {
      pending = job; // busy → drop intermediate (covered by radius)
      if (draining) return;
      draining = true;
      void (async () => {
        while (pending != null) {
          const next = pending;
          pending = null;
          try {
            await fn(next);
          } catch {
            /* ignore single-face failures */
          }
        }
        draining = false;
      })();
    },
  };
}
