#!/usr/bin/env node
/**
 * probe.mjs — headless version of the app's "Auto self-test".
 *
 * Runs the SAME pure-math core (core.mjs) the browser uses and prints the
 * expected-vs-actual table to stdout. No browser, no three.js, no DOM.
 *
 *   node probe.mjs                      # default model (dome), 5 seeds
 *   node probe.mjs --model=knot         # dome | box | knot
 *   node probe.mjs --seeds=3
 *   node probe.mjs --size=1440x900      # viewport CSS px used by CameraFit
 *   node probe.mjs --json               # machine-readable report on stdout
 *
 * Exit code: 0 when every strategy behaves as expected, 1 otherwise.
 */

import { runSelfTest, CANONICAL_VIEWPORT, buildModel, fitCamera } from "./core.mjs";

function parseArgs(argv) {
  const out = { model: "dome", seeds: 5, viewport: { ...CANONICAL_VIEWPORT }, json: false };
  // The default model MATCHES the page's default (dome), so `node probe.mjs`
  // reproduces the page's "Auto self-test" table. --model=box refuses S1/S3/S6
  // even more loudly (huge faces ⇒ hundreds of px of error).
  for (const arg of argv.slice(2)) {
    const [key, value] = arg.replace(/^--/, "").split("=");
    if (key === "json") out.json = true;
    else if (key === "model") out.model = value;
    else if (key === "seeds") out.seeds = Math.max(1, Number(value) || 5);
    else if (key === "size") {
      const m = /^(\d+)x(\d+)$/.exec(value || "");
      if (m) out.viewport = { width: Number(m[1]), height: Number(m[2]) };
    } else if (key === "help" || key === "h") {
      out.help = true;
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }
  return out;
}

let args;
try {
  args = parseArgs(process.argv);
} catch (err) {
  console.error(`probe: ${err.message}`);
  process.exit(2);
}

if (args.help) {
  console.log("usage: node probe.mjs [--model=box|dome|knot] [--seeds=N] [--size=WxH] [--json]");
  process.exit(0);
}

const model = buildModel(args.model);
const cam = fitCamera(model.bbox, args.viewport);
const report = runSelfTest({
  modelId: args.model,
  viewport: args.viewport,
  seedCount: args.seeds,
  mesh: model,
});

if (args.json) {
  console.log(JSON.stringify(report, null, 2));
} else {
  console.log(report.text);
  console.log("");
  const perScenario = report.scenarios
    .map((sc) => `${sc.id}:${sc.allOk ? "PASS" : "FAIL"}(${sc.aggregate.filter((a) => a.ok).length}/${sc.aggregate.length})`)
    .join(" ");
  console.log(
    ` [probe] model=${args.model} faces=${model.faceCount} seeds=${report.seeds.length} ` +
      `viewport=${args.viewport.width}x${args.viewport.height} zoom=${cam.zoom.toFixed(3)} ` +
      `worldPerPx=${cam.worldPerPx.toFixed(5)} s5Tolerance=${report.sizes.s5TolerancePx.toFixed(2)}px ` +
      `scenarios=[${perScenario}] verdict=${report.expectedVsActual.allOk ? "PASS" : "FAIL"}`
  );
}
process.exit(report.expectedVsActual.allOk ? 0 : 1);
