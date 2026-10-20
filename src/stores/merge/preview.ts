import { ref, watch } from "vue";
import { invoke } from "../../native";
import { ElMessage } from "element-plus";
import {
  type CheckIssue,
  type MergedPreview,
  type PreviewTable,
  type SourceTable,
} from "../../types";
import { displayName } from "./utils";
import type { MergeContext } from "./context";
export function createPreviewState() {
  const checkIssues = ref<CheckIssue[]>([]);
  const checkRan = ref(false);
  const preview = ref<MergedPreview | null>(null);
  const previewTitle = ref("");
  const sourcePreview = ref<PreviewTable | null>(null);
  const sourcePreviewTitle = ref("");
  const sourcePreviewVisible = ref(false);
  const sourcePreviewLoading = ref(false);
  return {
    checkIssues,
    checkRan,
    preview,
    previewTitle,
    sourcePreview,
    sourcePreviewTitle,
    sourcePreviewVisible,
    sourcePreviewLoading,
  };
}
export function createPreviewActions(
  context: Pick<
    MergeContext,
    | "checkIssues"
    | "checkRan"
    | "sources"
    | "options"
    | "preview"
    | "previewTitle"
    | "sourcePreview"
    | "sourcePreviewTitle"
    | "sourcePreviewVisible"
    | "sourcePreviewLoading"
  >,
) {
  const {
    checkIssues,
    checkRan,
    sources,
    options,
    preview,
    previewTitle,
    sourcePreview,
    sourcePreviewTitle,
    sourcePreviewVisible,
    sourcePreviewLoading,
  } = context;
  let sourcePreviewRequest = 0;
  let mergedPreviewRequest = 0;
  let previewedSource: SourceTable | null = null;
  const sourceCache = new Map<SourceTable, { key: string; preview: PreviewTable }>();
  const sourceReads = new Map<SourceTable, { key: string; promise: Promise<PreviewTable> }>();
  const previewKey = (table: SourceTable) => JSON.stringify([
    table.path, table.sheet_name, table.kind, table.header_row, table.header_rows, table.headers,
  ]);
  watch(
    () => sources.value.map((table) => [table, previewKey(table)] as const),
    (entries) => {
      const current = new Map(entries);
      for (const [table, entry] of sourceCache) {
        if (current.get(table) !== entry.key) sourceCache.delete(table);
      }
      for (const [table, entry] of sourceReads) {
        if (current.get(table) !== entry.key) sourceReads.delete(table);
      }
      if (previewedSource && !current.has(previewedSource)) closeSourcePreview();
    },
    { flush: "sync" },
  );
  function readSourcePreview(table: SourceTable, key: string, force: boolean) {
    const pending = sourceReads.get(table);
    if (!force && pending?.key === key) return pending.promise;
    const promise = invoke("preview_source", { table, limit: 30 }).then((result) => {
      if (sourceReads.get(table)?.promise === promise && sources.value.includes(table)
        && previewKey(table) === key) {
        sourceCache.delete(table);
        sourceCache.set(table, { key, preview: result });
        // 仅保留最近打开的 16 张表，每张缓存前 30 行，不随时间过期。
        if (sourceCache.size > 16) sourceCache.delete(sourceCache.keys().next().value!);
      }
      return result;
    }).finally(() => {
      if (sourceReads.get(table)?.promise === promise) sourceReads.delete(table);
    });
    sourceReads.set(table, { key, promise });
    return promise;
  }
  function clearMergedPreview() {
    mergedPreviewRequest += 1;
    preview.value = null;
    previewTitle.value = "";
  }
  function invalidateResults() {
    clearMergedPreview();
    checkIssues.value = [];
    checkRan.value = false;
  }
  async function showSourcePreview(index: number, force = false) {
    const table = sources.value[index];
    if (!table) return;
    const request = ++sourcePreviewRequest;
    previewedSource = table;
    sourcePreviewVisible.value = true;
    const key = previewKey(table);
    const cached = sourceCache.get(table);
    if (!force && cached?.key === key) {
      sourceCache.delete(table);
      sourceCache.set(table, cached);
      sourcePreviewLoading.value = false;
      sourcePreview.value = cached.preview;
      sourcePreviewTitle.value = `${displayName(table)} · 前 ${cached.preview.rows.length} 行`;
      return;
    }
    if (force) sourceCache.delete(table);
    sourcePreviewLoading.value = true;
    sourcePreview.value = null;
    sourcePreviewTitle.value = `${displayName(table)} · 正在读取…`;
    try {
      const result = await readSourcePreview(table, key, force);
      if (request !== sourcePreviewRequest) return;
      if (previewKey(table) !== key) {
        closeSourcePreview();
        return;
      }
      sourcePreview.value = result;
      sourcePreviewTitle.value = `${displayName(table)} · 前 ${result.rows.length} 行`;
    } catch (error) {
      if (request !== sourcePreviewRequest) return;
      sourcePreviewVisible.value = false;
      ElMessage.error(`预览失败：${error}`);
    } finally {
      if (request === sourcePreviewRequest) sourcePreviewLoading.value = false;
    }
  }
  function closeSourcePreview() {
    sourcePreviewRequest += 1;
    previewedSource = null;
    sourcePreviewVisible.value = false;
    sourcePreviewLoading.value = false;
    sourcePreview.value = null;
    sourcePreviewTitle.value = "";
  }
  async function refreshSourcePreview() {
    if (!previewedSource) return;
    const index = sources.value.indexOf(previewedSource);
    if (index >= 0) await showSourcePreview(index, true);
  }
  async function showMergedPreview() {
    const request = ++mergedPreviewRequest;
    try {
      const result = await invoke("preview_merged", {
        tables: sources.value,
        options: options.value,
        limit: 5,
      });
      if (request !== mergedPreviewRequest) return;
      preview.value = result;
      const rowCount = result.groups.reduce(
        (total, group) => total + group.rows.length,
        0,
      );
      previewTitle.value = `预览 ${rowCount} 行 · ${result.headers.length} 列 · ${result.groups.length} 张表`;
    } catch (error) {
      if (request !== mergedPreviewRequest) return;
      ElMessage.error(`结果预览失败：${error}`);
    }
  }
  return {
    clearMergedPreview,
    invalidateResults,
    showSourcePreview,
    refreshSourcePreview,
    closeSourcePreview,
    showMergedPreview,
  };
}
