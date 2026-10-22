<script setup lang="ts">
import { computed } from "vue";
import VirtualPreviewGrid from "./VirtualPreviewGrid.vue";
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
    const record: Record<string, string> = { __line: String(index + 1), __id: String(index) };
    row.forEach((cell, column) => {
      record[String(column)] = cell;
    });
    return record;
  });
});
const columns = computed(() => [{ key: "__line", title: "#", width: 52, fixed: "left" as const },
  ...(props.preview?.headers.map((title, column) => ({ key: String(column), title })) ?? [])]);
</script>

<template>
  <VirtualPreviewGrid v-if="preview" :columns="columns" :rows="rowsData" :max-height="maxHeight" />
</template>
