<script setup lang="ts">
import { computed } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

const statusText = computed(() => {
  if (store.busy && store.progressLabel) return store.progressLabel;
  if (store.progressLabel) return store.progressLabel;
  if (store.warnings.length > 0) {
    return `${store.warnings.length} 个文件或工作表未能读取，可在检查报告中查看`;
  }
  return `当前方式：${store.MODE_LABELS[store.options.mode] ?? store.options.mode}`;
});

const statusKind = computed(() => (store.warnings.length > 0 ? 2 : 0));
const statusReveal = computed(() => store.progressLabel === "合并完成");

async function chooseOutput() {
  if (store.busy) return;
  const path = await save({
    title: "选择输出文件",
    defaultPath: store.outputPath || "合并结果.xlsx",
    filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
  });
  if (path) store.outputPath = String(path);
}
</script>

<template>
  <div class="sf-bottombar">
    <div style="display: flex; align-items: center; gap: 12px">
      <span style="font-weight: 600; font-size: 12.5px">输出位置</span>
      <el-input
        v-model="store.outputPath"
        :disabled="store.busy"
        placeholder="请选择输出文件"
        style="flex: 1"
      />
      <el-button :disabled="store.busy" @click="chooseOutput">浏览…</el-button>
      <div class="sf-metric">
        <small>预计数据行</small>
        <b class="tabular">{{ store.formatNumber(store.rowsMetric) }}</b>
      </div>
      <div class="sf-metric">
        <small>输出列</small>
        <b>{{ store.planHeaders.length }}</b>
      </div>
      <div class="sf-metric">
        <small>预计 Sheet</small>
        <b>{{ store.sheetsMetric }}</b>
      </div>
      <el-button v-if="store.busy" @click="store.cancelMerge()">取消</el-button>
      <el-button type="primary" size="large" :disabled="!store.canStart" @click="store.startMerge()">
        {{ store.busy ? "正在处理…" : "开始合并" }}
      </el-button>
    </div>
    <div style="display: flex; align-items: center; gap: 10px">
      <el-progress
        :percentage="Math.round(store.progress * 100)"
        :indeterminate="store.busy && store.progress <= 0"
        :show-text="false"
        :stroke-width="8"
        style="flex: 1"
      />
      <span class="tabular" style="font-size: 11px; color: var(--sf-text-muted); width: 42px; text-align: right">
        {{ Math.round(store.progress * 100) }}%
      </span>
      <div class="sf-status" :class="{ error: statusKind === 2 }">
        <span class="dot"></span>
        <span>{{ statusText }}</span>
      </div>
      <el-button v-if="statusReveal" size="small" @click="store.revealOutput()">在文件夹中显示</el-button>
    </div>
  </div>
</template>
