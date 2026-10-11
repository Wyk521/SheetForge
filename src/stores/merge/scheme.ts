import { ref } from "vue";
import { invoke } from "../../native";
import { open, save } from "@tauri-apps/plugin-dialog";
import { ElMessage } from "element-plus";
import { type AppSettings } from "../../types";
import type { MergeContext } from "./context";
export function createSchemeState() {
    const settings = ref<AppSettings | null>(null);
    return { settings };
}
export function createSchemeActions(context: Pick<MergeContext, "sources" | "inputLabel" | "options" | "checkIssues" | "checkRan" | "settings" | "phase" | "progressLabel" | "busy" | "refreshPlan" | "clearMergedPreview" | "closeSourcePreview" | "loadDatabaseProfiles">) {
    const { sources, inputLabel, options, checkIssues, checkRan, settings, phase, progressLabel, busy, refreshPlan, clearMergedPreview, closeSourcePreview, loadDatabaseProfiles } = context;
    async function saveScheme() {
        const path = await save({
            title: "保存合并方案",
            defaultPath: "合并方案.json",
            filters: [{ name: "表格合并方案", extensions: ["json"] }],
        });
        if (!path)
            return;
        try {
            await invoke("save_scheme", {
                path: String(path),
                tables: sources.value,
                options: options.value,
            });
            progressLabel.value = `方案已保存：${path}`;
            ElMessage.success("方案已保存");
            await loadState();
        }
        catch (error) {
            ElMessage.error(`保存方案失败：${error}`);
        }
    }
    async function openScheme() {
        if (busy.value)
            return;
        const path = await open({
            title: "打开合并方案",
            multiple: false,
            filters: [{ name: "表格合并方案", extensions: ["json"] }],
        });
        if (!path)
            return;
        await openSchemeByPath(String(path));
    }
    async function openSchemeByPath(path: string) {
        if (busy.value)
            return;
        phase.value = "scanning";
        progressLabel.value = "正在重新读取方案中的数据源…";
        try {
            const scheme = await invoke("open_scheme", { path });
            sources.value = scheme.tables;
            options.value = scheme.options;
            clearMergedPreview();
            closeSourcePreview();
            inputLabel.value = `已打开方案：${path}`;
            checkIssues.value = [];
            checkRan.value = false;
            await loadState();
            await refreshPlan();
        }
        catch (error) {
            ElMessage.error(`打开方案失败：${error}`);
        }
        finally {
            phase.value = "ready";
        }
    }
    async function loadState() {
        const state = await invoke("get_state");
        settings.value = state.settings;
        await loadDatabaseProfiles();
    }
    return { saveScheme, openScheme, openSchemeByPath, loadState };
}
