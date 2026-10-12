import { ref, computed } from "vue";
import { invoke } from "../../native";
import { ElMessage } from "element-plus";
import {
  defaultConnectionProfile,
  defaultDatabaseImportOptions,
  type ConnectionInfo,
  type ConnectionProfile,
  type DatabaseImportFinished,
  type DatabaseImportOptions,
} from "../../types";
import type { MergeContext } from "./context";
export function createDatabaseState() {
  const databaseImport = ref<DatabaseImportOptions>(
    defaultDatabaseImportOptions(),
  );
  const databaseProfiles = ref<Record<string, ConnectionProfile>>({});
  const databaseConfigPath = ref("");
  const showDatabaseDialog = ref(false);
  const showDatabaseConnectionsDialog = ref(false);
  const databaseLastResult = ref<DatabaseImportFinished | null>(null);
  return {
    databaseImport,
    databaseProfiles,
    databaseConfigPath,
    showDatabaseDialog,
    showDatabaseConnectionsDialog,
    databaseLastResult,
  };
}
export function createDatabaseActions(
  context: Pick<
    MergeContext,
    | "databaseImport"
    | "databaseProfiles"
    | "databaseConfigPath"
    | "showDatabaseDialog"
    | "showDatabaseConnectionsDialog"
  >,
) {
  const {
    databaseImport,
    databaseProfiles,
    databaseConfigPath,
    showDatabaseDialog,
    showDatabaseConnectionsDialog,
  } = context;
  const databaseReady = computed(
    () =>
      !!databaseImport.value.profile_name &&
      !!databaseProfiles.value[databaseImport.value.profile_name] &&
      !!databaseImport.value.schema.trim() &&
      !!databaseImport.value.table.trim(),
  );
  async function loadDatabaseProfiles() {
    try {
      const result = await invoke("get_database_profiles");
      databaseProfiles.value = result.profiles;
      databaseConfigPath.value = result.config_path;
      if (
        databaseImport.value.profile_name &&
        !databaseProfiles.value[databaseImport.value.profile_name]
      ) {
        databaseImport.value.profile_name = "";
      }
      if (!databaseImport.value.profile_name) {
        databaseImport.value.profile_name =
          Object.keys(databaseProfiles.value)[0] ?? "";
      }
    } catch (error) {
      databaseProfiles.value = {};
      ElMessage.error(`读取数据库连接配置失败：${error}`);
    }
  }
  async function saveDatabaseProfile(
    name: string,
    profile: ConnectionProfile,
    password: string,
    rememberPassword: boolean,
  ): Promise<string | null> {
    const warning = await invoke("save_database_profile", {
      name,
      profile,
      password: password || null,
      rememberPassword,
    });
    await loadDatabaseProfiles();
    databaseImport.value.profile_name = name.trim();
    return warning;
  }
  async function deleteDatabaseProfile(name: string) {
    await invoke("delete_database_profile", { name });
    if (databaseImport.value.profile_name === name)
      databaseImport.value.profile_name = "";
    await loadDatabaseProfiles();
  }
  async function testDatabaseConnection(
    profileName: string | undefined,
    profile: ConnectionProfile,
    password: string,
  ): Promise<ConnectionInfo> {
    return invoke("test_database_connection", {
      profileName: profileName || null,
      profile,
      password: password || null,
    });
  }
  function newDatabaseProfile(): ConnectionProfile {
    return defaultConnectionProfile();
  }
  function openDatabaseTarget() {
    if (Object.keys(databaseProfiles.value).length === 0) {
      showDatabaseConnectionsDialog.value = true;
      return;
    }
    showDatabaseDialog.value = true;
  }
  function openDatabaseConnections() {
    showDatabaseConnectionsDialog.value = true;
  }
  return {
    databaseReady,
    loadDatabaseProfiles,
    saveDatabaseProfile,
    deleteDatabaseProfile,
    testDatabaseConnection,
    newDatabaseProfile,
    openDatabaseTarget,
    openDatabaseConnections,
  };
}
