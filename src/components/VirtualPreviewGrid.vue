<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { visibleRange } from "../utils/previewWindow";

export interface PreviewColumn { key: string; title: string; width?: number; fixed?: "left" }
const props = withDefaults(defineProps<{
  columns: PreviewColumn[];
  rows: Record<string, unknown>[];
  maxHeight?: number | string;
  rowClassName?: (args: { row: Record<string, unknown> }) => string;
}>(), { maxHeight: 440 });
const viewport = ref<HTMLElement>();
const width = ref(800);
const scrollLeft = ref(0);
const scrollTop = ref(0);
const rowHeight = 32;
const height = computed(() => Math.max(80, Math.min(Number.parseFloat(String(props.maxHeight)) || 440,
  rowHeight * (props.rows.length + 1) + 14)));
const columns = computed(() => {
  let left = 0;
  return props.columns.map((column, index) => {
    const result = { ...column, index, left, width: column.width ?? 140 };
    left += result.width;
    return result;
  });
});
const totalWidth = computed(() => columns.value.reduce((total, column) => total + column.width, 0));
const fixedColumns = computed(() => columns.value.filter(column => column.fixed));
const fixedWidth = computed(() => fixedColumns.value.reduce((total, column) => total + column.width, 0));
const visibleColumns = computed(() => columns.value.filter(column => !column.fixed
  && column.left + column.width >= scrollLeft.value + fixedWidth.value - 140
  && column.left <= scrollLeft.value + width.value + 140));
const visibleRows = computed(() => {
  const [start, end] = visibleRange(props.rows.length, scrollTop.value, height.value - rowHeight, rowHeight);
  return props.rows.slice(start, end).map((row, offset) => ({ row, index: start + offset }));
});
const text = (row: Record<string, unknown>, key: string) => String(row[key] ?? "");
function onScroll() {
  scrollLeft.value = viewport.value?.scrollLeft ?? 0;
  scrollTop.value = viewport.value?.scrollTop ?? 0;
}
let observer: ResizeObserver | undefined;
onMounted(() => {
  observer = new ResizeObserver(() => { if (viewport.value?.clientWidth) width.value = viewport.value.clientWidth; });
  if (viewport.value) { width.value = viewport.value.clientWidth || 800; observer.observe(viewport.value); }
});
onBeforeUnmount(() => observer?.disconnect());
</script>

<template>
  <div ref="viewport" class="sf-virtual-preview" :style="{ height: `${height}px` }" role="grid" aria-label="数据预览"
    :aria-rowcount="rows.length + 1" :aria-colcount="columns.length" tabindex="0" @scroll="onScroll">
    <div class="sf-virtual-content" :style="{ width: `${totalWidth}px`, minWidth: '100%', height: `${(rows.length + 1) * rowHeight}px` }">
      <div class="sf-virtual-header" role="row" aria-rowindex="1">
        <div v-for="column in visibleColumns" :key="column.key" role="columnheader" class="sf-virtual-cell"
          :aria-colindex="column.index + 1" :title="column.title" :style="{ left: `${column.left}px`, width: `${column.width}px` }">{{ column.title }}</div>
        <div class="sf-virtual-fixed" :style="{ width: `${fixedWidth}px` }">
          <div v-for="column in fixedColumns" :key="column.key" role="columnheader" class="sf-virtual-fixed-cell"
            :aria-colindex="column.index + 1" :title="column.title" :style="{ width: `${column.width}px` }">{{ column.title }}</div>
        </div>
      </div>
      <div v-for="{ row, index } in visibleRows" :key="String(row.__id ?? index)" class="sf-virtual-row" :class="rowClassName?.({ row })"
        role="row" :aria-rowindex="index + 2" :style="{ top: `${(index + 1) * rowHeight}px` }">
        <div v-for="column in visibleColumns" :key="column.key" role="gridcell" class="sf-virtual-cell"
          :aria-colindex="column.index + 1" :title="text(row, column.key)" :style="{ left: `${column.left}px`, width: `${column.width}px` }">{{ text(row, column.key) }}</div>
        <div class="sf-virtual-fixed" :style="{ width: `${fixedWidth}px` }">
          <div v-for="column in fixedColumns" :key="column.key" role="gridcell" class="sf-virtual-fixed-cell"
            :aria-colindex="column.index + 1" :title="text(row, column.key)" :style="{ width: `${column.width}px` }">{{ text(row, column.key) }}</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sf-virtual-preview { width: 100%; min-width: 0; overflow: auto; position: relative; border: 1px solid var(--el-border-color-lighter); color: var(--el-text-color-regular); background: var(--el-bg-color); }
.sf-virtual-content { position: relative; }
.sf-virtual-header, .sf-virtual-row { width: 100%; height: 32px; line-height: 31px; box-sizing: border-box; border-bottom: 1px solid var(--el-border-color-lighter); background: var(--el-bg-color); }
.sf-virtual-header { position: sticky; top: 0; z-index: 3; font-weight: 600; background: var(--el-fill-color-light); }
.sf-virtual-row { position: absolute; left: 0; }
.sf-virtual-cell, .sf-virtual-fixed-cell { height: 100%; box-sizing: border-box; padding: 0 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); border-right: 1px solid var(--el-border-color-lighter); }
.sf-virtual-cell { position: absolute; top: 0; }
.sf-virtual-fixed { position: sticky; left: 0; display: flex; height: 100%; background: inherit; z-index: 2; }
.sf-virtual-fixed-cell { flex-shrink: 0; }
</style>
