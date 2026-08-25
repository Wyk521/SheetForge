<script setup lang="ts">
import { computed, watch } from "vue";
import { ElMessage } from "element-plus";
import { useMergeStore } from "../stores/merge";

const store = useMergeStore();

const profileNames = computed(() => Object.keys(store.databaseProfiles));
const selectedProfile = computed(
  () => store.databaseProfiles[store.databaseImport.profile_name]
);

watch(
  () => store.showDatabaseDialog,
  async (visible) => {
    if (!visible) return;
    await store.loadDatabaseProfiles();
    if (profileNames.value.length === 0) {
      store.showDatabaseDialog = false;
      store.openDatabaseConnections();
      return;
    }
    if (!store.databaseProfiles[store.databaseImport.profile_name]) {
      store.databaseImport.profile_name = profileNames.value[0];
    }
  }
);

function changeProfile() {
  // 一次性密码只属于原连接，切换连接后不能继续沿用。
  store.databaseImport.password = "";
}

function manageConnections() {
  store.showDatabaseDialog = false;
  store.openDatabaseConnections();
}

function finish() {
  if (!store.databaseImport.profile_name) {
    ElMessage.error("请选择数据库连接");
    return;
  }
  if (!store.databaseImport.schema.trim() || !store.databaseImport.table.trim()) {
    ElMessage.error("请填写目标 Schema 和数据表名称");
    return;
  }
  store.showDatabaseDialog = false;
}
</script>

<template>
  <el-dialog
    v-model="store.showDatabaseDialog"
    title="PostgreSQL 本次导入目标"
    width="760px"
    top="10vh"
    destroy-on-close
    :close-on-click-modal="false"
  >
    <section class="db-connection-picker">
      <div class="db-section-heading">
        <div>
          <b>使用已保存的连接</b>
          <span>这里只选择连接，不会修改连接参数</span>
        </div>
        <el-button @click="manageConnections">管理连接…</el-button>
      </div>
      <el-select
        v-model="store.databaseImport.profile_name"
        size="large"
        style="width: 100%"
        placeholder="请选择连接"
        @change="changeProfile"
      >
        <el-option
          v-for="name in profileNames"
          :key="name"
          :label="`${name} — ${store.databaseProfiles[name].user}@${store.databaseProfiles[name].host}:${store.databaseProfiles[name].port}/${store.databaseProfiles[name].database}`"
          :value="name"
        />
      </el-select>
      <div v-if="selectedProfile" class="db-selected-summary">
        当前连接：{{ selectedProfile.user }}@{{ selectedProfile.host }}:{{ selectedProfile.port }}/{{ selectedProfile.database }}
      </div>
    </section>

    <el-divider />

    <section>
      <div class="db-section-heading">
        <div>
          <b>本次导入目标</b>
          <span>字段将全部创建为 TEXT NULL，保留前导零和超长编号</span>
        </div>
      </div>
      <el-form label-position="top" class="db-target-form">
        <el-form-item label="Schema">
          <el-input v-model="store.databaseImport.schema" placeholder="public" />
        </el-form-item>
        <el-form-item label="数据表">
          <el-input v-model="store.databaseImport.table" placeholder="例如：customer_data" />
        </el-form-item>
        <el-form-item label="表已存在时">
          <el-select v-model="store.databaseImport.if_exists" style="width: 100%">
            <el-option label="停止（最安全）" value="abort" />
            <el-option label="追加数据" value="append" />
            <el-option label="清空后导入" value="truncate" />
            <el-option label="删除并重建" value="replace" />
          </el-select>
        </el-form-item>
        <el-form-item label="COPY 格式">
          <el-select v-model="store.databaseImport.copy_format" style="width: 100%">
            <el-option label="二进制（推荐）" value="binary" />
            <el-option label="文本" value="text" />
          </el-select>
        </el-form-item>
      </el-form>
      <div class="db-switches">
        <el-checkbox v-model="store.databaseImport.empty_as_null">空字符串按 NULL 导入</el-checkbox>
        <el-checkbox v-model="store.databaseImport.fast_mode">快速提交（服务器崩溃时最近提交可能丢失）</el-checkbox>
        <el-checkbox
          :model-value="store.databaseImport.table_persistence === 'unlogged'"
          @change="(v: boolean | string | number) => (store.databaseImport.table_persistence = v ? 'unlogged' : 'logged')"
        >
          新表使用 UNLOGGED（崩溃会清空且不复制到备库）
        </el-checkbox>
      </div>
      <el-alert
        v-if="['truncate', 'replace'].includes(store.databaseImport.if_exists)"
        title="该策略会修改或删除目标表中的现有数据，开始前还会再次确认。"
        type="warning"
        :closable="false"
        show-icon
        style="margin-top: 12px"
      />
    </section>

    <template #footer>
      <el-button @click="store.showDatabaseDialog = false">取消</el-button>
      <el-button type="primary" @click="finish">使用此目标</el-button>
    </template>
  </el-dialog>
</template>

<style scoped>
.db-section-heading { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 12px; }
.db-section-heading b, .db-section-heading span { display: block; }
.db-section-heading b { font-size: 14px; }
.db-section-heading span { font-size: 11px; color: var(--sf-text-muted); margin-top: 3px; }
.db-selected-summary { color: var(--sf-text-muted); font-size: 11px; margin-top: 7px; }
.db-target-form { display: grid; grid-template-columns: 0.75fr 1.25fr 1fr 1fr; gap: 0 12px; }
.db-switches { display: flex; flex-wrap: wrap; column-gap: 18px; row-gap: 4px; }
</style>
