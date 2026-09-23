<script setup lang="ts">
import { computed, ref } from "vue";
import { useMergeStore } from "../stores/merge";
import type { SourceTable } from "../types";

const store = useMergeStore();

interface GroupItem {
  path: string;
  fileName: string;
  collapsed: boolean;
  enabled: boolean;
  tables: { index: number; table: SourceTable }[];
}

const grouped = computed<GroupItem[]>(() => {
  const filter = store.sourceSearch.trim().toLowerCase();
  const groups = new Map<string, { index: number; table: SourceTable }[]>();
  store.sources.forEach((table, index) => {
    const searchable = `${table.path} ${table.sheet_name}`.toLowerCase();
    if (filter && !searchable.includes(filter)) return;
    const list = groups.get(table.path) ?? [];
    list.push({ index, table });
    groups.set(table.path, list);
  });
  return [...groups.entries()].map(([path, tables]) => ({
    path,
    fileName: path.split(/[\\/]/).pop() ?? path,
    collapsed: store.collapsedGroups.has(path),
    enabled: tables.every((item) => item.table.enabled),
    tables,
  }));
});

const recentFolders = computed(() => store.settings?.recent_folders ?? []);
const recentSchemes = computed(() => store.settings?.recent_schemes ?? []);
const recentFolder = ref("");
const recentScheme = ref("");

async function onRecentFolder(value: string) {
  if (!value) return;
  recentFolder.value = "";
  await store.scanFolder(value);
}

async function onRecentScheme(value: string) {
  if (!value) return;
  recentScheme.value = "";
  await store.openSchemeByPath(value);
}

async function previewSource(index: number) {
  await store.showSourcePreview(index);
}
</script>

