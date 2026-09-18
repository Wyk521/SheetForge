<script setup lang="ts">
import { computed } from "vue";
import type { MergedPreview, MergedPreviewGroup } from "../types";

const props = withDefaults(
  defineProps<{
    preview: MergedPreview | null;
    maxHeight?: number | string;
  }>(),
  { maxHeight: 430 }
);

interface MergedPreviewRow {
  [key: string]: string | number | boolean;
  __line: string;
  __source_file: string;
  __source_sheet: string;
  __group_index: number;
  __group_start: boolean;
  __empty: boolean;
}

const previewRows = computed<MergedPreviewRow[]>(() => {
  if (!props.preview) return [];

  return props.preview.groups.flatMap((group: MergedPreviewGroup, groupIndex) => {
    if (group.rows.length === 0) {
      return [
        {
          __line: "空表",
          __source_file: group.source_file,
          __source_sheet: group.source_sheet,
          __group_index: groupIndex,
          __group_start: true,
          __empty: true,
        },
      ];
    }

    return group.rows.map((row, rowIndex) => {
      const record: MergedPreviewRow = {
        __line: String(rowIndex + 1),
        __source_file: group.source_file,
        __source_sheet: group.source_sheet,
        __group_index: groupIndex,
        __group_start: rowIndex === 0,
        __empty: false,
      };
      row.forEach((cell, column) => {
        record[String(column)] = cell;
      });
      return record;
    });
  });
});

function rowClassName({ row }: { row: MergedPreviewRow }) {
  return [
    row.__group_start ? "sf-merged-preview-group-start" : "",
    row.__group_index % 2 === 1 ? "sf-merged-preview-alternate" : "",
    row.__empty ? "sf-merged-preview-empty-row" : "",
  ]
    .filter(Boolean)
    .join(" ");
}
</script>

<template>
  <div v-if="preview" class="sf-merged-preview">
    <div v-if="preview.groups.length === 0" class="sf-merged-preview-empty">
      没有已选中的数据表，请先在数据源中勾选要参与合并的表。
    </div>
    <el-table
      v-else
      class="sf-merged-preview-table"
      :data="previewRows"
      size="small"
      border
      stripe
      :row-class-name="rowClassName"
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
  </div>
</template>

<style scoped>
.sf-merged-preview {
  min-width: 0;
}

.sf-merged-preview-table {
  width: 100%;
}

:deep(.sf-merged-preview-group-start > td) {
  border-top: 2px solid var(--sf-primary) !important;
}

:deep(.sf-merged-preview-group-start.sf-merged-preview-alternate > td) {
  border-top-color: #67c23a !important;
}

:deep(.sf-merged-preview-alternate:not(.sf-merged-preview-group-start) > td) {
  background: #fbfdf9;
}

:deep(.sf-merged-preview-empty-row > td) {
  color: var(--sf-text-muted);
  font-style: italic;
}

.sf-merged-preview-empty {
  padding: 22px;
  color: var(--sf-text-muted);
  font-size: 12px;
  text-align: center;
}
</style>
