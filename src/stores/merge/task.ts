import { ref, computed } from "vue";
import { invoke } from "../../native";
import { ElMessage, ElMessageBox } from "element-plus";
import {
  defaultDatabaseImportOptions,
  type OutputDestination,
} from "../../types";
import { formatNumber } from "./utils";
import type { MergeContext, AppPhase } from "./context";
import type { NativeOn } from "../../native";
export function createTaskState() {
  const outputPath = ref("");
  const outputDestination = ref<OutputDestination>("xlsx");
  const phase = ref<AppPhase>("ready");
  const progress = ref(0);
  const progressLabel = ref("");
  return { outputPath, outputDestination, phase, progress, progressLabel };
}
export function createTaskActions(
  context: Pick<
    MergeContext,
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
    if (outputDestination.value === "xlsx" && !outputPath.value.trim()) {
      ElMessage.error("请先选择输出文件");
      return;
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
  async function registerEvents(on: NativeOn) {
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
