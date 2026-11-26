import type { Intersection, Object3D } from "three";
import { PaintTool, type GradientHit } from "../../types/mesh";
import type { GradientColor, GradientTool } from "../../utils/gradientPaint";

// Match the backend cap. Compaction keeps both ends of unusually long drags.
export const MAX_GRADIENT_POINTS = 4096;

export type { GradientHit } from "../../types/mesh";

interface GradientOptions {
  stops: GradientColor[];
  radius: number;
  wholeRegion: boolean;
}

export interface GradientGesture extends GradientOptions {
  tool: GradientTool;
  start: GradientHit;
  points: GradientHit[];
  interrupted: boolean;
}

export type GradientPaintJob =
  | { tool: PaintTool.Brush; points: GradientHit[]; stops: GradientColor[]; radius: number }
  | { tool: PaintTool.Fill; start: GradientHit; end: GradientHit; stops: GradientColor[]; radius: number; wholeRegion: boolean };

export function gradientHit(mesh: Object3D | null, hit: Pick<Intersection, "faceIndex" | "point"> | null): GradientHit | null {
  if (!mesh || hit?.faceIndex == null) return null;
  const point = mesh.worldToLocal(hit.point.clone());
  return { faceId: hit.faceIndex, point: [point.x, point.y, point.z] };
}

export function startGradientGesture(tool: GradientTool, start: GradientHit, options: GradientOptions): GradientGesture {
  return { ...options, tool, start, points: [start], interrupted: false };
}

export function gradientDistance(a: GradientHit, b: GradientHit): number {
  return Math.hypot(a.point[0] - b.point[0], a.point[1] - b.point[1], a.point[2] - b.point[2]);
}

export function recordGradientHit(gesture: GradientGesture, hit: GradientHit | null) {
  if (gesture.tool === PaintTool.Fill) {
    gesture.points.length = 1;
    if (hit && gradientDistance(gesture.start, hit) > 1e-5) gesture.points.push(hit);
    return;
  }
  if (!hit) {
    // Joining across background could draw through a hole or onto a far shell.
    gesture.interrupted = true;
    return;
  }
  if (gesture.interrupted) return;
  const previous = gesture.points[gesture.points.length - 1];
  if (hit.faceId === previous.faceId && gradientDistance(hit, previous) <= 1e-5) return;
  if (gesture.points.length === MAX_GRADIENT_POINTS) {
    gesture.points = gesture.points.filter((_, index) => index % 2 === 0);
  }
  gesture.points.push(hit);
}

export function finishGradientGesture(gesture: GradientGesture, end: GradientHit | null): GradientPaintJob | null {
  recordGradientHit(gesture, end);
  const { tool, start, points, stops, radius, wholeRegion } = gesture;
  if (tool === PaintTool.Brush) return { tool, points, stops, radius };
  if (end && points.length === 2) return { tool, start, end, stops, radius, wholeRegion };
  return null;
}

interface GradientPointerHandlers {
  isCurrent: () => boolean;
  subscribe: (onChange: () => void) => () => void;
  brushRadius: () => number;
  start: (event: PointerEvent) => GradientGesture | null;
  sample: (point: ClientPoint) => GradientHit | null;
  preview: (gesture: GradientGesture | null) => void;
  commit: (job: GradientPaintJob) => void;
  invalidFill: () => void;
}

type ClientPoint = Pick<PointerEvent, "clientX" | "clientY">;
type ActiveGesture = { pointerId: number; gesture: GradientGesture; lastPoint: ClientPoint };

// Sparse pointer events can span an entire fold. Trace the screen interval
// with more rays before handing the resulting surface path to Rust.
function sampleMovement(active: ActiveGesture, point: ClientPoint, sample: GradientPointerHandlers["sample"]) {
  const { gesture, lastPoint } = active;
  const dx = point.clientX - lastPoint.clientX;
  const dy = point.clientY - lastPoint.clientY;
  const steps = Math.min(64, Math.ceil(Math.hypot(dx, dy) / 8));
  if (gesture.tool === PaintTool.Brush) {
    for (let step = 1; step < steps && !gesture.interrupted; step++) {
      recordGradientHit(gesture, sample({
        clientX: lastPoint.clientX + dx * step / steps,
        clientY: lastPoint.clientY + dy * step / steps,
      }));
    }
  }
  active.lastPoint = point;
  return sample(point);
}

