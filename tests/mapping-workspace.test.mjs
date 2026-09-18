import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const root = new URL("../", import.meta.url);

async function read(path) {
  return readFile(new URL(path, root), "utf8");
}

test("字段映射工作区不再暴露表级编辑模式", async () => {
  const [rules, store, main, commands, inspect] = await Promise.all([
    read("src/views/MergeRulesView.vue"),
    read("src/stores/merge.ts"),
    read("src-tauri/src/main.rs"),
    read("src-tauri/src/commands.rs"),
    read("src-tauri/src/inspect.rs"),
  ]);

  assert.doesNotMatch(rules, /按表改字段|mappingScope|setMapping\(/);
  assert.doesNotMatch(store, /mappingScope|selectedMappingTable|hideCommonMappings|mismatchOnly/);
  assert.doesNotMatch(main, /get_suggestions/);
  assert.doesNotMatch(commands, /get_suggestions/);
  assert.doesNotMatch(inspect, /mapping_suggestions/);
  assert.match(rules, /<FieldBatchView\s*\/>/);
});

test("源表预览在当前工作区弹出，不再强制跳到输出页", async () => {
  const [fieldBatch, dataSource, app, dialog] = await Promise.all([
    read("src/components/FieldBatchView.vue"),
    read("src/views/DataSourceView.vue"),
    read("src/App.vue"),
    read("src/components/SourcePreviewDialog.vue"),
  ]);

  assert.match(fieldBatch, /store\.showSourcePreview\(source\.index\)/);
  assert.match(fieldBatch, /@click\.stop/);
  assert.doesNotMatch(dataSource, /activePage\s*=\s*2/);
  assert.match(dataSource, /await store\.showSourcePreview\(index\)/);
  assert.match(app, /<SourcePreviewDialog\s*\/>/);
  assert.match(dialog, /store\.sourcePreviewVisible/);
});

test("导航和输出预览不再使用第一二三步文案", async () => {
  const [topBar, preview] = await Promise.all([
    read("src/components/TopBar.vue"),
    read("src/views/PreviewView.vue"),
  ]);

  assert.doesNotMatch(topBar, /01 数据源|02 合并规则|03 预览与检查/);
  assert.match(topBar, /合并工作区/);
  assert.match(topBar, /输出预览/);
  assert.doesNotMatch(preview, /源数据预览/);
  assert.match(preview, /结果预览/);
  assert.match(preview, /检查报告/);
});
