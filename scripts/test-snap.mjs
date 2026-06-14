import assert from "node:assert/strict";
import { createServer } from "vite";

const server = await createServer({
  appType: "custom",
  logLevel: "error",
  server: { middlewareMode: true },
});

try {
  const {
    applySnapGap,
    computeSnapTarget,
    detectEdgeSnapAction,
    isSnapAction,
    snapShortcutAction,
    snapSharedEdges,
  } = await server.ssrLoadModule("/src/lib/snap.ts");

  const landscape = { x: 10, y: 20, w: 900, h: 600 };
  assert.deepEqual(computeSnapTarget("firstThird", landscape), {
    x: 10,
    y: 20,
    w: 300,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("centerThird", landscape), {
    x: 310,
    y: 20,
    w: 300,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("lastThird", landscape), {
    x: 610,
    y: 20,
    w: 300,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("firstTwoThirds", landscape), {
    x: 10,
    y: 20,
    w: 600,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("centerTwoThirds", landscape), {
    x: 160,
    y: 20,
    w: 600,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("lastTwoThirds", landscape), {
    x: 310,
    y: 20,
    w: 600,
    h: 600,
  });

  const odd = { x: 0, y: 0, w: 901, h: 600 };
  assert.deepEqual(computeSnapTarget("firstThird", odd), {
    x: 0,
    y: 0,
    w: 300,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("centerThird", odd), {
    x: 300,
    y: 0,
    w: 300,
    h: 600,
  });
  assert.deepEqual(computeSnapTarget("lastThird", odd), {
    x: 600,
    y: 0,
    w: 301,
    h: 600,
  });

  const portrait = { x: -40, y: 100, w: 600, h: 900 };
  assert.deepEqual(computeSnapTarget("firstThird", portrait), {
    x: -40,
    y: 100,
    w: 600,
    h: 300,
  });
  assert.deepEqual(computeSnapTarget("centerThird", portrait), {
    x: -40,
    y: 400,
    w: 600,
    h: 300,
  });
  assert.deepEqual(computeSnapTarget("lastThird", portrait), {
    x: -40,
    y: 700,
    w: 600,
    h: 300,
  });
  assert.deepEqual(computeSnapTarget("centerTwoThirds", portrait), {
    x: -40,
    y: 250,
    w: 600,
    h: 600,
  });

  assert.equal(isSnapAction("lastTwoThirds"), true);
  assert.equal(isSnapAction("restore"), false);
  assert.equal(
    snapShortcutAction({ key: "ArrowRight", ctrlKey: true, altKey: true }),
    "rightHalf",
  );
  assert.equal(
    snapShortcutAction({ key: "u", ctrlKey: true, altKey: true }),
    "topLeft",
  );
  assert.equal(
    snapShortcutAction({ key: "0", ctrlKey: true, altKey: true }),
    "restore",
  );
  assert.equal(
    snapShortcutAction({ key: "ArrowRight", ctrlKey: false, altKey: true }),
    null,
  );
  assert.equal(
    snapShortcutAction({
      key: "ArrowRight",
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    }),
    null,
  );
  assert.equal(
    snapShortcutAction({ key: "x", ctrlKey: true, altKey: true }),
    null,
  );
  assert.deepEqual(snapSharedEdges("centerThird", landscape), [
    "left",
    "right",
  ]);
  assert.deepEqual(
    applySnapGap({ x: 0, y: 0, w: 300, h: 200 }, 12, ["right"]),
    { x: 12, y: 12, w: 282, h: 176 },
  );

  assert.equal(
    detectEdgeSnapAction(500, 790, { w: 900, h: 800 }),
    "centerThird",
  );
  assert.equal(
    detectEdgeSnapAction(5, 420, { w: 600, h: 900 }, { coarse: true }),
    "centerThird",
  );

  console.log("test-snap: PASS");
} finally {
  await server.close();
}
