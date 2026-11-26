export const NATIVE_MESH_FORMATS = ["stl", "obj", "ply", "glb", "gltf"] as const;
export const BLENDER_MESH_FORMATS = ["fbx", "usd", "usda", "usdc", "usdz"] as const;
export const MODEL_FORMATS = ["3mf", ...NATIVE_MESH_FORMATS, ...BLENDER_MESH_FORMATS] as const;

export type ModelFormat = (typeof MODEL_FORMATS)[number];
export type GeometryExportFormat = Exclude<ModelFormat, "3mf" | "obj">;

export function isBlenderFormat(format: ModelFormat): boolean {
  return BLENDER_MESH_FORMATS.some((candidate) => candidate === format);
}
