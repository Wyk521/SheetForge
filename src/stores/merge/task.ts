import { ref, computed } from "vue";
import { invoke } from "../../native";
import { ElMessage, ElMessageBox } from "element-plus";
import {
  defaultDatabaseImportOptions,
  type OutputDestination,
} from "../../types";
import { displayName, formatNumber } from "./utils";
import type { MergeContext, AppPhase } from "./context";
import type { NativeOn } from "../../native";
export function createTaskState() {
  const scanAppend = ref(false);
  const outputPath = ref("");
  const outputDestination = ref<OutputDestination>("xlsx");
  const phase = ref<AppPhase>("ready");
  const progress = ref(0);
  const progressLabel = ref("");
  return { scanAppend, outputPath, outputDestination, phase, progress, progressLabel };
}
export function createTaskActions(
  context: Pick<
    MergeContext,
    | "scanAppend"
    | "mergeSources"
    | "replaceReloadedTable"
    | "loadDatabaseProfiles"
    | "loadSettings"
    | "persistScheme"
    | "chooseScheme"
    | "readScheme"
    | "databaseImport"
    | "sources"
    | "options"
    | "planHeaders"
    | "checkIssues"
    | "checkRan"
    | "outputPath"
    | "outputDestination"
    | "phase"
    | "progress"
    | "progressLabel"
    | "databaseReady"
    | "busy"
    | "databaseLastResult"
    | "warnings"
    | "collapsedGroups"
    | "inputLabel"
    | "refreshPlan"
    | "clearMergedPreview"
    | "closeSourcePreview"
    | "openDatabaseTarget"
  >,
) {
  const {
    scanAppend,
    mergeSources,
    replaceReloadedTable,
    loadDatabaseProfiles,
    loadSettings,
    persistScheme,
    chooseScheme,
    readScheme,
    databaseImport,
    sources,
    options,
    planHeaders,
    checkIssues,
    checkRan,
    outputPath,
    outputDestination,
    phase,
    progress,
    progressLabel,
    databaseReady,
    busy,
    databaseLastResult,
    warnings,
    collapsedGroups,
    inputLabel,
    refreshPlan,
    clearMergedPreview,
    closeSourcePreview,
    openDatabaseTarget,
  } = context;
  const canStart = computed(() => {
    const ready =
      !busy.value &&
      sources.value.some((t) => t.enabled) &&
      planHeaders.value.length > 0;
    if (!ready) return false;
    return outputDestination.value === "xlsx"
      ? !!outputPath.value.trim()
      : databaseReady.value;
  });
  async function clearSources() {
    if (sources.value.length === 0) return;
    try {
      await ElMessageBox.confirm("确定清空所有数据源和输出位置？", "确认操作", {
        type: "warning",
      });
    } catch {
      return;
    }
    sources.value = [];
    warnings.value = [];
    clearMergedPreview();
    closeSourcePreview();
    checkIssues.value = [];
    checkRan.value = false;
    collapsedGroups.value = new Set();
    inputLabel.value = "尚未选择文件或文件夹，可直接拖放";
    planHeaders.value = [];
    outputPath.value = "";
    outputDestination.value = "xlsx";
    databaseImport.value = defaultDatabaseImportOptions();
    databaseLastResult.value = null;
    progress.value = 0;
    progressLabel.value = "";
    await refreshPlan();
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
  async function loadState() {
    await loadSettings();
    await loadDatabaseProfiles();
  }
  async function saveScheme() {
    const path = await persistScheme(sources.value, options.value);
    if (!path) return;
    progressLabel.value = `方案已保存：${path}`;
    try {
      await loadState();
    } catch (error) {
      ElMessage.error(`读取应用配置失败：${error}`);
    }
  }
  async function openScheme() {
    if (busy.value) return;
    const path = await chooseScheme();
    if (path) await openSchemeByPath(path);
  }
  async function openSchemeByPath(path: string) {
    if (busy.value) return;
    phase.value = "scanning";
    progressLabel.value = "正在重新读取方案中的数据源…";
    try {
      const scheme = await readScheme(path);
      sources.value = scheme.tables;
      options.value = scheme.options;
      clearMergedPreview();
      closeSourcePreview();
      inputLabel.value = `已打开方案：${path}`;
      checkIssues.value = [];
      checkRan.value = false;
      await loadState();
      await refreshPlan();
    } catch (error) {
      ElMessage.error(`打开方案失败：${error}`);
    } finally {
      phase.value = "ready";
    }
  }
  async function runPreflight(continuesMerge = false) {
    if (busy.value) return;
    phase.value = "checking";
    progress.value = 0;
    progressLabel.value = "正在执行处理前检查…";
    try {
      await invoke("run_preflight", {
        tables: sources.value,
        options: options.value,
        continuesMerge,
        destination: outputDestination.value,
      });
    } catch (error) {
      phase.value = "ready";
      progressLabel.value = "";
      ElMessage.error(String(error));
    }
  }
  async function confirmMerge() {
    phase.value = "merging";
    progress.value = 0;
    progressLabel.value =
      outputDestination.value === "xlsx"
        ? "正在准备输出工作簿…"
        : "正在连接 PostgreSQL…";
    try {
      if (outputDestination.value === "xlsx") {
        await invoke("start_merge", {
          tables: sources.value,
          options: options.value,
          output: outputPath.value,
        });
      } else {
        await invoke("start_database_import", {
          tables: sources.value,
          options: options.value,
          request: databaseImport.value,
        });
      }
    } catch (error) {
      phase.value = "ready";
      progressLabel.value = "";
      ElMessage.error(String(error));
    }
  }
  async function startMerge() {
    if (outputDestination.value === "xlsx") {
      outputPath.value = outputPath.value.trim();
      if (!outputPath.value) {
        ElMessage.error("请先选择输出文件");
        return;
      }
    }
    if (outputDestination.value === "postgres" && !databaseReady.value) {
      openDatabaseTarget();
      ElMessage.error("请先选择数据库连接并填写目标表");
      return;
    }
    await runPreflight(true);
  }
  async function cancelMerge() {
    if (phase.value !== "merging") return;
    try {
      await ElMessageBox.confirm(
        outputDestination.value === "xlsx"
          ? "确定取消当前合并？已写入的部分不会保留。"
          : "确定取消当前导入？数据库事务会回滚，不会保留部分数据。",
        "确认操作",
        { type: "warning" },
      );
    } catch {
      return;
    }
    // A native completion event can arrive while the confirmation dialog is open.
    if (phase.value !== "merging") return;
    const previousLabel = progressLabel.value;
    progressLabel.value = "正在取消…";
    try {
      await invoke("cancel_merge");
    } catch (error) {
      if (phase.value === "merging") progressLabel.value = previousLabel;
      ElMessage.error(String(error));
    }
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
    const logPath = await invoke("get_log_path");
    try {
      await revealItemInDir(logPath);
    } catch {
      ElMessage.error("日志文件尚未生成");
    }
  }
  async function registerSourceEvents(on: NativeOn) {
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
  async function registerEvents(on: NativeOn) {
    await registerSourceEvents(on);
    await on("database-import-finished", (e) => {
      databaseLastResult.value = e.payload;
      phase.value = "ready";
      progress.value = 1;
      progressLabel.value = "数据库导入完成";
      ElMessage.success(`已导入 ${formatNumber(e.payload.rows)} 行到 ${e.payload.target}`);
    });
    await on("merge-progress", (e) => {
      progress.value =
        e.payload.total === 0 ? 0 : e.payload.current / e.payload.total;
      progressLabel.value = `${e.payload.label}  ·  ${e.payload.current.toLocaleString()} / ${e.payload.total.toLocaleString()} 行`;
    });
    await on("merge-finished", (e) => {
      phase.value = "ready";
      progress.value = 1;
      progressLabel.value = "合并完成";
      ElMessage.success(
        `合并完成：${formatNumber(e.payload.rows)} 行，${e.payload.sheets} 个 Sheet`,
      );
      if (e.payload.truncated_cells > 0) {
        ElMessage.warning(
          `${formatNumber(e.payload.truncated_cells)} 个单元格超过 Excel 的 32,767 字符上限，已截断`,
        );
      }
      void revealOutput(e.payload.output);
    });
    await on("merge-cancelled", () => {
      phase.value = "ready";
      progressLabel.value = "已取消";
    });
    await on("merge-failed", (e) => {
      phase.value = "ready";
      progressLabel.value = "";
      ElMessage.error(`合并失败：${e.payload.message}`);
    });
    await on("preflight-done", async (e) => {
      checkIssues.value = e.payload.issues;
      checkRan.value = true;
      const errors = e.payload.issues.filter((i) => i.level === "Error");
      if (errors.length > 0) {
        phase.value = "ready";
        progressLabel.value = "";
        ElMessage.error(
          `处理前检查未通过：${errors[0].title} — ${errors[0].detail}`,
        );
      } else if (e.payload.continues_merge) {
        try {
          if (outputDestination.value === "xlsx") {
            const exists = await invoke("path_exists", {
              path: outputPath.value,
            });
            if (exists) {
              await ElMessageBox.confirm(
                `${outputPath.value} 已存在，是否覆盖？`,
                "覆盖已有文件",
                { type: "warning" },
              );
            }
          } else if (
            ["truncate", "replace"].includes(databaseImport.value.if_exists)
          ) {
            const action =
              databaseImport.value.if_exists === "replace"
                ? "删除并重建"
                : "清空";
            await ElMessageBox.confirm(
              `即将${action}数据表 ${databaseImport.value.schema}.${databaseImport.value.table}，是否继续？`,
              "确认数据库写入",
              { type: "warning", confirmButtonText: "确认并开始" },
            );
          }
        } catch {
          phase.value = "ready";
          return;
        }
        await confirmMerge();
      } else {
        phase.value = "ready";
        const warnCount = e.payload.issues.filter(
          (i) => i.level === "Warning",
        ).length;
        progressLabel.value = `检查完成：${errors.length} 个错误，${warnCount} 个提醒`;
      }
    });
  }
  return {
    canStart,
    scanFolder,
    scanFiles,
    applyGroupHeader,
    reloadTable,
    saveScheme,
    openScheme,
    openSchemeByPath,
    loadState,
    clearSources,
    runPreflight,
    confirmMerge,
    startMerge,
    cancelMerge,
    revealOutput,
    openLog,
    registerEvents,
  };
}
