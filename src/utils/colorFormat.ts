// Shared colour formatting for the swatch-bearing panels (colour panel,
// replace-colour section). One definition — the copy in ColorPanel.tsx was
// the seed; do not re-inline it in new components.
export const toHex = (c: [number, number, number, number] | [number, number, number]) =>
  `#${c[0].toString(16).padStart(2, "0")}${c[1].toString(16).padStart(2, "0")}${c[2].toString(16).padStart(2, "0")}`;
