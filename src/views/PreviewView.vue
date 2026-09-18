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
  <div>
    <div style="display: flex; align-items: center; margin-bottom: 12px">
      <h1 style="font-size: 17px; font-weight: 600; margin: 0">输出预览与检查</h1>
      <el-radio-group :model-value="subTab" style="margin-left: auto" @change="onSubTabChange">
        <el-radio-button :value="0">结果预览</el-radio-button>
        <el-radio-button :value="1">检查报告</el-radio-button>
      </el-radio-group>
    </div>

    <template v-if="subTab === 0">
      <div style="display: flex; align-items: center; gap: 10px; margin-bottom: 10px">
        <span style="font-size: 11px; color: var(--sf-text-muted)">
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
        <el-button style="margin-left: auto" @click="store.showMergedPreview()">刷新预览</el-button>
      </div>
      <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">{{ store.previewTitle }}</div>
      <MergedPreviewGrid v-if="store.preview" :preview="store.preview" />
      <div v-else style="color: var(--sf-text-muted); font-size: 12px; padding: 40px 0; text-align: center">
        点击「刷新预览」生成合并结果预览
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
