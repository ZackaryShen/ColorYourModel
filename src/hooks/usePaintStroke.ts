import { useCallback, useRef } from 'react';

// Dirty plates retain an open backend stroke while their view is unmounted.
// Keep IDs across view remounts and development module reloads in this renderer.
const session = globalThis as typeof globalThis & { __cymPaintStrokeCounter?: number };

export function usePaintStroke() {
  const activeStrokeIdRef = useRef<number | null>(null);
  const beginStroke = useCallback(() => {
    const strokeId = (session.__cymPaintStrokeCounter ?? 0) + 1;
    session.__cymPaintStrokeCounter = strokeId;
    activeStrokeIdRef.current = strokeId;
    return activeStrokeIdRef.current;
  }, []);
  return { activeStrokeIdRef, beginStroke };
}
