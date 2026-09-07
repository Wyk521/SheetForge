import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const appPath = new URL("../src/App.vue", import.meta.url);

test("page navigation saves and restores the content scroll position", async () => {
  const source = await readFile(appPath, "utf8");

  assert.match(source, /const appContent = ref<HTMLElement \| null>\(null\)/);
  assert.match(source, /const pageScrollTops = new Map<number, number>\(\)/);
  assert.match(source, /pageScrollTops\.set\(previousPage, appContent\.value\.scrollTop\)/);
  assert.match(source, /appContent\.value\.scrollTop = pageScrollTops\.get\(page\) \?\? 0/);
  assert.match(source, /\{ flush: "sync" \}/);
  assert.match(source, /<div ref="appContent" class="app-content">/);
});
