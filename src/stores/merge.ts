import { defineStore } from "pinia";
import { ref, computed } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ElMessage, ElMessageBox } from "element-plus";
import {
  defaultOptions,
  type AppSettings,
  type CheckIssue,
  type MergeFinished,
  type MergeOptions,
  type MergeProgress,
  type MergeScheme,
  type PreflightDone,
  type PreviewTable,
  type ScanFinished,
  type ScanProgress,
  type SourceTable,
  type TableReloaded,
  type TablesReloaded,
  type UpdateResult,
} from "../types";

export type AppPhase = "ready" | "scanning" | "checking" | "merging";

const MODE_LABELS: Record<string, string> = {
  Union: "列名并集",
  Intersection: "列名交集",
  Manual: "手动映射",
  Consolidate: "按键汇总",
  Join: "横向关联",
};

export const useMergeStore = defineStore("merge", () => {
  // ---- 状态（原 MergeApp 的状态集） ----
  const sources = ref<SourceTable[]>([]);
  const options = ref<MergeOptions>(defaultOptions());
  const outputPath = ref("");
  const inputLabel = ref("尚未选择文件或文件夹，可直接拖放");
  const phase = ref<AppPhase>("ready");
  const progress = ref(0);
  const progressLabel = ref("");
  const warnings = ref<string[]>([]);
  const checkIssues = ref<CheckIssue[]>([]);
  const checkRan = ref(false);
  const preview = ref<PreviewTable | null>(null);
  const previewTitle = ref("");
  const settings = ref<AppSettings | null>(null);
  const updateText = ref("检查更新");
  const updateUrl = ref<string | null>(null);
  const collapsedGroups = ref<Set<string>>(new Set());
  const selectedMappingTable = ref(0);
  const hideCommonMappings = ref(false);
  const mismatchOnly = ref(false);
  const sourceSearch = ref("");
  const mappingSearch = ref("");
  const planHeaders = ref<string[]>([]);
  const planCommonKeys = ref<Set<string>>(new Set());
  const scanAppend = ref(false);
  const activePage = ref(0);
  const showAbout = ref(false);

  // ---- 派生 ----
  const busy = computed(() => phase.value !== "ready");
  const hasSources = computed(() => sources.value.length > 0);
  const enabledIndices = computed(() =>
    sources.value.map((t, i) => (t.enabled ? i : -1)).filter((i) => i >= 0)
  );
  const rowsMetric = computed(() =>
    sources.value
      .filter((t) => t.enabled)
      .reduce((sum, t) => sum + t.estimated_rows, 0)
  );
  const sheetsMetric = computed(() => Math.max(1, Math.ceil(rowsMetric.value / 1_048_575)));
  const canStart = computed(
    () => !busy.value && sources.value.filter((t) => t.enabled).length > 0 && planHeaders.value.length > 0
  );

  // ---- 工具 ----
  const formatNumber = (value: number) => value.toLocaleString("en-US");

  function mergeSources(existing: SourceTable[], incoming: SourceTable[]): SourceTable[] {
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
        a.sheet_name.toLowerCase().localeCompare(b.sheet_name.toLowerCase())
    );
    return result;
  }

  async function refreshPlan() {
    try {
      const plan = await invoke<{ headers: string[]; common_keys: string[] }>("get_plan", {
        tables: sources.value,
        options: options.value,
      });
      planHeaders.value = plan.headers;
      planCommonKeys.value = new Set(plan.common_keys);
    } catch (error) {
      planHeaders.value = [];
      planCommonKeys.value = new Set();
    }
  }

  function displayName(table: SourceTable): string {
    const file = table.path.split(/[\\/]/).pop() ?? "";
    return `${file}  /  ${table.sheet_name}`;
  }

  // ---- 扫描 / 数据源 ----
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
      filters: [{ name: "表格文件", extensions: ["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv"] }],
    });
    if (paths && paths.length > 0) scanFiles(paths.map(String));
  }

  async function scanFolder(path: string) {
    scanAppend.value = sources.value.length > 0;
    if (!outputPath.value.trim()) {
      outputPath.value = `${path.replace(/[\\/]$/, "")}\\合并结果.xlsx`;
    }
    phase.value = "scanning";
    progress.value = 0;
    progressLabel.value = scanAppend.value ? "正在追加并读取新文件…" : "正在读取文件并自动识别表头…";
    inputLabel.value = path;
    warnings.value = [];
    preview.value = null;
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
    scanAppend.value = sources.value.length > 0;
    if (!outputPath.value.trim() && paths.length > 0) {
      const folder = paths[0].replace(/[\\/][^\\/]*$/, "");
      outputPath.value = `${folder}\\合并结果.xlsx`;
    }
    phase.value = "scanning";
    progress.value = 0;
    progressLabel.value = scanAppend.value ? "正在追加并读取新文件…" : "正在读取文件并自动识别表头…";
    inputLabel.value = `已选择 ${paths.length} 个文件`;
    warnings.value = [];
    preview.value = null;
    checkIssues.value = [];
    checkRan.value = false;
    try {
      await invoke("scan_files", { paths });
    } catch (error) {
      phase.value = "ready";
      ElMessage.error(String(error));
    }
  }

  async function clearSources() {
    if (!hasSources.value) return;
    try {
      await ElMessageBox.confirm("确定清空所有数据源？", "确认操作", { type: "warning" });
    } catch {
      return;
    }
    sources.value = [];
    warnings.value = [];
    preview.value = null;
    checkIssues.value = [];
    checkRan.value = false;
    collapsedGroups.value = new Set();
    inputLabel.value = "尚未选择文件或文件夹，可直接拖放";
    planHeaders.value = [];
    await refreshPlan();
  }

  function toggleSourceEnabled(index: number, enabled: boolean) {
    const table = sources.value[index];
    if (table) table.enabled = enabled;
    if (enabledIndices.value.length > 0 && !enabledIndices.value.includes(selectedMappingTable.value)) {
      selectedMappingTable.value = enabledIndices.value[0];
    }
    void refreshPlan();
  }

  function selectAll(enabled: boolean) {
    for (const table of sources.value) table.enabled = enabled;
    if (enabledIndices.value.length > 0) selectedMappingTable.value = enabledIndices.value[0];
    void refreshPlan();
  }

  function toggleGroup(path: string) {
    const set = new Set(collapsedGroups.value);
    if (set.has(path)) set.delete(path);
    else set.add(path);
    collapsedGroups.value = set;
  }

  function setGroupEnabled(path: string, enabled: boolean) {
    for (const table of sources.value) {
      if (table.path === path) table.enabled = enabled;
    }
    void refreshPlan();
  }

  async function removeGroup(path: string) {
    const count = sources.value.filter((t) => t.path === path).length;
    try {
      await ElMessageBox.confirm(`确定移除该工作簿及其 ${count} 个数据表？`, "确认操作", { type: "warning" });
    } catch {
      return;
    }
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
      await ElMessageBox.confirm(`确定移除数据表“${displayName(table)}”？`, "确认操作", { type: "warning" });
    } catch {
      return;
    }
    sources.value.splice(index, 1);
    void refreshPlan();
  }

  function applyGroupHeader(path: string) {
    const group = sources.value
      .map((table, index) => ({ index, table }))
      .filter((item) => item.table.path === path);
    if (group.length === 0) return;
    const headerRow = group[0].table.header_row;
    const headerRows = group[0].table.header_rows;
    phase.value = "scanning";
    progressLabel.value = `正在把从第 ${headerRow} 行开始、占用 ${headerRows} 行的表头设置应用到整本工作簿…`;
    void invoke("reload_group", {
      sources: group.map((item) => ({ index: item.index, table: item.table })),
      headerRow,
      headerRows,
    });
  }

  function reloadTable(index: number, headerRow: number, headerRows: number) {
    const table = sources.value[index];
    if (!table) return;
    phase.value = "scanning";
    progressLabel.value = `正在从第 ${headerRow} 行开始、读取 ${headerRows} 行表头：${displayName(table)}`;
    void invoke("reload_table", { index, table, headerRow, headerRows });
  }

  // ---- 合并规则 ----
  function setMode(mode: MergeOptions["mode"]) {
    options.value.mode = mode;
    void refreshPlan();
  }

  function setAdvanced(patch: Partial<MergeOptions>) {
    Object.assign(options.value, patch);
    void refreshPlan();
  }

  function moveOutputColumn(index: number, direction: number) {
    const headers = [...planHeaders.value];
    if (index < 0 || index >= headers.length) return;
    const target = Math.min(Math.max(index + direction, 0), headers.length - 1);
    [headers[index], headers[target]] = [headers[target], headers[index]];
    options.value.output_order = headers;
    void refreshPlan();
  }

  function selectedTable(): SourceTable | undefined {
    return sources.value[selectedMappingTable.value];
  }

  function setMapping(index: number, enabled: boolean, target: string) {
    const table = selectedTable();
    const mapping = table?.mappings[index];
    if (mapping) {
      mapping.enabled = enabled;
      mapping.target_name = target;
    }
    void refreshPlan();
  }

  function setMappingOperation(index: number, transform: number, aggregate: number) {
    const table = selectedTable();
    const mapping = table?.mappings[index];
    if (!mapping) return;
    const transforms = ["None", "Trim", "Uppercase", "Lowercase"] as const;
    const aggregates = ["First", "Sum", "UniqueJoin", "TextJoin"] as const;
    mapping.transform = transforms[transform] ?? "None";
    mapping.aggregate = aggregates[aggregate] ?? "First";
    void refreshPlan();
  }

  async function resetMapping() {
    const table = selectedTable();
    if (!table) return;
    try {
      await ElMessageBox.confirm(`确定恢复“${displayName(table)}”的所有字段映射？`, "确认操作", { type: "warning" });
    } catch {
      return;
    }
    for (const mapping of table.mappings) {
      mapping.target_name = mapping.source_name;
      mapping.enabled = true;
      mapping.transform = "None";
      mapping.aggregate = "First";
    }
    void refreshPlan();
  }

  async function applySuggestions() {
    const suggestions = await invoke<Record<string, string>>("get_suggestions", {
      tables: sources.value,
    });
    for (const table of sources.value) {
      for (const mapping of table.mappings) {
        const target = suggestions[mapping.source_name];
        if (target) mapping.target_name = target;
      }
    }
    void refreshPlan();
  }

  // ---- 预览 / 检查 ----
  async function showSourcePreview(index: number) {
    const table = sources.value[index];
    if (!table) return;
    try {
      const result = await invoke<PreviewTable>("preview_source", { table, limit: 30 });
      preview.value = result;
      previewTitle.value = `${displayName(table)} · 前 ${result.rows.length} 行`;
    } catch (error) {
      ElMessage.error(`预览失败：${error}`);
    }
  }

  async function showMergedPreview() {
    try {
      const result = await invoke<PreviewTable>("preview_merged", {
        tables: sources.value,
        options: options.value,
        limit: 30,
      });
      preview.value = result;
      previewTitle.value = `合并结果预览 · ${result.headers.length} 列 · 前 ${result.rows.length} 行`;
    } catch (error) {
      ElMessage.error(`结果预览失败：${error}`);
    }
  }

  async function runPreflight(continuesMerge = false) {
    if (busy.value) return;
    phase.value = "checking";
    progress.value = 0;
    progressLabel.value = "正在执行合并前检查…";
    await invoke("run_preflight", {
      tables: sources.value,
      options: options.value,
      continuesMerge,
    });
  }

  async function confirmMerge() {
    try {
      await invoke("start_merge", {
        tables: sources.value,
        options: options.value,
        output: outputPath.value,
      });
      phase.value = "merging";
      progress.value = 0;
      progressLabel.value = "正在准备输出工作簿…";
    } catch (error) {
      phase.value = "ready";
      ElMessage.error(String(error));
    }
  }

  async function startMerge() {
    if (!outputPath.value.trim()) {
      ElMessage.error("请先选择输出文件");
      return;
    }
    await runPreflight(true);
  }

  async function cancelMerge() {
    if (phase.value !== "merging") return;
    try {
      await ElMessageBox.confirm("确定取消当前合并？已写入的部分不会保留。", "确认操作", { type: "warning" });
    } catch {
      return;
    }
    progressLabel.value = "正在取消…";
    await invoke("cancel_merge");
  }

  // ---- 方案 ----
  async function saveScheme() {
    const path = await save({
      title: "保存合并方案",
      defaultPath: "合并方案.json",
      filters: [{ name: "表格合并方案", extensions: ["json"] }],
    });
    if (!path) return;
    try {
      await invoke("save_scheme", {
        path: String(path),
        tables: sources.value,
        options: options.value,
      });
      progressLabel.value = `方案已保存：${path}`;
      ElMessage.success("方案已保存");
      await loadState();
    } catch (error) {
      ElMessage.error(`保存方案失败：${error}`);
    }
  }

  async function openScheme() {
    const path = await open({
      title: "打开合并方案",
      multiple: false,
      filters: [{ name: "表格合并方案", extensions: ["json"] }],
    });
    if (!path) await openSchemeByPath(String(path));
  }

  async function openSchemeByPath(path: string) {
    try {
      const scheme = await invoke<MergeScheme>("open_scheme", { path });
      sources.value = scheme.tables;
      options.value = scheme.options;
      selectedMappingTable.value = enabledIndices.value[0] ?? 0;
      inputLabel.value = `已打开方案：${path}`;
      checkIssues.value = [];
      checkRan.value = false;
      await loadState();
      await refreshPlan();
    } catch (error) {
      ElMessage.error(`打开方案失败：${error}`);
    }
  }

  // ---- 更新 / 其他 ----
  async function checkUpdate() {
    updateText.value = "正在检查…";
    await invoke("check_update");
  }

  async function loadState() {
    const state = await invoke<{ settings: AppSettings }>("get_state");
    settings.value = state.settings;
    if (!outputPath.value.trim() && state.settings.output_directory) {
      outputPath.value = `${state.settings.output_directory}\\合并结果.xlsx`;
    }
  }

  // ---- 事件桥（启动时调用一次） ----
  let unlisteners: UnlistenFn[] = [];

  async function initEvents() {
    unlisteners.push(
      await listen<ScanProgress>("scan-progress", (e) => {
        progress.value = e.payload.total === 0 ? 0 : e.payload.done / e.payload.total;
        progressLabel.value = `正在扫描：${e.payload.name}`;
      }),
      await listen<ScanFinished>("scan-finished", (e) => {
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
        selectedMappingTable.value = enabledIndices.value[0] ?? 0;
        phase.value = "ready";
        progress.value = 1;
        progressLabel.value = `已识别 ${sources.value.length} 个数据表`;
        void refreshPlan();
      }),
      await listen<{ message: string }>("scan-failed", (e) => {
        phase.value = "ready";
        ElMessage.error(e.payload.message);
      }),
      await listen<TableReloaded>("table-reloaded", (e) => {
        if (sources.value[e.payload.index]) sources.value[e.payload.index] = e.payload.table;
        selectedMappingTable.value = e.payload.index;
        preview.value = null;
        checkIssues.value = [];
        checkRan.value = false;
        phase.value = "ready";
        progressLabel.value = `表头已刷新：${displayName(e.payload.table)}`;
        void refreshPlan();
      }),
      await listen<TablesReloaded>("tables-reloaded", (e) => {
        for (const item of e.payload.tables) {
          if (sources.value[item.index]) sources.value[item.index] = item.table;
        }
        preview.value = null;
        checkIssues.value = [];
        checkRan.value = false;
        phase.value = "ready";
        progressLabel.value = `已统一刷新 ${e.payload.tables.length} 个数据表的表头`;
        void refreshPlan();
      }),
      await listen<{ index: number; message: string }>("table-reload-failed", (e) => {
        phase.value = "ready";
        ElMessage.error(`重新读取表头失败：${e.payload.message}`);
      }),
      await listen<MergeProgress>("merge-progress", (e) => {
        progress.value = e.payload.total === 0 ? 0 : e.payload.current / e.payload.total;
        progressLabel.value = `${e.payload.label}  ·  ${e.payload.current.toLocaleString()} / ${e.payload.total.toLocaleString()} 行`;
      }),
      await listen<MergeFinished>("merge-finished", (e) => {
        phase.value = "ready";
        progress.value = 1;
        progressLabel.value = "合并完成";
        ElMessage.success(`合并完成：${formatNumber(e.payload.rows)} 行，${e.payload.sheets} 个 Sheet`);
        void revealOutput(e.payload.output);
      }),
      await listen("merge-cancelled", () => {
        phase.value = "ready";
        progressLabel.value = "已取消";
      }),
      await listen<{ message: string }>("merge-failed", (e) => {
        phase.value = "ready";
        progressLabel.value = "";
        ElMessage.error(`合并失败：${e.payload.message}`);
      }),
      await listen<PreflightDone>("preflight-done", async (e) => {
        checkIssues.value = e.payload.issues;
        checkRan.value = true;
        const errors = e.payload.issues.filter((i) => i.level === "Error");
        if (errors.length > 0) {
          phase.value = "ready";
          progressLabel.value = "";
          ElMessage.error(`合并前检查未通过：${errors[0].title} — ${errors[0].detail}`);
        } else if (e.payload.continues_merge) {
          const exists = await invoke<boolean>("path_exists", { path: outputPath.value });
          if (exists) {
            try {
              await ElMessageBox.confirm(`${outputPath.value} 已存在，是否覆盖？`, "覆盖已有文件", { type: "warning" });
            } catch {
              phase.value = "ready";
              return;
            }
          }
          await confirmMerge();
        } else {
          phase.value = "ready";
          const warnCount = e.payload.issues.filter((i) => i.level === "Warning").length;
          progressLabel.value = `检查完成：${errors.length} 个错误，${warnCount} 个提醒`;
        }
      }),
      await listen<UpdateResult>("update-result", (e) => {
        if (e.payload.newer) {
          updateText.value = `发现 ${e.payload.version}，打开下载页`;
          updateUrl.value = e.payload.url;
        } else {
          updateText.value = "已是最新版本";
        }
      }),
      await listen<{ message: string }>("update-failed", (e) => {
        updateText.value = "检查失败，重试";
        ElMessage.error(`检查更新失败：${e.payload.message}`);
      })
    );
  }

  async function revealOutput(output?: string) {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
      await revealItemInDir(output ?? outputPath.value);
    } catch {
      // 忽略：文件可能不存在
    }
  }

  async function openLog() {
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    const logPath = await invoke<string>("get_log_path");
    try {
      await revealItemInDir(logPath);
    } catch {
      ElMessage.error("日志文件尚未生成");
    }
  }

  return {
    // state
    sources, options, outputPath, inputLabel, phase, progress, progressLabel, warnings,
    checkIssues, checkRan, preview, previewTitle, settings, updateText, updateUrl,
    collapsedGroups, selectedMappingTable, hideCommonMappings, mismatchOnly,
    sourceSearch, mappingSearch, planHeaders, planCommonKeys, activePage, showAbout,
    // getters
    busy, hasSources, enabledIndices, rowsMetric, sheetsMetric, canStart, formatNumber,
    // actions
    chooseFolder, chooseFiles, scanFolder, scanFiles, clearSources,
    toggleSourceEnabled, selectAll, toggleGroup, setGroupEnabled, removeGroup, removeSource,
    applyGroupHeader, reloadTable,
    setMode, setAdvanced, moveOutputColumn, selectedTable, setMapping, setMappingOperation,
    resetMapping, applySuggestions, toggleCommonFields: () => (hideCommonMappings.value = !hideCommonMappings.value),
    setMismatchOnly: (v: boolean) => (mismatchOnly.value = v),
    showSourcePreview, showMergedPreview, runPreflight, startMerge, cancelMerge,
    saveScheme, openScheme, openSchemeByPath, checkUpdate, loadState, initEvents, revealOutput, openLog, refreshPlan,
    displayName, MODE_LABELS,
  };
});
