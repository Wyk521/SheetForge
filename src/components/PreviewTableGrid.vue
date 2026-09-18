<script setup lang="ts">
import { computed } from "vue";
import type { PreviewTable } from "../types";

const props = withDefaults(
  defineProps<{
    preview: PreviewTable | null;
    maxHeight?: number | string;
  }>(),
  { maxHeight: 440 }
);

const rowsData = computed(() => {
  if (!props.preview) return [];
  return props.preview.rows.map((row, index) => {
    const record: Record<string, string> = { __line: String(index + 1) };
    row.forEach((cell, column) => {
      record[String(column)] = cell;
    });
    return record;
  });
});
</script>

<template>
  <el-table v-if="preview" :data="rowsData" size="small" border :max-height="maxHeight">
    <el-table-column prop="__line" label="#" width="52" align="right" />
    <el-table-column
      v-for="(header, column) in preview.headers"
      :key="column"
      :prop="String(column)"
      :label="header"
      min-width="120"
      show-overflow-tooltip
    />
  </el-table>
</template>
