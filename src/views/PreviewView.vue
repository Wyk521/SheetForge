<script setup lang="ts">
import { computed, ref } from "vue";
import { ElMessage } from "element-plus";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();
const subTab = ref(0); // 0 源数据预览 / 1 结果预览 / 2 检查报告

const rowsData = computed(() => {
  const preview = store.preview;
  if (!preview) return [];
  return preview.rows.map((row, index) => {
    const record: Record<string, string> = { __line: String(index + 1) };
    row.forEach((cell, column) => {
      record[String(column)] = cell;
    });
    return record;
  });
});

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
  if (index === 1) void store.showMergedPreview();
  if (index === 2) void store.runPreflight(false);
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
  <div>
    <div style="display: flex; align-items: center; margin-bottom: 12px">
      <h1 style="font-size: 17px; font-weight: 600; margin: 0">预览与合并前检查</h1>
      <el-radio-group :model-value="subTab" style="margin-left: auto" @change="onSubTabChange">
        <el-radio-button :value="0">源数据预览</el-radio-button>
        <el-radio-button :value="1">结果预览</el-radio-button>
        <el-radio-button :value="2">检查报告</el-radio-button>
      </el-radio-group>
    </div>

    <template v-if="subTab === 0">
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px">
        <el-select
          :model-value="store.selectedMappingTable"
          style="flex: 1"
          @change="(v: number) => store.showSourcePreview(v)"
        >
          <el-option
            v-for="(index) in store.enabledIndices"
            :key="index"
            :label="store.displayName(store.sources[index])"
            :value="index"
          />
        </el-select>
        <el-button :disabled="!store.hasSources || store.busy" @click="store.showSourcePreview(store.selectedMappingTable)">
          刷新预览
        </el-button>
      </div>
      <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">{{ store.previewTitle }}</div>
      <el-table v-if="store.preview" :data="rowsData" size="small" border :max-height="440">
        <el-table-column prop="__line" label="#" width="52" align="right" />
        <el-table-column
          v-for="(header, column) in store.preview.headers"
          :key="column"
          :prop="String(column)"
          :label="header"
          min-width="120"
          show-overflow-tooltip
        />
      </el-table>
      <div v-else style="color: var(--sf-text-muted); font-size: 12px; padding: 40px 0; text-align: center">
        选择一个数据表生成预览
      </div>
    </template>

    <template v-else-if="subTab === 1">
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px">
        <span style="font-size: 11px; color: var(--sf-text-muted)">
          按当前规则显示前 30 行；正式导出仍采用流式处理
        </span>
        <el-button style="margin-left: auto" @click="store.showMergedPreview()">刷新预览</el-button>
      </div>
      <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">{{ store.previewTitle }}</div>
      <el-table v-if="store.preview" :data="rowsData" size="small" border :max-height="440">
        <el-table-column prop="__line" label="#" width="52" align="right" />
        <el-table-column
          v-for="(header, column) in store.preview.headers"
          :key="column"
          :prop="String(column)"
          :label="header"
          min-width="120"
          show-overflow-tooltip
        />
      </el-table>
      <div v-else style="color: var(--sf-text-muted); font-size: 12px; padding: 40px 0; text-align: center">
        点击「结果预览」生成合并结果预览
      </div>
    </template>

    <template v-else>
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px">
        <span :style="{ fontSize: '12px', fontWeight: 600, color: store.checkRan && store.checkIssues.length === 0 ? '#67c23a' : undefined }">
          {{ checkSummary }}
        </span>
        <div style="margin-left: auto; display: flex; gap: 8px">
          <el-button @click="store.runPreflight(false)">重新检查</el-button>
          <el-button @click="exportReport()">导出报告</el-button>
        </div>
      </div>
      <div style="display: flex; flex-direction: column; gap: 8px">
        <el-card v-for="(issue, index) in store.checkIssues" :key="index" shadow="never" style="border-left: 4px solid">
          <div style="display: flex; gap: 10px; align-items: flex-start">
            <el-tag :type="levelTag(issue.level) as any" size="small">{{ levelLabel(issue.level) }}</el-tag>
            <div>
              <div style="font-weight: 600; font-size: 12.5px">{{ issue.title }}</div>
              <div style="color: var(--sf-text-secondary); font-size: 11px; margin-top: 2px">{{ issue.detail }}</div>
            </div>
          </div>
        </el-card>
        <div v-if="store.checkIssues.length === 0 && store.checkRan" style="text-align: center; color: var(--sf-text-muted); padding: 30px 0">
          未发现问题
        </div>
      </div>
    </template>
  </div>
</template>
