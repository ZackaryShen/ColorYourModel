import { afterEach } from "vitest";
import { cleanup } from "@testing-library/react";

// Silence React 18 act() warnings under Vitest + jsdom.
(globalThis as unknown as { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

afterEach(() => {
  cleanup();
});
