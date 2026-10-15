import { ref, computed } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { ElMessageBox } from "element-plus";
import { type SourceTable } from "../../types";
import { displayName } from "./utils";
import type { MergeContext } from "./context";
export function createSourcesState() {
  const sources = ref<SourceTable[]>([]);
  const inputLabel = ref("尚未选择文件或文件夹，可直接拖放");
  const warnings = ref<string[]>([]);
  const collapsedGroups = ref<Set<string>>(new Set());
  const sourceSearch = ref("");
  return {
    sources,
    inputLabel,
    warnings,
    collapsedGroups,
    sourceSearch,
  };
}
export function createSourcesActions(
  context: Pick<
    MergeContext,
    | "sources"
    | "collapsedGroups"
    | "sourceSearch"
    | "busy"
    | "refreshPlan"
    | "scanFolder"
    | "scanFiles"
    | "closeSourcePreview"
  >,
) {
  const {
    sources,
    collapsedGroups,
    sourceSearch,
    busy,
    refreshPlan,
    scanFolder,
    scanFiles,
    closeSourcePreview,
  } = context;
  const hasSources = computed(() => sources.value.length > 0);
  const sourceFilterActive = computed(
    () => sourceSearch.value.trim().length > 0,
  );
  const visibleSourceIndices = computed(() => {
    const filter = sourceSearch.value.trim().toLowerCase();
    if (!filter) return sources.value.map((_, index) => index);
    return sources.value
      .map((table, index) => ({ table, index }))
      .filter(({ table }) =>
        `${table.path} ${table.sheet_name}`.toLowerCase().includes(filter),
      )
      .map(({ index }) => index);
  });
  const visibleSourceCount = computed(() => visibleSourceIndices.value.length);
  const visibleEnabledCount = computed(
    () =>
      visibleSourceIndices.value.filter(
        (index) => sources.value[index]?.enabled,
      ).length,
  );
  const rowsMetric = computed(() =>
    sources.value
      .filter((t) => t.enabled)
      .reduce((sum, t) => sum + t.estimated_rows, 0),
  );
  const sheetsMetric = computed(() =>
    Math.max(1, Math.ceil(rowsMetric.value / 1048575)),
  );
  function mergeSources(
    existing: SourceTable[],
    incoming: SourceTable[],
  ): SourceTable[] {
    const seen = new Set(existing.map((t) => `${t.path}\u0000${t.sheet_name}`));
    const result = [...existing];
    for (const table of incoming) {
      const key = `${table.path}\u0000${table.sheet_name}`;
      if (!seen.has(key)) {
        seen.add(key);
        result.push(table);
      }
    }
    result.sort(
      (a, b) =>
        a.path.toLowerCase().localeCompare(b.path.toLowerCase()) ||
        a.sheet_name.toLowerCase().localeCompare(b.sheet_name.toLowerCase()),
    );
    return result;
  }
  function replaceReloadedTable(index: number, table: SourceTable) {
    const sameTable = (candidate: SourceTable | undefined) =>
      candidate?.path === table.path &&
      candidate.sheet_name === table.sheet_name;
    const target = sameTable(sources.value[index])
      ? index
      : sources.value.findIndex((candidate) => sameTable(candidate));
    if (target >= 0) sources.value[target] = table;
  }
  async function chooseFolder() {
    if (busy.value) return;
    const path = await open({ directory: true, title: "选择文件夹" });
    if (path) scanFolder(String(path));
  }
  async function chooseFiles() {
    if (busy.value) return;
    const paths = await open({
      multiple: true,
      title: "选择多个文件",
      filters: [
        {
          name: "表格文件",
          extensions: ["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"],
        },
      ],
    });
    if (paths && paths.length > 0) scanFiles(paths.map(String));
  }
  function toggleSourceEnabled(index: number, enabled: boolean) {
    const table = sources.value[index];
    if (table) table.enabled = enabled;
    void refreshPlan();
  }
  function selectAll(enabled: boolean, filtered = false) {
    const visible = filtered ? new Set(visibleSourceIndices.value) : null;
    let changed = false;
    for (const [index, table] of sources.value.entries()) {
      // 筛选状态下按当前结果重建选择集合，未命中的表不能继续参与合并。
      const nextEnabled = visible ? enabled && visible.has(index) : enabled;
      if (table.enabled !== nextEnabled) {
        table.enabled = nextEnabled;
        changed = true;
      }
    }
    if (!changed) return;
    void refreshPlan();
  }
  function toggleGroup(path: string) {
    const set = new Set(collapsedGroups.value);
    if (set.has(path)) set.delete(path);
    else set.add(path);
    collapsedGroups.value = set;
  }
  function setGroupEnabled(path: string, enabled: boolean, filtered = false) {
    const visible = filtered ? new Set(visibleSourceIndices.value) : null;
    let changed = false;
    for (const [index, table] of sources.value.entries()) {
      if (table.path !== path || (visible && !visible.has(index))) continue;
      if (table.enabled !== enabled) {
        table.enabled = enabled;
        changed = true;
      }
    }
    if (changed) void refreshPlan();
  }
  async function removeGroup(path: string) {
    const count = sources.value.filter((t) => t.path === path).length;
    try {
      await ElMessageBox.confirm(
        `确定移除该工作簿及其 ${count} 个数据表？`,
        "确认操作",
        { type: "warning" },
      );
    } catch {
      return;
    }
    closeSourcePreview();
    sources.value = sources.value.filter((t) => t.path !== path);
    const set = new Set(collapsedGroups.value);
    set.delete(path);
    collapsedGroups.value = set;
    void refreshPlan();
  }
  async function removeSource(index: number) {
    const table = sources.value[index];
    if (!table) return;
    try {
      await ElMessageBox.confirm(
        `确定移除数据表“${displayName(table)}”？`,
        "确认操作",
        { type: "warning" },
      );
    } catch {
      return;
    }
    closeSourcePreview();
    sources.value = sources.value.filter(
      (source) => source.path !== table.path || source.sheet_name !== table.sheet_name,
    );
    void refreshPlan();
  }
  return {
    hasSources,
    sourceFilterActive,
    visibleSourceIndices,
    visibleSourceCount,
    visibleEnabledCount,
    rowsMetric,
    sheetsMetric,
    mergeSources,
    replaceReloadedTable,
    chooseFolder,
    chooseFiles,
    toggleSourceEnabled,
    selectAll,
    toggleGroup,
    setGroupEnabled,
    removeGroup,
    removeSource,
  };
}
