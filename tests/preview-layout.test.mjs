import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("输出预览压缩页头并使用全部剩余可视高度", async () => {
  const [preview, grid] = await Promise.all([
    read("src/views/PreviewView.vue"),
    read("src/components/MergedPreviewGrid.vue"),
  ]);

  assert.doesNotMatch(preview, /class="sf-page-description"/);
  assert.match(preview, /class="sf-preview-title"/);
  assert.match(preview, /const previewMaxHeight = ref<number>\(\)/);
  assert.match(preview, /content\.getBoundingClientRect\(\)\.bottom/);
  assert.match(preview, /new ResizeObserver\(schedulePreviewHeight\)/);
  assert.match(preview, /:max-height="previewMaxHeight"/);
  assert.doesNotMatch(grid, /maxHeight:\s*430/);
});
