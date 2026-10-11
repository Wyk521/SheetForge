import { ref, computed } from "vue";
import { invoke } from "../../native";
import { open } from "@tauri-apps/plugin-dialog";
import { ElMessage, ElMessageBox } from "element-plus";
import { type SourceTable } from "../../types";
import { displayName } from "./utils";
import type { MergeContext } from "./context";
import type { NativeOn } from "../../native";
export function createSourcesState() {
  const sources = ref<SourceTable[]>([]);
  const inputLabel = ref("尚未选择文件或文件夹，可直接拖放");
  const warnings = ref<string[]>([]);
  const collapsedGroups = ref<Set<string>>(new Set());
  const sourceSearch = ref("");
  const scanAppend = ref(false);
  return {
    sources,
    inputLabel,
    warnings,
    collapsedGroups,
    sourceSearch,
    scanAppend,
  };
}
export function createSourcesActions(
  context: Pick<
    MergeContext,
    | "sources"
    | "inputLabel"
    | "warnings"
    | "collapsedGroups"
    | "sourceSearch"
    | "scanAppend"
    | "options"
    | "checkIssues"
    | "checkRan"
    | "outputPath"
    | "phase"
    | "progress"
    | "progressLabel"
    | "busy"
    | "refreshPlan"
    | "clearMergedPreview"
    | "closeSourcePreview"
  >,
) {
  const {
    sources,
    inputLabel,
    warnings,
    collapsedGroups,
    sourceSearch,
    scanAppend,
    options,
    checkIssues,
    checkRan,
    outputPath,
    phase,
    progress,
    progressLabel,
    busy,
    refreshPlan,
    clearMergedPreview,
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
  async function scanFolder(path: string) {
    if (busy.value) return;
    scanAppend.value = sources.value.length > 0;
    if (!outputPath.value.trim()) {
      outputPath.value = `${path.replace(/[\\/]$/, "")}\\合并结果.xlsx`;
    }
    phase.value = "scanning";
    progress.value = 0;
    progressLabel.value = scanAppend.value
      ? "正在追加并读取新文件…"
      : "正在读取文件并自动识别表头…";
    inputLabel.value = path;
    warnings.value = [];
    clearMergedPreview();
    closeSourcePreview();
    checkIssues.value = [];
    checkRan.value = false;
    try {
      await invoke("scan_folder", { path });
    } catch (error) {
      phase.value = "ready";
      ElMessage.error(String(error));
    }
  }
  async function scanFiles(paths: string[]) {
    if (busy.value) return;
    scanAppend.value = sources.value.length > 0;
    if (!outputPath.value.trim() && paths.length > 0) {
      const folder = paths[0].replace(/[\\/][^\\/]*$/, "");
      outputPath.value = `${folder}\\合并结果.xlsx`;
    }
    phase.value = "scanning";
    progress.value = 0;
    progressLabel.value = scanAppend.value
      ? "正在追加并读取新文件…"
      : "正在读取文件并自动识别表头…";
    inputLabel.value = `已选择 ${paths.length} 个文件`;
    warnings.value = [];
    clearMergedPreview();
    closeSourcePreview();
    checkIssues.value = [];
    checkRan.value = false;
    try {
      await invoke("scan_files", { paths });
    } catch (error) {
      phase.value = "ready";
      ElMessage.error(String(error));
    }
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
    sources.value.splice(index, 1);
    void refreshPlan();
  }
  async function applyGroupHeader(path: string) {
    const group = sources.value
      .map((table, index) => ({ index, table }))
      .filter((item) => item.table.path === path);
    if (group.length === 0) return;
    const headerRow = group[0].table.header_row;
    const headerRows = group[0].table.header_rows;
    phase.value = "scanning";
    progressLabel.value = `正在把从第 ${headerRow} 行开始、占用 ${headerRows} 行的表头设置应用到整本工作簿…`;
    try {
      await invoke("reload_group", {
        sources: group.map((item) => ({
          index: item.index,
          table: item.table,
        })),
        headerRow,
        headerRows,
      });
    } catch (error) {
      phase.value = "ready";
      progressLabel.value = "";
      ElMessage.error(String(error));
    }
  }
  async function reloadTable(
    index: number,
    headerRow: number,
    headerRows: number,
  ) {
    const table = sources.value[index];
    if (!table) return;
    phase.value = "scanning";
    progressLabel.value = `正在从第 ${headerRow} 行开始、读取 ${headerRows} 行表头：${displayName(table)}`;
    try {
      await invoke("reload_table", { index, table, headerRow, headerRows });
    } catch (error) {
      phase.value = "ready";
      progressLabel.value = "";
      ElMessage.error(String(error));
    }
  }
  async function registerEvents(on: NativeOn) {
    await on("scan-progress", (e) => {
      progress.value =
        e.payload.total === 0 ? 0 : e.payload.done / e.payload.total;
      progressLabel.value = `正在扫描：${e.payload.name}`;
    });
    await on("scan-finished", (e) => {
      const appended = scanAppend.value;
      scanAppend.value = false;
      if (appended) {
        const before = sources.value.length;
        sources.value = mergeSources(sources.value, e.payload.tables);
        const added = sources.value.length - before;
        inputLabel.value = `当前共 ${sources.value.length} 个数据表（本次新增 ${added} 个）`;
      } else {
        sources.value = e.payload.tables;
        options.value.output_order = [];
      }
      warnings.value = e.payload.warnings;
      phase.value = "ready";
      progress.value = 1;
      progressLabel.value = `已识别 ${sources.value.length} 个数据表`;
      void refreshPlan();
    });
    await on("scan-failed", (e) => {
      phase.value = "ready";
      ElMessage.error(e.payload.message);
    });
    await on("table-reloaded", (e) => {
      replaceReloadedTable(e.payload.index, e.payload.table);
      clearMergedPreview();
      closeSourcePreview();
      checkIssues.value = [];
      checkRan.value = false;
      phase.value = "ready";
      progressLabel.value = `表头已刷新：${displayName(e.payload.table)}`;
      void refreshPlan();
    });
    await on("tables-reloaded", (e) => {
      for (const item of e.payload.tables) {
        replaceReloadedTable(item.index, item.table);
      }
      clearMergedPreview();
      closeSourcePreview();
      checkIssues.value = [];
      checkRan.value = false;
      phase.value = "ready";
      const failures = e.payload.failures ?? [];
      progressLabel.value =
        failures.length > 0
          ? `已刷新 ${e.payload.tables.length} 个数据表，${failures.length} 个失败`
          : `已统一刷新 ${e.payload.tables.length} 个数据表的表头`;
      if (failures.length > 0) {
        ElMessage.warning(
          `${failures.length} 个数据表刷新失败，其余已应用：${failures[0]}`,
        );
      }
      void refreshPlan();
    });
    await on("table-reload-failed", (e) => {
      phase.value = "ready";
      ElMessage.error(`重新读取表头失败：${e.payload.message}`);
    });
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
    scanFolder,
    scanFiles,
    toggleSourceEnabled,
    selectAll,
    toggleGroup,
    setGroupEnabled,
    removeGroup,
    removeSource,
    applyGroupHeader,
    reloadTable,
    registerEvents,
  };
}
