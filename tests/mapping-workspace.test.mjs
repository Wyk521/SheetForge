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
  assert.match(fieldBatch, /预览源表/);
  assert.match(fieldBatch, /@click\.stop/);
  assert.doesNotMatch(dataSource, /activePage\s*=\s*2/);
  assert.match(dataSource, /await store\.showSourcePreview\(index\)/);
  assert.match(app, /<SourcePreviewDialog\s*\/>/);
  assert.match(dialog, /store\.sourcePreviewVisible/);
});

test("输出预览按来源表分组，并为每个来源单独抽样", async () => {
  const [types, store, inspect, commands, preview, grid] = await Promise.all([
    read("src/types.ts"),
    read("src/stores/merge.ts"),
    read("src-tauri/src/inspect.rs"),
    read("src-tauri/src/commands.rs"),
    read("src/views/PreviewView.vue"),
    read("src/components/MergedPreviewGrid.vue"),
  ]);

  assert.match(types, /interface MergedPreviewGroup/);
  assert.match(types, /source_file: string/);
  assert.match(types, /source_sheet: string/);
  assert.match(store, /preview_merged[\s\S]*limit: 5/);
  assert.match(inspect, /struct MergedPreviewGroup/);
  assert.match(inspect, /for \(source_index, table in tables\.iter\(\)\.enumerate\(\)\.filter/);
  assert.match(inspect, /source_file:/);
  assert.match(commands, /Result<MergedPreview, String>/);
  assert.match(preview, /MergedPreviewGrid/);
  assert.match(preview, /每张表前 5 行/);
  assert.match(grid, /label="来源表名"/);
  assert.match(grid, /label="来源 Sheet"/);
  assert.match(grid, /sf-merged-preview-group/);
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
