import { describe, it, expect } from "vitest";
import { createSerialPaintQueue, type PaintJob } from "./paintQueue";

// Regression tests for issue #20: v0.1.1's enqueue→drain loop forwarded
// wholeRegion/hoveredSegment/renderedHover but NOT strokeId, so the backend
// received `stroke_id: None` for every pointermove sample, never coalesced,
// and one Ctrl+Z peeled back a single sample of the drag. These tests pin the
// forwarding at the layer where the field was dropped.

/** Let the drain's microtasks/macrotasks run. One setTimeout turn is enough
 *  for one `await` inside the consumer. */
const flush = () => new Promise<void>((r) => setTimeout(r, 0));

const job = (over: Partial<PaintJob> = {}): PaintJob => ({
  faceId: 0,
  wholeRegion: false,
  strokeId: 1,
  ...over,
});

describe("createSerialPaintQueue", () => {
  it("delivers every job that reaches the consumer with its strokeId intact (#20)", async () => {
    const received: PaintJob[] = [];
    const q = createSerialPaintQueue<PaintJob>(async (j) => {
      await flush();
      received.push(j);
    });

    // One drag: pointerdown + five pointermove samples, one stroke id.
    const enqueued = [10, 11, 12, 13, 14, 15].map((face) =>
      job({ faceId: face, strokeId: 7 })
    );
    for (const j of enqueued) q.enqueue(j);
    for (let i = 0; i < 10; i++) await flush();

    expect(received.length).toBeGreaterThan(0);
    // THE regression: v0.1.1 delivered `strokeId: undefined`, the backend saw
    // None, and every sample became its own undo entry ("Ctrl+Z peels the
    // stroke apart"). Each delivered job must carry the drag's id verbatim.
    for (const j of received) expect(j.strokeId).toBe(7);
    // Latest-wins: the most recent enqueue is never dropped.
    expect(received[received.length - 1]).toBe(enqueued[enqueued.length - 1]);
    // Delivered jobs are the enqueued objects, not rebuilt/shaped copies.
    for (const j of received) expect(enqueued).toContain(j);
  });

  it("keeps only the most recent job while a call is in flight (latest-wins)", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const delivered: number[] = [];
    const q = createSerialPaintQueue<PaintJob>(async (j) => {
      if (j.faceId === 0) await gate; // hold the first call in flight
      delivered.push(j.faceId);
    });

    q.enqueue(job({ faceId: 0 }));
    q.enqueue(job({ faceId: 1 }));
    q.enqueue(job({ faceId: 2 }));

    release();
    await flush();
    expect(delivered).toEqual([0, 2]);
  });

  it("a new drag's strokeId reaches the consumer without bleeding from the previous stroke", async () => {
    const received: number[] = [];
    const q = createSerialPaintQueue<PaintJob>(async (j) => {
      await flush();
      received.push(j.strokeId as number);
    });

    q.enqueue(job({ strokeId: 7 }));
    await flush();
    await flush();
    q.enqueue(job({ strokeId: 8 }));
    await flush();
    await flush();

    expect(received).toEqual([7, 8]);
  });

  it("a failing job does not kill the queue", async () => {
    const delivered: number[] = [];
    const q = createSerialPaintQueue<PaintJob>(async (j) => {
      if (j.faceId === 1) throw new Error("ipc down");
      delivered.push(j.faceId);
    });

    q.enqueue(job({ faceId: 0 }));
    await flush();
    q.enqueue(job({ faceId: 1 }));
    await flush();
    q.enqueue(job({ faceId: 2 }));
    await flush();

    expect(delivered).toEqual([0, 2]);
  });
});
