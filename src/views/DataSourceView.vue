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
  store.activePage = 2;
}
</script>

<template>
  <div>
    <div style="display: flex; align-items: flex-end; gap: 12px; margin-bottom: 14px">
      <div>
        <h1 style="font-size: 18px; font-weight: 600; margin: 0">选择数据源</h1>
        <p style="font-size: 11.5px; color: var(--sf-text-muted); margin: 4px 0 0">
          「开始行」是表头从第几行开始；只有多行表头才需要调整「占用行数」
        </p>
      </div>
      <div style="margin-left: auto; display: flex; gap: 8px; align-items: center">
        <el-button type="primary" :disabled="store.busy" @click="store.chooseFolder()">
          选择文件夹
        </el-button>
        <el-button :disabled="store.busy" @click="store.chooseFiles()">选择多个文件</el-button>
        <el-select
          v-model="recentFolder"
          placeholder="最近文件夹…"
          style="width: 150px"
          clearable
          @change="onRecentFolder"
        >
          <el-option v-for="folder in recentFolders" :key="folder" :label="folder" :value="folder" />
        </el-select>
        <el-select
          v-model="recentScheme"
          placeholder="最近方案…"
          style="width: 150px"
          clearable
          @change="onRecentScheme"
        >
          <el-option v-for="scheme in recentSchemes" :key="scheme" :label="scheme" :value="scheme" />
        </el-select>
      </div>
    </div>

    <div style="display: flex; gap: 10px; align-items: center; margin-bottom: 12px">
      <el-input
        v-model="store.sourceSearch"
        placeholder="搜索文件名、Sheet 或路径"
        clearable
        style="flex: 1; min-width: 180px"
      />
      <span style="color: var(--sf-text-muted); font-size: 11px; white-space: nowrap">
        <template v-if="store.sourceFilterActive">
          当前匹配 {{ store.visibleSourceCount }} 张 · 已选 {{ store.visibleEnabledCount }} 张
        </template>
        <template v-else>共 {{ store.visibleSourceCount }} 张 · 已选 {{ store.visibleEnabledCount }} 张</template>
      </span>
      <el-button
        :disabled="!store.hasSources || store.busy || (store.sourceFilterActive && store.visibleSourceCount === 0)"
        @click="store.selectAll(true, store.sourceFilterActive)"
      >
        {{ store.sourceFilterActive ? "全选当前结果" : "全选" }}
      </el-button>
      <el-button
        :disabled="!store.hasSources || store.busy || (store.sourceFilterActive && store.visibleSourceCount === 0)"
        @click="store.selectAll(false, store.sourceFilterActive)"
      >
        {{ store.sourceFilterActive ? "全不选当前结果" : "全不选" }}
      </el-button>
      <el-button :disabled="!store.hasSources || store.busy" @click="store.clearSources()">
        清空
      </el-button>
    </div>

    <el-card shadow="never" style="border-radius: 4px">
      <template v-if="!store.hasSources">
        <div style="padding: 60px 0; text-align: center; color: var(--sf-text-muted)">
          <div style="font-size: 15px; font-weight: 600; color: var(--sf-text); margin-bottom: 6px">
            把 Excel / CSV 拖到这里
          </div>
          <div style="font-size: 12px">也可以选择文件夹，软件会递归查找所有支持的工作簿</div>
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
            <span
              style="cursor: pointer; color: var(--sf-primary); font-size: 14px; width: 14px; text-align: center"
              @click="store.toggleGroup(group.path)"
            >
              {{ group.collapsed ? "›" : "⌄" }}
            </span>
            <span class="sf-row-name">{{ group.fileName }}</span>
            <span class="sf-row-meta">
              {{ store.sourceFilterActive ? "匹配 " : "" }}{{ group.tables.length }} 个数据表 · 已选
              {{ group.tables.filter((item) => item.table.enabled).length }} 个
            </span>
            <div style="margin-left: auto; display: flex; gap: 8px">
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
            <div style="flex: 1; min-width: 0">
              <div class="sf-row-name" style="overflow: hidden; text-overflow: ellipsis; white-space: nowrap">
                {{ item.table.sheet_name }}
              </div>
              <div class="sf-row-meta">
                {{ store.formatNumber(item.table.estimated_rows) }} 行 · {{ item.table.headers.length }}
                列 · 推荐表头第 {{ item.table.suggested_header_row }}
              </div>
            </div>
            <span style="color: var(--sf-text-muted); font-size: 11px">开始行</span>
            <el-input-number
              :model-value="item.table.header_row"
              :min="1"
              :max="100000"
              size="small"
              style="width: 100px"
              :disabled="store.busy"
              @change="(value: number | undefined) => store.reloadTable(item.index, value ?? 1, item.table.header_rows)"
            />
            <span style="color: var(--sf-text-muted); font-size: 11px">占用行数</span>
            <el-input-number
              :model-value="item.table.header_rows"
              :min="1"
              :max="3"
              size="small"
              style="width: 80px"
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
    <div style="font-size: 10px; color: var(--sf-text-muted); margin-top: 8px">{{ store.inputLabel }}</div>
  </div>
</template>
