/** Serialise edit commands and their render updates. Adjacent pending solid
 * samples coalesce. Completed gestures and history actions keep their place. */

/** One input sample for the solid paint tools. */
export interface PaintJob {
  faceId: number;
  wholeRegion: boolean;
  hoveredSegment?: number | null;
  renderedHover?: number | null;
  /** The backend groups samples with the same pointerdown id into one undo entry. */
  strokeId?: number | null;
}

export interface SerialPaintQueue<Job> {
  enqueue(job: Job, kind?: "sample" | "gesture"): void;
  enqueueAction(job: Job): Promise<void>;
  cancel(): void;
}

type Pending<Job> = {
  job: Job;
  kind: "sample" | "gesture";
  resolve?: () => void;
  reject?: (error: unknown) => void;
};

/** Process a job and apply its result before starting the next one. */
export function createSerialPaintQueue<Job>(
  fn: (job: Job) => Promise<void>
): SerialPaintQueue<Job> {
  const pending: Pending<Job>[] = [];
  let draining = false;
  const add = (entry: Pending<Job>) => {
    if (entry.kind === "sample" && pending[pending.length - 1]?.kind === "sample") pending.pop();
    pending.push(entry);
    if (draining) return;
    draining = true;
    void (async () => {
      for (let next = pending.shift(); next; next = pending.shift()) {
        try {
          await fn(next.job);
          next.resolve?.();
        } catch (error) {
          next.reject?.(error);
        }
      }
      draining = false;
    })();
  };
  return {
    cancel() {
      for (const entry of pending.splice(0)) entry.resolve?.();
    },
    enqueue(job: Job, kind = "sample") {
      add({ job, kind });
    },
    enqueueAction(job: Job) {
      return new Promise<void>((resolve, reject) => add({ job, kind: "gesture", resolve, reject }));
    },
  };
}
