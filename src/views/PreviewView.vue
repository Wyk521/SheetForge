<script setup lang="ts">
import { computed, ref } from "vue";
import { ElMessage } from "element-plus";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useMergeStore } from "../stores/merge";
import MergedPreviewGrid from "../components/MergedPreviewGrid.vue";

const store = useMergeStore();
const subTab = ref(0); // 0 合并结果预览 / 1 检查报告

const errorCount = computed(() => store.checkIssues.filter((i) => i.level === "Error").length);
const warnCount = computed(() => store.checkIssues.filter((i) => i.level === "Warning").length);
const checkSummary = computed(() => {
  if (store.checkIssues.length === 0) {
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

async function exportReport() {
  const text = store.checkIssues
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
    <div class="sf-page-head sf-preview-head">
      <div class="sf-page-heading-copy">
        <p class="sf-page-code"><span>03</span> 核验与交付</p>
        <h1 class="sf-page-title">输出预览与检查</h1>
        <p class="sf-page-description">在写入文件或数据库前，先核对列结构、来源分段和检查报告。</p>
      </div>
      <el-radio-group class="sf-preview-tabs" :model-value="subTab" @change="onSubTabChange">
        <el-radio-button :value="0">结果预览</el-radio-button>
        <el-radio-button :value="1">检查报告</el-radio-button>
      </el-radio-group>
    </div>

    <template v-if="subTab === 0">
      <div class="sf-preview-toolbar">
        <span class="sf-preview-help">
          所有已选数据表连续显示，每张表前 5 行；表头只显示一次，来源切换处用分隔线区分
        </span>
        <el-tag
          v-if="store.options.mode === 'Consolidate' || store.options.mode === 'Join'"
          type="warning"
          size="small"
          effect="plain"
        >
          {{
            store.options.mode === "Consolidate"
              ? "预览未体现按键汇总结果，仅供列结构参考"
              : "预览未体现关联结果，仅供列结构参考"
          }}
        </el-tag>
        <el-tag v-if="store.preview" type="info" size="small" effect="plain">
          {{ store.preview.groups.length }} 张来源表
        </el-tag>
        <el-button class="sf-toolbar-end" @click="store.showMergedPreview()">刷新预览</el-button>
      </div>
      <div class="sf-preview-title">{{ store.previewTitle }}</div>
      <MergedPreviewGrid v-if="store.preview" :preview="store.preview" />
      <div v-else class="sf-preview-empty">
        点击「刷新预览」生成合并结果预览
      </div>
    </template>

    <template v-else>
      <div class="sf-check-toolbar">
        <span class="sf-check-summary" :class="{ 'is-clear': store.checkRan && store.checkIssues.length === 0 }">
          {{ checkSummary }}
        </span>
        <div class="sf-toolbar-actions">
          <el-button @click="store.runPreflight(false)">重新检查</el-button>
          <el-button @click="exportReport()">导出报告</el-button>
        </div>
      </div>
      <div class="sf-issue-list">
        <el-card v-for="(issue, index) in store.checkIssues" :key="index" class="sf-issue-card" shadow="never">
          <div class="sf-issue-row">
            <el-tag :type="levelTag(issue.level) as any" size="small">{{ levelLabel(issue.level) }}</el-tag>
            <div>
              <div class="sf-issue-title">{{ issue.title }}</div>
              <div class="sf-issue-detail">{{ issue.detail }}</div>
            </div>
          </div>
        </el-card>
        <div v-if="store.checkIssues.length === 0 && store.checkRan" class="sf-check-empty">
          未发现问题
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.sf-preview-head {
  align-items: end;
}

.sf-preview-tabs {
  flex-shrink: 0;
}

.sf-preview-toolbar,
.sf-check-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  margin-bottom: var(--space-sm);
  min-width: 0;
}

.sf-preview-help {
  color: var(--sf-text-muted);
  font-size: var(--text-xs);
  line-height: 1.5;
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
  margin-bottom: var(--space-xs);
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
