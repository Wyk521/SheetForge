<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { useMergeStore } from "../stores/merge";
import type { ConnectionInfo, ConnectionProfile } from "../types";

const store = useMergeStore();
const editingName = ref("");
const isExisting = ref(false);
const password = ref("");
const rememberPassword = ref(true);
const testing = ref(false);
const saving = ref(false);
const connectionInfo = ref<ConnectionInfo | null>(null);
const profile = reactive<ConnectionProfile>(store.newDatabaseProfile());

const profileNames = computed(() => Object.keys(store.databaseProfiles));

function copyProfile(value: ConnectionProfile) {
  Object.assign(profile, value);
}

function selectProfile(name: string) {
  const selected = store.databaseProfiles[name];
  if (!selected) return;
  editingName.value = name;
  isExisting.value = true;
  password.value = "";
  connectionInfo.value = null;
  copyProfile(selected);
  store.databaseImport.profile_name = name;
  store.databaseImport.password = "";
}

function beginNew() {
  editingName.value = "";
  isExisting.value = false;
  password.value = "";
  connectionInfo.value = null;
  copyProfile(store.newDatabaseProfile());
}

watch(
  () => store.showDatabaseConnectionsDialog,
  async (visible) => {
    if (!visible) return;
    await store.loadDatabaseProfiles();
    const selected = store.databaseImport.profile_name;
    if (selected && store.databaseProfiles[selected]) selectProfile(selected);
    else if (profileNames.value.length > 0) selectProfile(profileNames.value[0]);
    else beginNew();
  }
);

async function testConnection() {
  testing.value = true;
  connectionInfo.value = null;
  try {
    connectionInfo.value = await store.testDatabaseConnection(
      isExisting.value ? editingName.value : undefined,
      { ...profile },
      password.value
    );
    ElMessage.success(`连接成功，耗时 ${connectionInfo.value.elapsed_ms} 毫秒`);
  } catch (error) {
    ElMessage.error(`连接失败：${error}`);
  } finally {
    testing.value = false;
  }
}

async function saveProfile() {
  const name = editingName.value.trim();
  if (!name) {
    ElMessage.error("请输入连接名称");
    return;
  }
  if (!profile.host.trim() || !profile.database.trim() || !profile.user.trim()) {
    ElMessage.error("请完整填写主机、数据库和用户名");
    return;
  }
  saving.value = true;
  try {
    const warning = await store.saveDatabaseProfile(
      name,
      { ...profile },
      password.value,
      rememberPassword.value
    );
    editingName.value = name;
    isExisting.value = true;
    // 未保存到系统凭据时，密码仅在本次应用运行期间用于该连接。
    store.databaseImport.password = password.value;
    store.databaseImport.remember_password = rememberPassword.value;
    if (warning) ElMessage.warning(warning);
    else ElMessage.success("数据库连接已保存；密码未写入配置文件");
  } catch (error) {
    ElMessage.error(`保存失败：${error}`);
  } finally {
    saving.value = false;
  }
}

async function deleteProfile() {
  if (!isExisting.value) return;
  try {
    await ElMessageBox.confirm(`确定删除连接“${editingName.value}”？`, "删除连接", {
      type: "warning",
    });
    await store.deleteDatabaseProfile(editingName.value);
    if (profileNames.value.length > 0) selectProfile(profileNames.value[0]);
    else beginNew();
    ElMessage.success("连接已删除");
  } catch (error) {
    if (String(error) !== "cancel") ElMessage.error(`删除失败：${error}`);
  }
}

function finishManagement() {
  store.showDatabaseConnectionsDialog = false;
  if (store.outputDestination === "postgres" && profileNames.value.length > 0) {
    store.openDatabaseTarget();
  }
}
</script>

