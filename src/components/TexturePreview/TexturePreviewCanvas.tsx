import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import type { MeshData } from "../../types/mesh";
import type { OriginalTexturePreview } from "../../types/texture";
import { log } from "../../utils/logger";
import { createOriginalGeometry, createPreviewGeometry, writePreviewColors } from "./previewGeometry";

export type TextureComparisonMode = "split" | "single" | "overlay";
export type TexturePreviewSide = "original" | "converted";

interface CanvasProps {
  original?: OriginalTexturePreview;
  converted: MeshData;
  mode?: TextureComparisonMode;
  single?: TexturePreviewSide;
  divider?: number;
  resetKey?: number;
  label: string;
  onError?: () => void;
  onReady?: () => void;
}

const WRAP = {
  repeat: THREE.RepeatWrapping,
  clamp: THREE.ClampToEdgeWrapping,
  mirror: THREE.MirroredRepeatWrapping,
} as const;

// glTF stores sampler filters as WebGL enums. Three uses its own values.
const FILTER = {
  9728: THREE.NearestFilter,
  9729: THREE.LinearFilter,
  9984: THREE.NearestMipmapNearestFilter,
  9985: THREE.LinearMipmapNearestFilter,
  9986: THREE.NearestMipmapLinearFilter,
  9987: THREE.LinearMipmapLinearFilter,
} as const;

function litScene(mesh: THREE.Mesh) {
  const scene = new THREE.Scene();
  const key = new THREE.DirectionalLight(0xffffff, 1.45);
  key.position.set(3, 4, 8);
  const fill = new THREE.DirectionalLight(0xffffff, 0.55);
  fill.position.set(-4, -5, 2);
  scene.add(mesh, key, fill, new THREE.AmbientLight(0xffffff, 1.7));
  return scene;
}

function mountRenderer(container: HTMLDivElement, getProps: () => CanvasProps) {
  const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0, 0);
  const canvas = renderer.domElement;
  canvas.tabIndex = 0;
  canvas.style.cssText = "width:100%;height:100%;display:block;cursor:grab";
  container.append(canvas);

  const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 10);
  // The backend normalizes GLB and STL geometry to millimeters with Z up.
  camera.up.set(0, 0, 1);
  camera.position.set(1, 1, 1);
  const controls = new OrbitControls(camera, canvas);
  controls.minZoom = 0.3;
  controls.maxZoom = 16;
  controls.listenToKeyEvents(canvas);

  const original = new THREE.Mesh(
    new THREE.BufferGeometry(),
    new THREE.MeshLambertMaterial({ side: THREE.DoubleSide }),
  );
  original.visible = false;
  const converted = new THREE.Mesh(
    new THREE.BufferGeometry(),
    new THREE.MeshLambertMaterial({ side: THREE.DoubleSide, vertexColors: true }),
  );
  const originalScene = litScene(original);
  const convertedScene = litScene(converted);

  let radius = 1;
  let bounds = new THREE.Box3();
  let width = 0;
  let height = 0;
  let animation = 0;

  const renderPane = (scene: THREE.Scene, left: number, paneWidth: number, clipLeft = left, clipWidth = paneWidth) => {
    if (clipWidth <= 0) return;
    const aspect = paneWidth / height;
    const extent = radius * 1.12 * Math.max(1, 1 / aspect);
    camera.left = -extent * aspect;
    camera.right = extent * aspect;
    camera.top = extent;
    camera.bottom = -extent;
    camera.updateProjectionMatrix();
    renderer.setViewport(left, 0, paneWidth, height);
    renderer.setScissor(clipLeft, 0, clipWidth, height);
    renderer.render(scene, camera);
  };

  const draw = () => {
    animation = 0;
    const nextWidth = container.clientWidth;
    const nextHeight = container.clientHeight;
    if (!nextWidth || !nextHeight) return;
    if (width !== nextWidth || height !== nextHeight) {
      width = nextWidth;
      height = nextHeight;
      renderer.setSize(width, height, false);
    }

    const { mode = "single", single = "converted", divider = 50 } = getProps();
    renderer.setScissorTest(false);
    renderer.clear();
    renderer.setScissorTest(true);
    if (mode === "split") {
      const leftWidth = Math.floor(width / 2);
      renderPane(originalScene, 0, leftWidth);
      renderPane(convertedScene, leftWidth, width - leftWidth);
    } else if (mode === "overlay") {
      // Both scenes retain the full viewport projection. Only their clipping
      // boundary moves, so a feature stays at the same pixel on either side.
      const boundary = Math.round(width * divider / 100);
      renderPane(originalScene, 0, width, 0, boundary);
      renderPane(convertedScene, 0, width, boundary, width - boundary);
    } else {
      renderPane(single === "original" ? originalScene : convertedScene, 0, width);
    }
  };

  const requestDraw = () => {
    if (!animation) animation = window.requestAnimationFrame(draw);
  };
  controls.addEventListener("change", requestDraw);
  const observer = new ResizeObserver(requestDraw);
  observer.observe(container);

  const onContextLost = (event: Event) => {
    event.preventDefault();
    getProps().onError?.();
  };
  const onContextRestored = () => {
    if (!getProps().original || original.visible) getProps().onReady?.();
    requestDraw();
  };
  canvas.addEventListener("webglcontextlost", onContextLost);
  canvas.addEventListener("webglcontextrestored", onContextRestored);

  return {
    canvas,
    original,
    converted,
    requestDraw,
    frame(newBounds: MeshData["bbox"]) {
      const next = new THREE.Box3(new THREE.Vector3(...newBounds.min), new THREE.Vector3(...newBounds.max));
      if (next.equals(bounds)) return;
      bounds = next;
      const sphere = bounds.getBoundingSphere(new THREE.Sphere());
      radius = Math.max(sphere.radius, 0.0001);
      controls.target.copy(sphere.center);
      camera.position.copy(sphere.center).addScaledVector(new THREE.Vector3(1, 1.1, 0.75).normalize(), radius * 3.5);
      camera.far = radius * 12;
      camera.zoom = 1;
      controls.update();
      controls.saveState();
      requestDraw();
    },
    reset: () => {
      controls.reset();
      requestDraw();
    },
    dispose: () => {
      observer.disconnect();
      window.cancelAnimationFrame(animation);
      canvas.removeEventListener("webglcontextlost", onContextLost);
      canvas.removeEventListener("webglcontextrestored", onContextRestored);
      controls.stopListenToKeyEvents();
      controls.dispose();
      original.geometry.dispose();
      original.material.map?.dispose();
      original.material.dispose();
      converted.geometry.dispose();
      converted.material.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      canvas.remove();
    },
  };
}

