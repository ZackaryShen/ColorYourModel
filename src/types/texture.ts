import type { MeshData } from "./mesh";

type TextureWrap = "repeat" | "clamp" | "mirror";
type TextureMagFilter = 9728 | 9729;
type TextureMinFilter = TextureMagFilter | 9984 | 9985 | 9986 | 9987;

export interface OriginalTexturePreview {
  vertices: number[];
  faces: number[];
  cornerUvs: number[];
  imageUrl: string;
  wrapS: TextureWrap;
  wrapT: TextureWrap;
  magFilter?: TextureMagFilter;
  minFilter?: TextureMinFilter;
  baseColorFactor: number[];
}

export interface TexturePreview {
  original: OriginalTexturePreview;
  converted: MeshData;
  palette: number[][];
  warning?: "precision" | "budget" | "level-limit";
}
