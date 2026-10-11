import { defineStore } from "pinia";
import { ref, computed, onScopeDispose } from "vue";
import { createEventScope } from "./event-scope";
import type { WorkflowCallbacks } from "./merge/context";
export type { AppPhase } from "./merge/context";
export { headerKey } from "./merge/utils";
export type { FieldGroup, FieldGroupSource } from "./merge/utils";
import { displayName, formatNumber } from "./merge/utils";
import { createUpdateState, createUpdateActions } from "./merge/update";
import { createDatabaseState, createDatabaseActions } from "./merge/database";
import { createSourcesState, createSourcesActions } from "./merge/sources";
import { createMappingState, createMappingActions } from "./merge/mapping";
import { createPreviewState, createPreviewActions } from "./merge/preview";
import { createSchemeState, createSchemeActions } from "./merge/scheme";
import { createTaskState, createTaskActions } from "./merge/task";
const MODE_LABELS: Record<string, string> = {
    Manual: "修正表头",
    Intersection: "列名交集",
};
export const useMergeStore = defineStore("merge", () => {
    const state = { ...createUpdateState(), ...createDatabaseState(), ...createSourcesState(), ...createMappingState(), ...createPreviewState(), ...createSchemeState(), ...createTaskState() };
    const { updateText, updateAvailable, updateDialogVisible, updateChecking, updateDownloading, updateProgress, updateStatus, databaseImport, databaseProfiles, databaseConfigPath, showDatabaseDialog, showDatabaseConnectionsDialog, databaseLastResult, sources, inputLabel, warnings, collapsedGroups, sourceSearch, options, mappingSearch, onlyMultiField, planHeaders, checkIssues, checkRan, preview, previewTitle, sourcePreview, sourcePreviewTitle, sourcePreviewVisible, sourcePreviewLoading, settings, outputPath, outputDestination, phase, progress, progressLabel } = state;
    const activePage = ref(0);
    const showAbout = ref(false);
    const busy = computed(() => phase.value !== "ready");
    const callbacks: WorkflowCallbacks = {
        refreshPlan: () => mapping.refreshPlan(),
        clearMergedPreview: () => previewDomain.clearMergedPreview(),
        closeSourcePreview: () => previewDomain.closeSourcePreview(),
        loadDatabaseProfiles: () => database.loadDatabaseProfiles(),
        loadState: () => scheme.loadState(),
        openDatabaseTarget: () => database.openDatabaseTarget(),
        revealOutput: (output) => task.revealOutput(output),
    };
    const context = { ...state, ...callbacks, busy };
    const update = createUpdateActions(context);
    const { checkUpdate, installUpdate } = update;
    const database = createDatabaseActions(context);
    const { databaseReady, loadDatabaseProfiles, saveDatabaseProfile, deleteDatabaseProfile, testDatabaseConnection, newDatabaseProfile, openDatabaseTarget, openDatabaseConnections } = database;
    const previewDomain = createPreviewActions(context);
    const { showSourcePreview, closeSourcePreview, showMergedPreview } = previewDomain;
    const mapping = createMappingActions(context);
    const { fieldGroups, refreshPlan, setMode, setAdvanced, moveOutputColumn, moveOutputColumnByName, setFieldTarget, setFieldEnabled, setFieldTransform, resetField } = mapping;
    const sourceDomain = createSourcesActions(context);
    const { hasSources, sourceFilterActive, visibleSourceCount, visibleEnabledCount, rowsMetric, sheetsMetric, chooseFolder, chooseFiles, scanFolder, scanFiles, clearSources, toggleSourceEnabled, selectAll, toggleGroup, setGroupEnabled, removeGroup, removeSource, applyGroupHeader, reloadTable } = sourceDomain;
    const scheme = createSchemeActions(context);
    const { saveScheme, openScheme, openSchemeByPath, loadState } = scheme;
    const task = createTaskActions({ ...state, ...callbacks, busy, databaseReady: database.databaseReady });
    const { canStart, runPreflight, startMerge, cancelMerge, revealOutput, openLog } = task;
    const eventScope = createEventScope();
    const disposeEvents = eventScope.dispose;
    onScopeDispose(disposeEvents);
    function initEvents() {
        return eventScope.init(async (on) => {
            await update.registerEvents(on);
            await database.registerEvents(on);
            await sourceDomain.registerEvents(on);
            await task.registerEvents(on);
        });
    }
    return {
        // state
        sources, options, outputPath, outputDestination, databaseImport, databaseProfiles,
        databaseConfigPath, databaseLastResult, showDatabaseDialog, showDatabaseConnectionsDialog,
        inputLabel, phase, progress, progressLabel, warnings,
        checkIssues, checkRan, preview, previewTitle, settings, updateText,
        updateAvailable, updateDialogVisible, updateChecking, updateDownloading, updateProgress, updateStatus,
        sourcePreview, sourcePreviewTitle, sourcePreviewVisible, sourcePreviewLoading,
        collapsedGroups, sourceSearch, sourceFilterActive, visibleSourceCount, visibleEnabledCount,
        mappingSearch, onlyMultiField, planHeaders,
        activePage, showAbout,
        // getters
        busy, hasSources, rowsMetric, sheetsMetric, databaseReady, canStart, fieldGroups, formatNumber,
        // actions
        chooseFolder, chooseFiles, scanFolder, scanFiles, clearSources,
        toggleSourceEnabled, selectAll, toggleGroup, setGroupEnabled, removeGroup, removeSource,
        applyGroupHeader, reloadTable,
        setMode, setAdvanced, moveOutputColumn, moveOutputColumnByName,
        setFieldTarget, setFieldEnabled, setFieldTransform, resetField,
        showSourcePreview, closeSourcePreview, showMergedPreview, runPreflight, startMerge, cancelMerge,
        loadDatabaseProfiles, saveDatabaseProfile, deleteDatabaseProfile, testDatabaseConnection,
        newDatabaseProfile, openDatabaseTarget, openDatabaseConnections,
        saveScheme, openScheme, openSchemeByPath, checkUpdate, installUpdate, loadState, initEvents, disposeEvents, revealOutput, openLog, refreshPlan,
        displayName, MODE_LABELS,
    };
});
