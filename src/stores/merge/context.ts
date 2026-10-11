import type { Ref } from "vue";
import type { FieldGroup } from "./utils";
import type { createUpdateState } from "./update";
import type { createDatabaseState } from "./database";
import type { createSourcesState } from "./sources";
import type { createMappingState } from "./mapping";
import type { createPreviewState } from "./preview";
import type { createSchemeState } from "./scheme";
import type { createTaskState } from "./task";
export type AppPhase = "ready" | "scanning" | "checking" | "merging";
export interface WorkflowCallbacks {
    refreshPlan: () => Promise<void>;
    clearMergedPreview: () => void;
    closeSourcePreview: () => void;
    loadDatabaseProfiles: () => Promise<void>;
    loadState: () => Promise<void>;
    openDatabaseTarget: () => void;
    revealOutput: (output?: string) => Promise<void>;
}
export type MergeState = ReturnType<typeof createUpdateState> & ReturnType<typeof createDatabaseState> & ReturnType<typeof createSourcesState> & ReturnType<typeof createMappingState> & ReturnType<typeof createPreviewState> & ReturnType<typeof createSchemeState> & ReturnType<typeof createTaskState>;
export type MergeContext = MergeState & WorkflowCallbacks & {
    busy: Readonly<Ref<boolean>>;
    hasSources: Readonly<Ref<boolean>>;
    sourceFilterActive: Readonly<Ref<boolean>>;
    visibleSourceIndices: Readonly<Ref<number[]>>;
    visibleSourceCount: Readonly<Ref<number>>;
    visibleEnabledCount: Readonly<Ref<number>>;
    rowsMetric: Readonly<Ref<number>>;
    sheetsMetric: Readonly<Ref<number>>;
    databaseReady: Readonly<Ref<boolean>>;
    canStart: Readonly<Ref<boolean>>;
    fieldGroups: Readonly<Ref<FieldGroup[]>>;
};