<template>
  <div class="sf-page sf-page-source">
    <div class="sf-page-head sf-source-head">
      <div class="sf-page-heading-copy">
        <p class="sf-page-code"><span>01</span> 装载与校准</p>
        <h1 class="sf-page-title">选择数据源</h1>
        <p class="sf-page-description">
          「开始行」是表头从第几行开始；只有多行表头才需要调整「占用行数」
        </p>
      </div>
      <div class="sf-page-actions">
        <el-button type="primary" :disabled="store.busy" @click="store.chooseFolder()">
          选择文件夹
        </el-button>
        <el-button :disabled="store.busy" @click="store.chooseFiles()">选择多个文件</el-button>
        <el-select
          v-model="recentFolder"
          placeholder="最近文件夹…"
          class="sf-recent-select"
          clearable
          :disabled="store.busy"
          @change="onRecentFolder"
        >
          <el-option v-for="folder in recentFolders" :key="folder" :label="folder" :value="folder" />
        </el-select>
        <el-select
          v-model="recentScheme"
          placeholder="最近方案…"
          class="sf-recent-select"
          clearable
          :disabled="store.busy"
          @change="onRecentScheme"
        >
          <el-option v-for="scheme in recentSchemes" :key="scheme" :label="scheme" :value="scheme" />
        </el-select>
      </div>
    </div>

    <div class="sf-workbench-toolbar">
      <el-input
        v-model="store.sourceSearch"
        placeholder="搜索文件名、Sheet 或路径"
        clearable
        class="sf-source-search"
      />
      <span class="sf-toolbar-summary">
        <template v-if="store.sourceFilterActive">
          当前匹配 {{ store.visibleSourceCount }} 张 · 已选 {{ store.visibleEnabledCount }} 张
        </template>
        <template v-else>共 {{ store.visibleSourceCount }} 张 · 已选 {{ store.visibleEnabledCount }} 张</template>
      </span>
      <el-button
        :disabled="!store.hasSources || store.busy || (store.sourceFilterActive && store.visibleSourceCount === 0)"
        @click="store.selectAll(true, store.sourceFilterActive)"
      >
        {{ store.sourceFilterActive ? "仅选当前结果" : "全选" }}
      </el-button>
      <el-button
        :disabled="!store.hasSources || store.busy"
        @click="store.selectAll(false, store.sourceFilterActive)"
      >
        {{ store.sourceFilterActive ? "清空选择" : "全不选" }}
      </el-button>
      <el-button :disabled="!store.hasSources || store.busy" @click="store.clearSources()">
        清空
      </el-button>
    </div>

    <el-card class="sf-source-sheet" shadow="never">
      <template v-if="!store.hasSources">
        <div class="sf-empty-ticket">
          <span class="sf-empty-mark" aria-hidden="true"></span>
          <div class="sf-empty-title">
            把 Excel / CSV 拖到这里
          </div>
          <div class="sf-empty-copy">也可以选择文件夹，软件会递归查找所有支持的工作簿</div>
          <el-button type="primary" :disabled="store.busy" @click="store.chooseFiles()">选择多个文件</el-button>
        </div>
      </template>
      <template v-else>
        <div v-for="group in grouped" :key="group.path">
          <div class="sf-group-row">
            <el-checkbox
              :model-value="group.enabled"
              :disabled="store.busy"
              @change="(value: boolean | string | number) => store.setGroupEnabled(group.path, Boolean(value), store.sourceFilterActive)"
            />
            <button
              type="button"
              class="sf-group-toggle"
              :aria-label="group.collapsed ? '展开工作簿' : '收起工作簿'"
              @click="store.toggleGroup(group.path)"
            >
              {{ group.collapsed ? "›" : "⌄" }}
            </button>
            <span class="sf-row-name">{{ group.fileName }}</span>
            <span class="sf-row-meta">
              {{ store.sourceFilterActive ? "匹配 " : "" }}{{ group.tables.length }} 个数据表 · 已选
              {{ group.tables.filter((item) => item.table.enabled).length }} 个
            </span>
            <div class="sf-row-actions">
              <el-button size="small" :disabled="store.busy" @click="store.applyGroupHeader(group.path)">
                统一表头
              </el-button>
              <el-button size="small" @click="store.toggleGroup(group.path)">
                {{ group.collapsed ? "展开" : "收起" }}
              </el-button>
              <el-button size="small" :disabled="store.busy" @click="store.removeGroup(group.path)">
                移除整簿
              </el-button>
            </div>
          </div>
          <div v-if="!group.collapsed" v-for="item in group.tables" :key="item.index" class="sf-sheet-row">
            <el-checkbox
              :model-value="item.table.enabled"
              :disabled="store.busy"
              @change="(value: boolean | string | number) => store.toggleSourceEnabled(item.index, Boolean(value))"
            />
            <div class="sf-sheet-copy">
              <div class="sf-row-name sf-row-name-truncate">
                {{ item.table.sheet_name }}
              </div>
              <div class="sf-row-meta">
                {{ store.formatNumber(item.table.estimated_rows) }} 行 · {{ item.table.headers.length }}
                列 · 推荐表头第 {{ item.table.suggested_header_row }}
              </div>
            </div>
            <span class="sf-field-label">开始行</span>
            <el-input-number
              :model-value="item.table.header_row"
              :min="1"
              :max="100000"
              size="small"
              class="sf-header-row-input"
              :disabled="store.busy"
              @change="(value: number | undefined) => store.reloadTable(item.index, value ?? 1, item.table.header_rows)"
            />
            <span class="sf-field-label">占用行数</span>
            <el-input-number
              :model-value="item.table.header_rows"
              :min="1"
              :max="3"
              size="small"
              class="sf-header-rows-input"
              :disabled="store.busy"
              @change="(value: number | undefined) => store.reloadTable(item.index, item.table.header_row, value ?? 1)"
            />
            <el-button size="small" :disabled="store.busy" @click="previewSource(item.index)">
              预览
            </el-button>
            <el-button size="small" :disabled="store.busy" @click="store.removeSource(item.index)">
              移除
            </el-button>
          </div>
        </div>
      </template>
    </el-card>
    <div class="sf-source-path">{{ store.inputLabel }}</div>
  </div>
</template>

<style scoped>
.sf-source-head {
  align-items: end;
}

.sf-source-search {
  flex: 1;
  min-width: 180px;
}

.sf-recent-select {
  width: 9.375rem;
}

.sf-sheet-copy {
  flex: 1;
  min-width: 0;
}

.sf-group-row > .sf-row-name {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf-header-row-input {
  width: 6.25rem;
}

.sf-header-rows-input {
  width: 5rem;
}

.sf-row-name-truncate {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf-row-actions {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  margin-inline-start: auto;
}

.sf-field-label {
  color: var(--sf-text-muted);
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  white-space: nowrap;
}

.sf-source-path {
  margin-top: var(--space-xs);
  overflow: hidden;
  color: var(--sf-text-muted);
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
