// Minimal decision-matrix verification for the D2 fill-routing fix.
// Mirrors the logic in src/hooks/usePaintTool.ts after the change.
const MANUAL_SEGMENT_OFFSET = 100_000;

function fillWholeSegment(opts, seg, label) {
  // D2: any existing partition is fillable whole, regardless of share.
  return !opts?.wholeRegion && !!seg && label !== undefined;
}

function route(opts, label, segs) {
  const seg = label !== undefined ? segs.find((s) => s.id === label) : undefined;
  const whole = fillWholeSegment(opts, seg, label);
  return whole ? "segment(整段)" : (opts?.wholeRegion ? "whole-region" : "local(画笔直径!)");
}

const cases = [
  // [name, opts, label, segments, expected]
  ["giant 背景分区 label=0 (share>0.8) 点击 → 必须整段填充 (修复前是画笔)", {}, 0, [{ id: 0, faceCount: 9000 }, { id: 1, faceCount: 1000 }], "segment(整段)"],
  ["单分区模型 (realSegs.length==1) 点击 → 必须整段填充 (修复前是画笔)", {}, 0, [{ id: 0, faceCount: 10000 }], "segment(整段)"],
  ["普通分区点击 → 整段填充", {}, 3, [{ id: 0, faceCount: 5000 }, { id: 3, faceCount: 5000 }], "segment(整段)"],
  ["手动分区 label>=100000 点击 → 整段填充", {}, MANUAL_SEGMENT_OFFSET + 1, [{ id: 0, faceCount: 9000 }, { id: MANUAL_SEGMENT_OFFSET + 1, faceCount: 1000 }], "segment(整段)"],
  ["segments 空 (autoSegment 失败) → 降级画笔但不报错 (REFUTE 安全路径)", {}, 0, [], "local(画笔直径!)"],
  ["wholeRegion (Shift+click) → whole-region 洪泛", { wholeRegion: true }, 0, [{ id: 0, faceCount: 10000 }], "whole-region"],
  ["label undefined (无 hover 无 click label) → local 安全", {}, undefined, [{ id: 0, faceCount: 10000 }], "local(画笔直径!)"],
];

let pass = 0;
for (const [name, opts, label, segs, expected] of cases) {
  const got = route(opts, label, segs);
  const ok = got === expected;
  if (ok) pass++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}\n      got=${got} expected=${expected}`);
}
console.log(`\n${pass}/${cases.length} cases pass`);
process.exit(pass === cases.length ? 0 : 1);
