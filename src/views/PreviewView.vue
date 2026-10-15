<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { ElMessage } from "element-plus";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "../native";
import { useMergeStore } from "../stores/merge";
import MergedPreviewGrid from "../components/MergedPreviewGrid.vue";

const store = useMergeStore();
const subTab = ref(0); // 0 合并结果预览 / 1 检查报告
const previewGridHost = ref<HTMLElement | null>(null);
const previewMaxHeight = ref<number>();
let contentResizeObserver: ResizeObserver | undefined;

const errorCount = computed(() => store.reportIssues.filter((i) => i.level === "Error").length);
const warnCount = computed(() => store.reportIssues.filter((i) => i.level === "Warning").length);
const checkSummary = computed(() => {
  if (store.reportIssues.length === 0) {
    return store.checkRan ? "检查完成：未发现问题" : "尚未检查";
  }
  return `检查完成：${errorCount.value} 个错误，${warnCount.value} 个提醒`;
});

function levelTag(level: string) {
  if (level === "Error") return "danger";
  if (level === "Warning") return "warning";
  return "info";
}

function levelLabel(level: string) {
  if (level === "Error") return "错误";
  if (level === "Warning") return "提醒";
  return "信息";
}

function onSubTabChange(index: number) {
  subTab.value = index;
  if (index === 0) void store.showMergedPreview();
  if (index === 1) void store.runPreflight(false);
}

function updatePreviewHeight() {
  const host = previewGridHost.value;
  const content = host?.closest(".app-content") as HTMLElement | null;
  const page = host?.closest(".sf-page") as HTMLElement | null;
  if (!host || !content || !page || !host.offsetParent) return;

  const contentBottom = content.getBoundingClientRect().bottom;
  const gridTop = host.getBoundingClientRect().top;
  const pageBottomPadding = Number.parseFloat(getComputedStyle(page).paddingBottom) || 0;
  const availableHeight = Math.floor(contentBottom - gridTop - pageBottomPadding);
  if (availableHeight > 0) previewMaxHeight.value = Math.max(240, availableHeight);
}

function schedulePreviewHeight() {
  void nextTick(updatePreviewHeight);
}

watch(
  () => [
    store.activePage,
    subTab.value,
    store.preview?.groups.length ?? 0,
    store.preview?.headers.length ?? 0,
  ],
  schedulePreviewHeight,
  { flush: "post" },
);

onMounted(() => {
  window.addEventListener("resize", schedulePreviewHeight);
  const content = previewGridHost.value?.closest(".app-content");
  if (content) {
    contentResizeObserver = new ResizeObserver(schedulePreviewHeight);
    contentResizeObserver.observe(content);
  }
  schedulePreviewHeight();
});

onUnmounted(() => {
  window.removeEventListener("resize", schedulePreviewHeight);
  contentResizeObserver?.disconnect();
});

async function exportReport() {
  const text = store.reportIssues
    .map((issue) => `[${issue.level}] ${issue.title}\n${issue.detail}`)
    .join("\n\n");
  try {
    const path = await save({
      title: "导出检查报告",
      defaultPath: "合并检查报告.txt",
      filters: [{ name: "文本报告", extensions: ["txt"] }],
    });
    if (!path) return;
    await invoke("save_text_file", { path: String(path), content: text });
    ElMessage.success("报告已导出");
  } catch (error) {
    ElMessage.error(`导出检查报告失败：${error}`);
  }
}
</script>

<template>
  <div class="sf-page sf-page-preview">
    <div class="sf-tabbar">
      <el-radio-group class="sf-preview-tabs" size="small" :model-value="subTab" aria-label="输出视图" @change="onSubTabChange">
        <el-radio-button :value="0">结果预览</el-radio-button>
        <el-radio-button :value="1">检查报告</el-radio-button>
      </el-radio-group>
    </div>

    <template v-if="subTab === 0">
      <div class="sf-preview-toolbar">
        <span
          class="sf-preview-title"
          title="每张表显示前 5 行"
        >
          {{ store.previewTitle }}
        </span>
        <el-button class="sf-toolbar-end" size="small" @click="store.showMergedPreview()">刷新预览</el-button>
      </div>
      <div ref="previewGridHost" class="sf-preview-grid-host">
        <MergedPreviewGrid
          v-if="store.preview"
          :preview="store.preview"
          :max-height="previewMaxHeight"
        />
        <div v-else class="sf-preview-empty">
          点击「刷新预览」生成合并结果预览
        </div>
      </div>
    </template>

    <template v-else>
      <div class="sf-check-toolbar">
        <span class="sf-check-summary" :class="{ 'is-clear': store.checkRan && store.reportIssues.length === 0 }">
          {{ checkSummary }}
        </span>
        <div class="sf-toolbar-actions">
          <el-button @click="store.runPreflight(false)">重新检查</el-button>
          <el-button @click="exportReport()">导出报告</el-button>
        </div>
      </div>
      <div class="sf-issue-list">
        <el-card v-for="(issue, index) in store.reportIssues" :key="index" class="sf-issue-card" shadow="never">
          <div class="sf-issue-row">
            <el-tag :type="levelTag(issue.level) as any" size="small">{{ levelLabel(issue.level) }}</el-tag>
            <div>
              <div class="sf-issue-title">{{ issue.title }}</div>
              <div class="sf-issue-detail">{{ issue.detail }}</div>
            </div>
          </div>
        </el-card>
        <div v-if="store.reportIssues.length === 0 && store.checkRan" class="sf-check-empty">
          未发现问题
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.sf-page-preview {
  padding-top: var(--space-sm);
  padding-bottom: var(--space-sm);
}

.sf-preview-tabs {
  flex-shrink: 0;
}

.sf-preview-toolbar,
.sf-check-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  min-width: 0;
}

.sf-preview-toolbar {
  min-height: 2rem;
  margin-bottom: var(--space-xs);
}

.sf-check-toolbar {
  margin-bottom: var(--space-sm);
}

.sf-toolbar-end,
.sf-toolbar-actions {
  margin-inline-start: auto;
}

.sf-toolbar-actions {
  display: flex;
  gap: var(--space-xs);
}

.sf-preview-title,
.sf-check-summary,
.sf-issue-title {
  font-weight: 700;
  font-size: var(--text-sm);
}

.sf-preview-title {
  overflow: hidden;
  min-width: 0;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf-preview-grid-host {
  min-width: 0;
}

.sf-preview-empty,
.sf-check-empty {
  padding: var(--space-2xl) 0;
  color: var(--sf-text-muted);
  font-size: var(--text-sm);
  text-align: center;
}

.sf-check-summary.is-clear {
  color: var(--color-success);
}

.sf-issue-list {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}

.sf-issue-row {
  display: flex;
  align-items: flex-start;
  gap: var(--space-sm);
}

.sf-issue-detail {
  margin-top: var(--space-3xs);
  color: var(--sf-text-secondary);
  font-size: var(--text-xs);
  line-height: 1.55;
}
</style>