export function TexturePreviewCanvas(props: CanvasProps) {
  const { original, converted, mode, single, divider, resetKey, label } = props;
  const { vertices, faces, bbox, faceColors } = converted;
  const container = useRef<HTMLDivElement>(null);
  const renderer = useRef<ReturnType<typeof mountRenderer> | null>(null);
  const latest = useRef(props);
  latest.current = props;

  useEffect(() => {
    if (!container.current) return;
    try {
      const mounted = mountRenderer(container.current, () => latest.current);
      renderer.current = mounted;
      return () => {
        renderer.current = null;
        mounted.dispose();
      };
    } catch (error) {
      log.warn("texturePreview", "Could not initialize WebGL", { error: String(error) });
      latest.current.onError?.();
    }
  }, []);

  useEffect(() => {
    const active = renderer.current;
    if (!active) return;
    const mesh = active.converted;
    mesh.geometry.dispose();
    mesh.geometry = createPreviewGeometry(vertices, faces);
    active.requestDraw();
  }, [vertices, faces]);

  useEffect(() => {
    const active = renderer.current;
    if (!active) return;
    writePreviewColors(active.converted.geometry, faceColors);
    active.requestDraw();
  }, [vertices, faces, faceColors]);

  useEffect(() => { renderer.current?.frame(bbox); }, [bbox]);
  useEffect(() => { renderer.current?.reset(); }, [resetKey]);
  useEffect(() => { renderer.current?.requestDraw(); }, [mode, single, divider]);
  useEffect(() => { renderer.current?.canvas.setAttribute("aria-label", label); }, [label]);

  useEffect(() => {
    const active = renderer.current;
    if (!active) return;
    const mesh = active.original;
    mesh.visible = false;
    mesh.material.map?.dispose();
    mesh.material.map = null;
    if (!original) {
      latest.current.onReady?.();
      active.requestDraw();
      return;
    }

    mesh.geometry.dispose();
    mesh.geometry = createOriginalGeometry(original);
    const [r, g, b] = original.baseColorFactor;
    mesh.material.color.setRGB(r, g, b, THREE.LinearSRGBColorSpace);
    active.requestDraw();

    let disposed = false;
    const texture = new THREE.TextureLoader().load(original.imageUrl, () => {
      if (disposed) return;
      mesh.material.map = texture;
      mesh.material.needsUpdate = true;
      mesh.visible = true;
      latest.current.onReady?.();
      active.requestDraw();
    }, undefined, () => {
      if (!disposed) latest.current.onError?.();
    });
    // glTF UV (0,0) addresses the top-left texel and the factor is linear.
    texture.flipY = false;
    texture.colorSpace = THREE.SRGBColorSpace;
    texture.wrapS = WRAP[original.wrapS];
    texture.wrapT = WRAP[original.wrapT];
    if (original.magFilter !== undefined) texture.magFilter = FILTER[original.magFilter];
    if (original.minFilter !== undefined) texture.minFilter = FILTER[original.minFilter];
    texture.generateMipmaps = texture.minFilter !== THREE.NearestFilter
      && texture.minFilter !== THREE.LinearFilter;
    return () => {
      disposed = true;
      if (mesh.material.map === texture) mesh.material.map = null;
      texture.dispose();
    };
  }, [original]);

  return <div ref={container} style={{ width: "100%", height: "100%", background: "var(--bg-canvas, #2a2a2a)" }} />;
}
