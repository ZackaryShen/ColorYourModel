import { describe, expect, it } from "vitest";
import { Mesh, Object3D, Vector3 } from "three";
import { PaintTool } from "../../types/mesh";
import { bindGradientPointer, finishGradientGesture, gradientHit, recordGradientHit, startGradientGesture, writeGradientGuide, type GradientHit } from "./gradientGesture";

const stops: [number, number, number, number][] = [
  [255, 0, 0, 255],
  [0, 255, 0, 255],
  [0, 0, 255, 255],
];
const start = { faceId: 1, point: [0, 0, 0] as [number, number, number] };
const bend = { faceId: 1, point: [1, 0, 0] as [number, number, number] };
const end = { faceId: 2, point: [1, 2, 0] as [number, number, number] };
const options = { stops, radius: 2, wholeRegion: false };

describe("gradient gestures", () => {
  it("records the raycast position in mesh coordinates under a rotated parent", () => {
    const parent = new Object3D();
    parent.rotation.x = -Math.PI / 2;
    parent.position.set(3, 4, 5);
    const mesh = new Mesh();
    parent.add(mesh);
    const world = mesh.localToWorld(new Vector3(1, 2, 3));
    const sample = gradientHit(mesh, { faceIndex: 7, point: world });
    expect(sample?.faceId).toBe(7);
    expect(sample?.point[0]).toBeCloseTo(1);
    expect(sample?.point[1]).toBeCloseTo(2);
    expect(sample?.point[2]).toBeCloseTo(3);
  });

  it("keeps sample order and same-face curvature for a brush stroke", () => {
    const stroke = startGradientGesture(PaintTool.Brush, start, options);
    recordGradientHit(stroke, bend);
    expect(finishGradientGesture(stroke, end)).toEqual({
      tool: PaintTool.Brush,
      points: [start, bend, end],
      stops,
      radius: 2,
    });
  });

  it("never joins disjoint runs after the brush leaves the model", () => {
    const stroke = startGradientGesture(PaintTool.Brush, start, options);
    recordGradientHit(stroke, bend);
    recordGradientHit(stroke, null);
    recordGradientHit(stroke, end);
    expect(finishGradientGesture(stroke, end)).toMatchObject({ points: [start, bend] });
  });

  it("retains the end of a very long brush stroke within the backend sample limit", () => {
    const stroke = startGradientGesture(PaintTool.Brush, start, options);
    for (let x = 1; x <= 4300; x++) {
      recordGradientHit(stroke, { faceId: 1, point: [x, 0, 0] });
    }
    const result = finishGradientGesture(stroke, end);
    if (result?.tool !== PaintTool.Brush) throw new Error("Expected brush gesture");
    expect(result.points.length).toBeLessThanOrEqual(4096);
    expect(result.points[0]).toEqual(start);
    expect(result.points[result.points.length - 1]).toEqual(end);
  });

  it("uses the start region and the release direction for fill", () => {
    const gesture = startGradientGesture(PaintTool.Fill, start, { ...options, wholeRegion: true });
    recordGradientHit(gesture, bend);
    expect(finishGradientGesture(gesture, end)).toEqual({
      tool: PaintTool.Fill,
      start,
      end,
      stops,
      radius: 2,
      wholeRegion: true,
    });
  });

  it("declines a fill whose release is off the model or at the starting point", () => {
    const gesture = startGradientGesture(PaintTool.Fill, start, options);
    recordGradientHit(gesture, bend);
    expect(finishGradientGesture(gesture, null)).toBeNull();
    expect(finishGradientGesture(gesture, start)).toBeNull();
  });
});

