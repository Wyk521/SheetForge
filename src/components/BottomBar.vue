<script setup lang="ts">
import { computed } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { useMergeStore } from "../stores/merge";
import type { OutputDestination } from "../types";

const store = useMergeStore();

const statusText = computed(() => {
  if (store.progressLabel) return store.progressLabel;
  if (store.warnings.length > 0) {
    return `${store.warnings.length} 个文件或工作表未能读取，可在检查报告中查看`;
  }
  return `当前方式：${store.MODE_LABELS[store.options.mode] ?? store.options.mode}`;
});

const statusKind = computed(() => (store.warnings.length > 0 ? 2 : 0));
const statusReveal = computed(
  () => store.outputDestination === "xlsx" && store.progressLabel === "合并完成"
);
const selectedProfile = computed(
  () => store.databaseProfiles[store.databaseImport.profile_name]
);
const startLabel = computed(() => {
  if (store.busy) return store.outputDestination === "xlsx" ? "正在合并…" : "正在导入…";
  return store.outputDestination === "xlsx" ? "开始合并" : "导入数据库";
});

async function chooseOutput() {
  if (store.busy) return;
  const path = await save({
    title: "选择输出文件",
    defaultPath: store.outputPath || "合并结果.xlsx",
    filters: [{ name: "Excel 工作簿", extensions: ["xlsx"] }],
  });
  if (path) store.outputPath = String(path);
}

function changeDestination(value: string | number | boolean | undefined) {
  store.outputDestination = String(value) as OutputDestination;
  if (store.outputDestination === "postgres") {
    store.openDatabaseTarget();
  }
}
</script>

<template>
  <div class="sf-bottombar">
    <div class="sf-output-row">
      <div class="sf-destination-switch">
        <span class="sf-output-label">输出到</span>
        <el-radio-group
          :model-value="store.outputDestination"
          size="large"
          :disabled="store.busy"
          @change="changeDestination"
        >
          <el-radio-button value="xlsx">Excel 文件</el-radio-button>
          <el-radio-button value="postgres">PostgreSQL</el-radio-button>
        </el-radio-group>
      </div>

      <template v-if="store.outputDestination === 'xlsx'">
        <el-input
          v-model="store.outputPath"
          :disabled="store.busy"
          placeholder="请选择输出文件"
          class="sf-output-input"
        />
        <el-button :disabled="store.busy" @click="chooseOutput">浏览…</el-button>
      </template>
      <template v-else>
        <div class="sf-db-target" :class="{ incomplete: !store.databaseReady }">
          <div>
            <small>{{ selectedProfile ? `${selectedProfile.user}@${selectedProfile.host}:${selectedProfile.port}/${selectedProfile.database}` : "尚未选择数据库连接" }}</small>
            <b>{{ store.databaseImport.schema || "—" }}.{{ store.databaseImport.table || "请填写目标表" }}</b>
          </div>
          <el-tag v-if="store.databaseImport.if_exists !== 'abort'" size="small" type="warning">
            {{ { append: "追加", truncate: "清空", replace: "重建" }[store.databaseImport.if_exists] }}
          </el-tag>
        </div>
        <el-button :disabled="store.busy" @click="store.openDatabaseTarget()">本次导入目标…</el-button>
      </template>

      <div class="sf-metric">
        <small>预计数据行</small>
        <b class="tabular">{{ store.formatNumber(store.rowsMetric) }}</b>
      </div>
      <div class="sf-metric">
        <small>输出列</small>
        <b>{{ store.planHeaders.length }}</b>
      </div>
      <div v-if="store.outputDestination === 'xlsx'" class="sf-metric">
        <small>预计 Sheet</small>
        <b>{{ store.sheetsMetric }}</b>
      </div>
      <el-button v-if="store.busy" @click="store.cancelMerge()">取消</el-button>
      <el-button type="primary" size="large" :disabled="!store.canStart" @click="store.startMerge()">
        {{ startLabel }}
      </el-button>
    </div>
    <div class="sf-progress-row">
      <el-progress
        :percentage="Math.round(store.progress * 100)"
        :indeterminate="store.busy && store.progress <= 0"
        :show-text="false"
        :stroke-width="8"
        style="flex: 1"
      />
      <span class="tabular sf-progress-number">{{ Math.round(store.progress * 100) }}%</span>
      <div class="sf-status" :class="{ error: statusKind === 2 }">
        <span class="dot"></span>
        <span>{{ statusText }}</span>
      </div>
      <el-button v-if="statusReveal" size="small" @click="store.revealOutput()">在文件夹中显示</el-button>
    </div>
  </div>
</template>

<style scoped>
.sf-output-row, .sf-progress-row { display: flex; align-items: center; gap: 10px; }
.sf-destination-switch { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.sf-output-label { font-weight: 700; font-size: 12px; }
.sf-output-input { flex: 1; min-width: 180px; }
.sf-db-target { flex: 1; min-width: 220px; border: 1px solid var(--sf-border); background: #fff; border-radius: 9px; padding: 6px 10px; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
.sf-db-target.incomplete { border-color: var(--el-color-warning-light-5); background: var(--el-color-warning-light-9); }
.sf-db-target small, .sf-db-target b { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.sf-db-target small { color: var(--sf-text-muted); font-size: 9.5px; }
.sf-db-target b { margin-top: 2px; font-size: 12px; }
.sf-progress-number { font-size: 11px; color: var(--sf-text-muted); width: 42px; text-align: right; }
</style>