<template>
  <el-dialog
    v-model="store.showDatabaseConnectionsDialog"
    title="数据库连接"
    width="min(860px, 92vw)"
    top="7vh"
    destroy-on-close
    :close-on-click-modal="false"
  >
    <div class="db-dialog-grid">
      <aside class="db-profile-list">
        <div class="db-section-title">已保存连接</div>
        <button
          v-for="name in profileNames"
          :key="name"
          class="db-profile-item"
          :class="{ active: isExisting && editingName === name }"
          type="button"
          @click="selectProfile(name)"
        >
          <strong>{{ name }}</strong>
          <small>
            {{ store.databaseProfiles[name].user }}@{{ store.databaseProfiles[name].host }}:{{ store.databaseProfiles[name].port }}
          </small>
        </button>
        <el-empty v-if="profileNames.length === 0" description="还没有连接" :image-size="54" />
        <el-button class="db-new-button" @click="beginNew">＋ 新建连接</el-button>
        <div class="db-config-path">配置文件<br />{{ store.databaseConfigPath }}</div>
      </aside>

      <section class="db-form-area">
        <div class="db-section-heading">
          <div>
            <b>{{ isExisting ? `编辑连接：${editingName}` : "新建连接" }}</b>
            <span>连接参数沿用 pg-table-importer；密码只进入系统凭据管理器</span>
          </div>
          <el-button v-if="isExisting" text type="danger" @click="deleteProfile">删除连接</el-button>
        </div>
        <el-form label-position="top" class="db-form">
          <el-form-item label="连接名称">
            <el-input v-model="editingName" :disabled="isExisting" placeholder="例如：生产库" />
          </el-form-item>
          <el-form-item label="主机">
            <el-input v-model="profile.host" placeholder="localhost" />
          </el-form-item>
          <el-form-item label="端口">
            <el-input-number v-model="profile.port" :min="1" :max="65535" controls-position="right" class="db-full-control" />
          </el-form-item>
          <el-form-item label="数据库">
            <el-input v-model="profile.database" placeholder="postgres" />
          </el-form-item>
          <el-form-item label="用户名">
            <el-input v-model="profile.user" placeholder="postgres" />
          </el-form-item>
          <el-form-item label="SSL 模式">
            <el-select v-model="profile.sslmode" class="db-full-control">
              <el-option label="allow（先尝试非 SSL）" value="allow" />
              <el-option label="prefer（优先 SSL）" value="prefer" />
              <el-option label="require（必须 SSL）" value="require" />
              <el-option label="verify-ca（校验证书链）" value="verify-ca" />
              <el-option label="verify-full（校验证书链与主机名）" value="verify-full" />
              <el-option label="disable（不使用 SSL）" value="disable" />
            </el-select>
          </el-form-item>
          <el-form-item label="密码" class="db-password-field">
            <el-input v-model="password" type="password" show-password placeholder="留空则读取已保存凭据或 PGPASSWORD" />
            <el-checkbox v-model="rememberPassword">保存到操作系统凭据管理器</el-checkbox>
          </el-form-item>
        </el-form>
        <div class="db-actions">
          <el-button :loading="testing" @click="testConnection">测试连接</el-button>
          <el-button type="primary" plain :loading="saving" @click="saveProfile">保存连接</el-button>
          <span v-if="connectionInfo" class="db-test-ok">
            ✓ {{ connectionInfo.user }}@{{ connectionInfo.database }} · {{ connectionInfo.elapsed_ms }} ms
          </span>
        </div>
      </section>
    </div>

    <template #footer>
      <el-button type="primary" @click="finishManagement">完成</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
:deep(.el-dialog__body) { max-height: calc(86vh - 8rem); overflow-y: auto; }
.db-dialog-grid { display: grid; grid-template-columns: 13rem minmax(0, 1fr); gap: var(--space-lg); min-height: 22rem; }
.db-profile-list { border-right: var(--rule-hair) solid var(--sf-border); padding-right: var(--space-md); }
.db-section-title { margin-bottom: var(--space-xs); font-size: var(--text-xs); font-weight: 700; }
.db-profile-item { width: 100%; margin-bottom: var(--space-2xs); padding: var(--space-xs) var(--space-sm); border: var(--rule-hair) solid transparent; border-radius: var(--radius-control); background: transparent; color: var(--sf-text); cursor: pointer; text-align: left; }
.db-profile-item.active { border-color: var(--el-color-primary-light-5); background: var(--el-color-primary-light-9); }
.db-profile-item:active { transform: translateY(1px); }
.db-profile-item strong, .db-profile-item small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.db-profile-item small { margin-top: var(--space-3xs); color: var(--sf-text-muted); }
.db-new-button { width: 100%; margin-top: var(--space-xs); }
.db-config-path { margin-top: var(--space-md); color: var(--sf-text-muted); font-size: var(--text-2xs); line-height: 1.45; word-break: break-all; }
.db-form-area { min-width: 0; }
.db-section-heading { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: var(--space-sm); }
.db-section-heading b, .db-section-heading span { display: block; }
.db-section-heading b { font-size: var(--text-base); }
.db-section-heading span { margin-top: var(--space-3xs); color: var(--sf-text-muted); font-size: var(--text-2xs); }
.db-form { display: grid; grid-template-columns: 1.25fr 0.8fr 1fr; gap: 0 var(--space-sm); }
.db-password-field { grid-column: span 2; }
.db-password-field :deep(.el-form-item__content) { display: block; }
.db-password-field .el-checkbox { margin-top: var(--space-2xs); }
.db-actions { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-xs); }
.db-full-control { width: 100%; }
.db-test-ok { color: var(--color-success); font-size: var(--text-2xs); }

@media (hover: hover) and (pointer: fine) {
  .db-profile-item:hover { background: var(--el-color-primary-light-9); }
}

@media (max-width: 48rem) {
  .db-dialog-grid,
  .db-form { grid-template-columns: minmax(0, 1fr); }
  .db-profile-list { padding-right: 0; padding-bottom: var(--space-md); border-right: 0; border-bottom: var(--rule-hair) solid var(--sf-border); }
  .db-password-field { grid-column: auto; }
  .db-section-heading { flex-direction: column; gap: var(--space-xs); }
  .db-test-ok { flex: 1 1 100%; }
}
</style>
