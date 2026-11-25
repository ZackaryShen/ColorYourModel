import type { MeshData } from "../types/mesh";

/** View the native response buffers directly; legacy/mock objects are already decoded. */
export function decodeMeshResponse(response: ArrayBuffer | MeshData | null): MeshData | null {
  if (!(response instanceof ArrayBuffer)) return response;
  if (response.byteLength === 0) return null;
  if (response.byteLength < 20) throw new Error("Incomplete mesh response");

  const header = new DataView(response);
  const vertices = header.getUint32(0, true);
  const faces = header.getUint32(4, true);
  const labels = header.getUint32(8, true);
  const colors = header.getUint32(12, true);
  const metadata = header.getUint32(16, true);
  const facesOffset = 20 + vertices * 4;
  const labelsOffset = facesOffset + faces * 4;
  const colorsOffset = labelsOffset + labels * 4;
  const metadataOffset = colorsOffset + colors;
  if (vertices % 3 !== 0 || faces !== labels * 3 || colors !== labels * 4 ||
      metadataOffset + metadata !== response.byteLength) {
    throw new Error("Invalid mesh response sizes");
  }
  const data: MeshData = JSON.parse(new TextDecoder().decode(new Uint8Array(response, metadataOffset, metadata)));
  if (data.faceCount !== labels) throw new Error("Invalid mesh face count");
  data.vertices = new Float32Array(response, 20, vertices);
  data.faces = new Uint32Array(response, facesOffset, faces);
  data.segmentLabels = new Uint32Array(response, labelsOffset, labels);
  data.faceColors = new Uint8Array(response, colorsOffset, colors);
  return data;
}
