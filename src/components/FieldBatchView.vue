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
      !(g.uniformTarget ?? "").toLowerCase().includes(search) &&
      !g.tables.some((source) => source.name.toLowerCase().includes(search))
    ) {
      return false;
    }
    return true;
  });
});
</script>

<template>
  <div class="sf-field-batch">
    <div class="sf-field-batch-toolbar">
      <el-input v-model="store.mappingSearch" class="sf-field-batch-search" placeholder="搜索字段" clearable />
      <el-checkbox v-model="store.onlyMultiField">只看多表字段</el-checkbox>
      <span class="sf-field-batch-hint">
        按来源字段分组，改动会同步到所有启用表；展开后点击“预览源表”即可在当前工作区查看
      </span>
    </div>
    <div v-if="filteredGroups.length > 0" class="sf-field-table-wrap">
      <el-table class="sf-field-table" :data="filteredGroups" size="small" border>
      <el-table-column type="expand">
        <template #default="{ row }">
          <div class="sf-source-preview-list">
            <div class="sf-source-preview-count">
              涉及 {{ row.count }} 张启用表：
            </div>
            <div
              v-for="source in row.tables"
              :key="source.index"
              class="sf-source-preview-row"
            >
              <span class="sf-source-preview-name">
                {{ source.name }}
              </span>
              <el-button
                class="sf-source-preview-action"
                size="small"
                type="primary"
                plain
                :disabled="store.busy"
                title="预览源表"
                @click.stop="store.showSourcePreview(source.index)"
              >
                预览
              </el-button>
            </div>
          </div>
        </template>
      </el-table-column>
      <el-table-column label="源字段" min-width="140" show-overflow-tooltip>
        <template #default="{ row }">
          <span class="sf-field-name">{{ row.key }}</span>
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
              :aria-label="`上移输出字段：${row.uniformTarget || row.key}`"
              :disabled="!row.uniformTarget || outputIndex(row.uniformTarget) <= 0"
              @click="store.moveOutputColumnByName(row.uniformTarget ?? '', -1)"
            >
              ↑
            </el-button>
            <el-button
              size="small"
              text
              :aria-label="`下移输出字段：${row.uniformTarget || row.key}`"
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
          <div v-if="row.uniformEnabled === null" class="sf-partial-state">
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
    </div>
    <div v-else class="sf-field-batch-empty">
      {{ store.fieldGroups.length === 0 ? "暂无启用表字段" : "没有匹配的字段" }}
    </div>
  </div>
</template>

<style scoped>
.sf-field-batch {
  min-width: 0;
}

.sf-field-batch-toolbar {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  min-width: 0;
  margin-bottom: var(--space-sm);
}

.sf-field-batch-search {
  flex: 0 0 12.5rem;
}

.sf-field-batch-hint {
  flex: 1;
  min-width: 0;
  color: var(--sf-text-muted);
  font-size: var(--text-2xs);
  line-height: 1.4;
  text-align: right;
}

.sf-field-table-wrap {
  width: 100%;
  min-width: 0;
  overflow-x: auto;
}

.sf-source-preview-list {
  min-width: 0;
  padding: var(--space-xs) var(--space-lg);
}

.sf-source-preview-count {
  margin-bottom: var(--space-2xs);
  color: var(--sf-text-muted);
  font-size: var(--text-2xs);
}

.sf-source-preview-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--space-xs);
  min-height: 2rem;
  min-width: 0;
}

.sf-source-preview-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf-source-preview-action {
  flex-shrink: 0;
  white-space: nowrap;
}

.sf-field-name {
  font-weight: 700;
}

.sf-partial-state {
  color: var(--sf-text-muted);
  font-family: var(--font-mono);
  font-size: var(--text-2xs);
}

.sf-field-batch-empty {
  padding: var(--space-xl) 0;
  color: var(--sf-text-muted);
  font-size: var(--text-xs);
  text-align: center;
}

@media (max-width: 1080px) {
  .sf-field-batch-toolbar {
    flex-wrap: wrap;
  }

  .sf-field-batch-hint {
    flex-basis: 100%;
    text-align: left;
  }
}
</style>