export function bindGradientPointer(canvas: HTMLCanvasElement, handlers: GradientPointerHandlers): () => void {
  let active: ActiveGesture | null = null;

  function clear() {
    if (!active) return;
    const { pointerId } = active;
    active = null;
    handlers.preview(null);
    if (canvas.hasPointerCapture?.(pointerId)) canvas.releasePointerCapture(pointerId);
  }

  const down = (event: PointerEvent) => {
    if (active || event.button !== 0 || !handlers.isCurrent()) return;
    const gesture = handlers.start(event);
    if (!gesture) return;
    // On-model gradient drags own the pointer before OrbitControls sees it.
    event.stopImmediatePropagation();
    active = { pointerId: event.pointerId, gesture, lastPoint: event };
    try { canvas.setPointerCapture?.(event.pointerId); } catch { clear(); return; }
    handlers.preview(gesture);
  };

  const move = (event: PointerEvent) => {
    if (active?.pointerId !== event.pointerId) return;
    event.stopImmediatePropagation();
    if (!handlers.isCurrent()) { clear(); return; }
    const samples = event.getCoalescedEvents?.();
    for (const sample of samples?.length ? samples : [event]) {
      recordGradientHit(active.gesture, sampleMovement(active, sample, handlers.sample));
    }
    handlers.preview(active.gesture);
  };

  const up = (event: PointerEvent) => {
    if (active?.pointerId !== event.pointerId) return;
    event.stopImmediatePropagation();
    const current = handlers.isCurrent();
    const job = current ? finishGradientGesture(active.gesture, sampleMovement(active, event, handlers.sample)) : null;
    clear();
    if (job) handlers.commit(job);
    else if (current) handlers.invalidFill();
  };

  const cancel = (event: PointerEvent) => {
    if (active?.pointerId === event.pointerId) clear();
  };

  const leave = (event: PointerEvent) => {
    if (active?.pointerId === event.pointerId && !canvas.hasPointerCapture?.(event.pointerId)) clear();
  };

  const keydown = (event: KeyboardEvent) => {
    if (event.key === "Escape") clear();
  };

  canvas.addEventListener("pointerdown", down, true);
  canvas.addEventListener("pointermove", move, true);
  canvas.addEventListener("pointerup", up, true);
  canvas.addEventListener("pointercancel", cancel);
  canvas.addEventListener("lostpointercapture", cancel);
  canvas.addEventListener("pointerleave", leave);
  window.addEventListener("blur", clear);
  window.addEventListener("keydown", keydown);
  const unsubscribe = handlers.subscribe(() => {
    if (!active) return;
    if (!handlers.isCurrent()) { clear(); return; }
    const gesture = active.gesture;
    const radius = handlers.brushRadius();
    if (gesture.tool === PaintTool.Brush && radius !== gesture.radius) {
      // The brush commits a single radius for the whole gesture.
      gesture.radius = radius;
      handlers.preview(gesture);
    }
  });
  return () => {
    unsubscribe();
    canvas.removeEventListener("pointerdown", down, true);
    canvas.removeEventListener("pointermove", move, true);
    canvas.removeEventListener("pointerup", up, true);
    canvas.removeEventListener("pointercancel", cancel);
    canvas.removeEventListener("lostpointercapture", cancel);
    canvas.removeEventListener("pointerleave", leave);
    window.removeEventListener("blur", clear);
    window.removeEventListener("keydown", keydown);
    clear();
  };
}

// Only an in-progress Brush stroke previews the paint footprint. The Fill
// direction and the optional saved centreline stay legible at every zoom.
export function gradientGuideAppearance(
  live: Pick<GradientGesture, "tool" | "radius"> | null,
  pixelsPerMillimeter: number,
) {
  return live?.tool === PaintTool.Brush
    ? { linewidth: live.radius * 2 * pixelsPerMillimeter, outline: false }
    : { linewidth: 3, outline: true };
}

/** Draw the path itself, inserting a vertex wherever it crosses a colour stop. */
export function writeGradientGuide(gesture: Pick<GradientGesture, "points" | "stops">, positions: Float32Array, colors: Float32Array): number {
  const { points, stops } = gesture;
  let total = 0;
  for (let i = 1; i < points.length; i++) total += gradientDistance(points[i - 1], points[i]);
  if (total <= 0) return 0;

  let vertex = 0;
  let travelled = 0;
  const lastStop = stops.length - 1;
  const stopSpacing = total / lastStop;
  for (let i = 1; i < points.length; i++) {
    const from = points[i - 1];
    const to = points[i];
    const length = gradientDistance(from, to);
    if (length <= 0) continue;
    const end = travelled + length;
    const write = (distance: number) => {
      const u = (distance - travelled) / length;
      const stopPosition = Math.min(lastStop, distance / stopSpacing);
      const stopIndex = Math.min(lastStop - 1, Math.floor(stopPosition));
      const mix = stopPosition - stopIndex;
      for (let axis = 0; axis < 3; axis++) {
        positions[vertex + axis] = from.point[axis] + u * (to.point[axis] - from.point[axis]);
        colors[vertex + axis] = (stops[stopIndex][axis] * (1 - mix) + stops[stopIndex + 1][axis] * mix) / 255;
      }
      vertex += 3;
    };
    write(travelled);
    for (let stop = Math.floor(travelled / stopSpacing) + 1; stop < lastStop; stop++) {
      const boundary = stop * stopSpacing;
      if (boundary >= end) break;
      write(boundary);
      write(boundary);
    }
    write(end);
    travelled = end;
  }
  return vertex / 6;
}
