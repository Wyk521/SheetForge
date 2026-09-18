<script setup lang="ts">
import type { MergedPreview, MergedPreviewGroup } from "../types";

const props = withDefaults(
  defineProps<{
    preview: MergedPreview | null;
    maxHeight?: number | string;
  }>(),
  { maxHeight: 430 }
);

function rowsData(group: MergedPreviewGroup) {
  return group.rows.map((row, index) => {
    const record: Record<string, string> = {
      __line: String(index + 1),
      __source_file: group.source_file,
      __source_sheet: group.source_sheet,
    };
    row.forEach((cell, column) => {
      record[String(column)] = cell;
    });
    return record;
  });
}
</script>

<template>
  <div v-if="preview" class="sf-merged-preview">
    <div v-if="preview.groups.length === 0" class="sf-merged-preview-empty">
      没有已选中的数据表，请先在数据源中勾选要参与合并的表。
    </div>
    <div
      v-for="(group, groupIndex) in preview.groups"
      :key="group.source_index"
      class="sf-merged-preview-group"
      :class="{ alternate: groupIndex % 2 === 1 }"
    >
      <div class="sf-merged-preview-heading">
        <div class="sf-merged-preview-source">
          <span class="sf-merged-preview-label">来源表名</span>
          <span class="sf-merged-preview-file" :title="group.source_file">{{ group.source_file }}</span>
          <span class="sf-merged-preview-label">来源 Sheet</span>
          <span class="sf-merged-preview-sheet" :title="group.source_sheet">{{ group.source_sheet }}</span>
        </div>
        <el-tag size="small" effect="plain" :type="group.rows.length > 0 ? 'info' : 'warning'">
          {{ group.rows.length > 0 ? `前 ${group.rows.length} 行` : "没有数据行" }}
        </el-tag>
      </div>
      <el-table
        v-if="group.rows.length > 0"
        :data="rowsData(group)"
        size="small"
        border
        stripe
        :max-height="props.maxHeight"
      >
        <el-table-column prop="__source_file" label="来源表名" width="180" fixed="left" show-overflow-tooltip />
        <el-table-column prop="__source_sheet" label="来源 Sheet" width="130" fixed="left" show-overflow-tooltip />
        <el-table-column prop="__line" label="#" width="52" fixed="left" align="right" />
        <el-table-column
          v-for="(header, column) in preview.headers"
          :key="column"
          :prop="String(column)"
          :label="header"
          min-width="120"
          show-overflow-tooltip
        />
      </el-table>
      <div v-else class="sf-merged-preview-no-rows">该来源表没有可显示的数据行。</div>
    </div>
  </div>
</template>

<style scoped>
.sf-merged-preview {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.sf-merged-preview-group {
  overflow: hidden;
  border: 1px solid var(--sf-border);
  border-left: 4px solid var(--sf-primary);
  border-radius: 8px;
  background: var(--sf-bg-card);
}

.sf-merged-preview-group.alternate {
  border-left-color: #67c23a;
}

.sf-merged-preview-heading {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 42px;
  padding: 7px 12px;
  background: #f7f8fc;
  border-bottom: 1px solid var(--sf-border-soft);
}

.sf-merged-preview-source {
  display: flex;
  align-items: center;
  gap: 7px;
  min-width: 0;
  flex: 1;
}

.sf-merged-preview-label {
  flex-shrink: 0;
  color: var(--sf-text-muted);
  font-size: 11px;
}

.sf-merged-preview-file,
.sf-merged-preview-sheet {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  font-weight: 600;
}

.sf-merged-preview-file {
  max-width: 38%;
}

.sf-merged-preview-sheet {
  max-width: 24%;
}

.sf-merged-preview-no-rows,
.sf-merged-preview-empty {
  padding: 22px;
  color: var(--sf-text-muted);
  font-size: 12px;
  text-align: center;
}
</style>
