import { useFrame, useThree } from "@react-three/fiber";
import { useEffect, useState, type MutableRefObject } from "react";
import { DynamicDrawUsage, type InterleavedBufferAttribute } from "three";
import { LineMaterial } from "three/examples/jsm/lines/LineMaterial.js";
import { LineSegments2 } from "three/examples/jsm/lines/LineSegments2.js";
import { LineSegmentsGeometry } from "three/examples/jsm/lines/LineSegmentsGeometry.js";
import { MAX_GRADIENT_STOPS } from "../../utils/gradientPaint";
import type { GradientPaintContext } from "../../types/mesh";
import { gradientGuideAppearance, MAX_GRADIENT_POINTS, writeGradientGuide, type GradientGesture, type GradientHit } from "./gradientGesture";

export function GradientGuide({ preview, saved }: {
  preview: MutableRefObject<GradientGesture | null>;
  saved: GradientPaintContext | null;
}) {
  const invalidate = useThree(s => s.invalidate);
  useEffect(() => { invalidate(); }, [saved, invalidate]);
  const [guide] = useState(() => {
    const capacity = (MAX_GRADIENT_POINTS + MAX_GRADIENT_STOPS) * 6;
    const geometry = new LineSegmentsGeometry();
    geometry.setPositions(new Float32Array(capacity));
    geometry.setColors(new Float32Array(capacity));
    const positions = (geometry.getAttribute("instanceStart") as InterleavedBufferAttribute).data;
    const colors = (geometry.getAttribute("instanceColorStart") as InterleavedBufferAttribute).data;
    positions.setUsage(DynamicDrawUsage);
    colors.setUsage(DynamicDrawUsage);

    const outline = new LineSegments2(geometry, new LineMaterial({
      color: 0x191919, depthTest: false, depthWrite: false,
    }));
    outline.renderOrder = 30;
    outline.frustumCulled = false;
    outline.visible = false;
    const path = new LineSegments2(geometry, new LineMaterial({
      vertexColors: true, depthTest: false, depthWrite: false,
    }));
    path.renderOrder = 31;
    path.frustumCulled = false;
    path.visible = false;

    return { geometry, positions, colors, outline, path, lastHit: null as GradientHit | null };
  });

  useEffect(() => () => {
    guide.geometry.dispose();
    guide.outline.material.dispose();
    guide.path.material.dispose();
  }, [guide]);

  useFrame(({ camera, size }) => {
    const live = preview.current;
    const gesture = live ?? saved;
    const lastHit = gesture && gesture.points.length > 1 ? gesture.points[gesture.points.length - 1] : null;
    // The orthographic projection maps model millimetres to the CSS pixels
    // LineMaterial uses. Read the current matrix so zoom changes mid-drag
    // also scale the preview.
    const pixelsPerMillimeter = camera.projectionMatrix.elements[5] * size.height / 2;
    const appearance = gradientGuideAppearance(live, pixelsPerMillimeter);
    guide.path.visible = Boolean(lastHit);
    guide.path.material.linewidth = appearance.linewidth;
    guide.outline.visible = appearance.outline && Boolean(lastHit);
    guide.outline.material.linewidth = appearance.linewidth + 2;
    if (guide.lastHit === lastHit) return;
    guide.lastHit = lastHit;
    if (!gesture || !lastHit) return;
    const count = writeGradientGuide(gesture, guide.positions.array as Float32Array, guide.colors.array as Float32Array);
    guide.geometry.instanceCount = count;
    guide.positions.addUpdateRange(0, count * 6);
    guide.colors.addUpdateRange(0, count * 6);
    guide.positions.needsUpdate = true;
    guide.colors.needsUpdate = true;
  });

  return (
    <>
      <primitive object={guide.outline} />
      <primitive object={guide.path} />
    </>
  );
}