describe("gradient pointer capture", () => {
  function fixture() {
    const canvas = document.createElement("canvas");
    const committed: unknown[] = [];
    const previewRadii: (number | undefined)[] = [];
    let radius = options.radius;
    let current = true;
    let notifyChange: (() => void) | undefined;
    const dispose = bindGradientPointer(canvas, {
      isCurrent: () => current,
      subscribe: (notify: () => void) => {
        notifyChange = notify;
        return () => { notifyChange = undefined; };
      },
      brushRadius: () => radius,
      start: () => startGradientGesture(PaintTool.Brush, start, { ...options, radius }),
      sample: () => end,
      preview: gesture => { previewRadii.push(gesture?.radius); },
      commit: job => { committed.push(job); },
      invalidFill: () => {},
    });
    const pointer = (type: string, pointerId = 1) => {
      const event = Object.assign(new MouseEvent(type, { button: 0 }), { pointerId });
      canvas.dispatchEvent(event);
    };
    const setCurrent = (value: boolean) => {
      current = value;
      notifyChange?.();
    };
    const setRadius = (value: number) => {
      radius = value;
      notifyChange?.();
    };
    return { committed, pointer, dispose, setCurrent, previewRadii, setRadius };
  }

  it("commits on release only, using the initiating pointer", () => {
    const { committed, pointer, dispose } = fixture();
    pointer("pointerdown");
    pointer("pointermove");
    pointer("pointerup", 2);
    expect(committed).toEqual([]);
    pointer("pointerup");
    expect(committed).toEqual([{ tool: PaintTool.Brush, points: [start, end], stops, radius: 2 }]);
    dispose();
  });

  it("keeps the visible and committed radius in sync when the brush is resized mid-drag", () => {
    const { committed, pointer, dispose, previewRadii, setRadius } = fixture();
    pointer("pointerdown");
    pointer("pointermove");
    expect(previewRadii.at(-1)).toBe(2);
    setRadius(3.5);
    expect(previewRadii.at(-1)).toBe(3.5);
    pointer("pointerup");
    expect(committed).toEqual([{ tool: PaintTool.Brush, points: [start, end], stops, radius: 3.5 }]);
    dispose();
  });

  it("drops the open gesture on pointer cancellation and starts fresh next time", () => {
    const { committed, pointer, dispose } = fixture();
    pointer("pointerdown");
    pointer("pointermove");
    pointer("pointercancel");
    pointer("pointerup");
    expect(committed).toEqual([]);
    pointer("pointerdown");
    pointer("pointerup");
    expect(committed).toHaveLength(1);
    dispose();
  });

  it("drops a release that arrives after the initiating tab has changed", () => {
    const { committed, pointer, setCurrent, dispose } = fixture();
    pointer("pointerdown");
    pointer("pointermove");
    setCurrent(false);
    pointer("pointerup");
    expect(committed).toEqual([]);
    dispose();
  });

  it("cancels when a busy interval starts and ends without pointer movement", () => {
    const { committed, pointer, setCurrent, dispose } = fixture();
    pointer("pointerdown");
    pointer("pointermove");
    setCurrent(false);
    setCurrent(true);
    pointer("pointerup");
    expect(committed).toEqual([]);
    pointer("pointerdown");
    pointer("pointerup");
    expect(committed).toHaveLength(1);
    dispose();
  });

  it("raycasts between sparse brush events across a folded surface", () => {
    const canvas = document.createElement("canvas");
    const committed: GradientHit[][] = [];
    const dispose = bindGradientPointer(canvas, {
      isCurrent: () => true,
      subscribe: () => () => {},
      brushRadius: () => options.radius,
      start: () => startGradientGesture(PaintTool.Brush, start, options),
      sample: ({ clientX }) => ({
        faceId: Math.round(clientX / 8),
        point: [clientX, 0, Math.min(clientX, 48 - clientX)],
      }),
      preview: () => {},
      commit: job => { if (job.tool === PaintTool.Brush) committed.push(job.points); },
      invalidFill: () => {},
    });
    const pointer = (type: string, clientX: number) => {
      canvas.dispatchEvent(Object.assign(new MouseEvent(type, { button: 0, clientX }), { pointerId: 1 }));
    };
    pointer("pointerdown", 0);
    pointer("pointerup", 48);
    expect(committed).toHaveLength(1);
    expect(committed[0].some(hit => hit.point[2] >= 16)).toBe(true);
    expect(committed[0][committed[0].length - 1]?.point).toEqual([48, 0, 0]);
    dispose();
  });
});

it("puts intermediate guide stops at equal distances along the drawn path", () => {
  const gesture = startGradientGesture(PaintTool.Brush, start, options);
  recordGradientHit(gesture, bend);
  recordGradientHit(gesture, end);
  const positions = new Float32Array(18);
  const colors = new Float32Array(18);
  expect(writeGradientGuide(gesture, positions, colors)).toBe(3);
  expect(Array.from(positions.slice(6, 12))).toEqual([1, 0, 0, 1, 0.5, 0]);
  expect(Array.from(colors.slice(9, 12))).toEqual([0, 1, 0]);
  expect(Array.from(colors.slice(15, 18))).toEqual([0, 0, 1]);
});
