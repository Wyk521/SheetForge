import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("输出预览使用居中紧凑标签栏并占满剩余高度", async () => {
  const [preview, grid] = await Promise.all([
    read("src/views/PreviewView.vue"),
    read("src/components/MergedPreviewGrid.vue"),
  ]);

  assert.match(preview, /class="sf-tabbar"/);
  assert.doesNotMatch(preview, /sf-page-code|sf-page-title|sf-page-description/);
  assert.doesNotMatch(preview, /张来源表/);
  assert.match(preview, /class="sf-preview-title"/);
  assert.match(preview, /const previewMaxHeight = ref<number>\(\)/);
  assert.match(preview, /content\.getBoundingClientRect\(\)\.bottom/);
  assert.match(preview, /new ResizeObserver\(schedulePreviewHeight\)/);
  assert.match(preview, /:max-height="previewMaxHeight"/);
  assert.doesNotMatch(grid, /maxHeight:\s*430/);
});
