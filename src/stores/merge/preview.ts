import { ref } from "vue";
import { invoke } from "../../native";
import { ElMessage } from "element-plus";
import {
  type CheckIssue,
  type MergedPreview,
  type PreviewTable,
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
  function clearMergedPreview() {
    mergedPreviewRequest += 1;
    preview.value = null;
    previewTitle.value = "";
  }
  async function showSourcePreview(index: number) {
    const table = sources.value[index];
    if (!table) return;
    const request = ++sourcePreviewRequest;
    sourcePreviewVisible.value = true;
    sourcePreviewLoading.value = true;
    sourcePreview.value = null;
    sourcePreviewTitle.value = `${displayName(table)} · 正在读取…`;
    try {
      const result = await invoke("preview_source", { table, limit: 30 });
      if (request !== sourcePreviewRequest) return;
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
    sourcePreviewVisible.value = false;
    sourcePreviewLoading.value = false;
    sourcePreview.value = null;
    sourcePreviewTitle.value = "";
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
    showSourcePreview,
    closeSourcePreview,
    showMergedPreview,
  };
}
