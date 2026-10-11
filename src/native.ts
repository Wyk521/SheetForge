import { invoke as nativeInvoke } from "@tauri-apps/api/core";
import type { EventCallback } from "@tauri-apps/api/event";
import type * as T from "./types";
type Request<A, R = void> = {
    args: A;
    result: R;
};
type Rules = {
    tables: T.SourceTable[];
    options: T.MergeOptions;
};
/** The wire contract is kept here; pages and domains do not choose response types. */
export interface NativeCommands {
    save_text_file: Request<{ path: string; content: string }>;
    get_state: Request<undefined, {
        settings: T.AppSettings;
    }>;
    get_plan: Request<Rules, {
        headers: string[];
    }>;
    get_log_path: Request<undefined, string>;
    path_exists: Request<{
        path: string;
    }, boolean>;
    get_database_profiles: Request<undefined, T.DatabaseProfiles>;
    save_database_profile: Request<{
        name: string;
        profile: T.ConnectionProfile;
        password: string | null;
        rememberPassword: boolean;
    }, string | null>;
    delete_database_profile: Request<{
        name: string;
    }>;
    test_database_connection: Request<{
        profileName: string | null;
        profile: T.ConnectionProfile;
        password: string | null;
    }, T.ConnectionInfo>;
    scan_folder: Request<{
        path: string;
    }>;
    scan_files: Request<{
        paths: string[];
    }>;
    reload_table: Request<{
        index: number;
        table: T.SourceTable;
        headerRow: number;
        headerRows: number;
    }>;
    reload_group: Request<{
        sources: {
            index: number;
            table: T.SourceTable;
        }[];
        headerRow: number;
        headerRows: number;
    }>;
    preview_source: Request<{
        table: T.SourceTable;
        limit: number;
    }, T.PreviewTable>;
    preview_merged: Request<Rules & {
        limit: number;
    }, T.MergedPreview>;
    run_preflight: Request<Rules & {
        continuesMerge: boolean;
        destination: T.OutputDestination;
    }>;
    start_merge: Request<Rules & {
        output: string;
    }>;
    start_database_import: Request<Rules & {
        request: T.DatabaseImportOptions;
    }>;
    cancel_merge: Request<undefined>;
    save_scheme: Request<Rules & {
        path: string;
    }>;
    open_scheme: Request<{
        path: string;
    }, T.MergeScheme>;
    check_update: Request<undefined>;
    install_update: Request<{
        update: T.UpdateResult;
    }>;
}
export function invoke<K extends keyof NativeCommands>(command: K, ...args: NativeCommands[K]["args"] extends undefined ? [
    args?: undefined
] : [
    args: NativeCommands[K]["args"]
]): Promise<NativeCommands[K]["result"]> {
    return nativeInvoke(command, args[0]);
}
export interface NativeEvents {
    "scan-progress": T.ScanProgress;
    "scan-finished": T.ScanFinished;
    "scan-failed": {
        message: string;
    };
    "table-reloaded": T.TableReloaded;
    "tables-reloaded": T.TablesReloaded;
    "table-reload-failed": {
        index: number;
        message: string;
    };
    "merge-progress": T.MergeProgress;
    "merge-finished": T.MergeFinished;
    "merge-cancelled": undefined;
    "merge-failed": {
        message: string;
    };
    "database-import-finished": T.DatabaseImportFinished;
    "preflight-done": T.PreflightDone;
    "update-result": T.UpdateResult;
    "update-download-progress": T.UpdateDownloadProgress;
    "update-installing": undefined;
    "update-failed": {
        message: string;
    };
}
export type NativeOn = <K extends keyof NativeEvents>(name: K, callback: EventCallback<NativeEvents[K]>) => Promise<void>;
