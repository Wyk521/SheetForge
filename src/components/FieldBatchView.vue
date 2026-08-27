<script setup lang="ts">
import { computed } from "vue";
import { headerKey, useMergeStore } from "../stores/merge";
import type { TransformOp } from "../types";

const store = useMergeStore();

function outputIndex(name: string): number {
  return store.planHeaders.findIndex((header) => headerKey(header) === headerKey(name));
}

const TRANSFORMS: { value: TransformOp; label: string }[] = [
  { value: "None", label: "不处理" },
  { value: "Trim", label: "去空格" },
  { value: "Uppercase", label: "转大写" },
  { value: "Lowercase", label: "转小写" },
];

const filteredGroups = computed(() => {
  const search = store.mappingSearch.trim().toLowerCase();
  return store.fieldGroups.filter((g) => {
    if (store.onlyMultiField && g.count < 2) return false;
    if (
      search &&
      !g.key.toLowerCase().includes(search) &&
      !(g.uniformTarget ?? "").toLowerCase().includes(search)
    ) {
      return false;
    }
    return true;
  });
});
</script>

<template>
  <div>
    <div style="display: flex; gap: 8px; align-items: center; margin-bottom: 10px">
      <el-input v-model="store.mappingSearch" placeholder="搜索字段" style="width: 200px" clearable />
      <el-checkbox v-model="store.onlyMultiField">只看多表字段</el-checkbox>
      <span style="flex: 1; font-size: 11px; color: var(--sf-text-muted); text-align: right">
        按来源表头分组，改动会同步到所有启用表中的同名字段
      </span>
    </div>
    <el-table v-if="filteredGroups.length > 0" :data="filteredGroups" size="small" border>
      <el-table-column type="expand">
        <template #default="{ row }">
          <div style="padding: 6px 24px">
            <div style="font-size: 11px; color: var(--sf-text-muted); margin-bottom: 4px">
              涉及 {{ row.count }} 张启用表：
            </div>
            <div v-for="(name, i) in row.tables" :key="i" style="font-size: 12px; line-height: 1.8">
              {{ name }}
            </div>
          </div>
        </template>
      </el-table-column>
      <el-table-column label="源字段" min-width="140" show-overflow-tooltip>
        <template #default="{ row }">
          <span style="font-weight: 600">{{ row.key }}</span>
        </template>
      </el-table-column>
      <el-table-column label="涉及表数" width="100">
        <template #default="{ row }">
          <el-tag size="small" type="info">{{ row.count }} 张</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="顺序" width="82" align="center">
        <template #default="{ row }">
          <el-button-group>
            <el-button
              size="small"
              text
              :disabled="!row.uniformTarget || outputIndex(row.uniformTarget) <= 0"
              @click="store.moveOutputColumnByName(row.uniformTarget ?? '', -1)"
            >
              ↑
            </el-button>
            <el-button
              size="small"
              text
              :disabled="!row.uniformTarget || outputIndex(row.uniformTarget) < 0 || outputIndex(row.uniformTarget) >= store.planHeaders.length - 1"
              @click="store.moveOutputColumnByName(row.uniformTarget ?? '', 1)"
            >
              ↓
            </el-button>
          </el-button-group>
        </template>
      </el-table-column>
      <el-table-column label="目标字段" min-width="160">
        <template #default="{ row }">
          <el-input
            :model-value="row.uniformTarget ?? ''"
            size="small"
            placeholder="多个值"
            @update:model-value="(v: string) => store.setFieldTarget(row.key, v)"
          />
        </template>
      </el-table-column>
      <el-table-column label="启用" width="90">
        <template #default="{ row }">
          <el-switch
            :model-value="row.uniformEnabled ?? false"
            size="small"
            @change="(v: boolean | string | number) => store.setFieldEnabled(row.key, Boolean(v))"
          />
          <div v-if="row.uniformEnabled === null" style="font-size: 10.5px; color: var(--sf-text-muted)">
            部分启用
          </div>
        </template>
      </el-table-column>
      <el-table-column label="清洗" width="120">
        <template #default="{ row }">
          <el-select
            :model-value="row.uniformTransform ?? ''"
            size="small"
            placeholder="多值"
            @change="(v: string | number | boolean | undefined) => store.setFieldTransform(row.key, String(v) as TransformOp)"
          >
            <el-option v-for="op in TRANSFORMS" :key="op.value" :label="op.label" :value="op.value" />
          </el-select>
        </template>
      </el-table-column>
      <el-table-column label="" width="70" align="center">
        <template #default="{ row }">
          <el-button size="small" text type="danger" @click="store.resetField(row.key)">恢复</el-button>
        </template>
      </el-table-column>
    </el-table>
    <div v-else style="padding: 30px 0; text-align: center; color: var(--sf-text-muted); font-size: 12px">
      {{ store.fieldGroups.length === 0 ? "暂无启用表字段" : "没有匹配的字段" }}
    </div>
  </div>
</template>
