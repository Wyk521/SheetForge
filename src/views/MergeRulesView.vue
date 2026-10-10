<script setup lang="ts">
import { computed } from "vue";
import { useMergeStore } from "../stores/merge";
import FieldBatchView from "../components/FieldBatchView.vue";
import type { MergeMode } from "../types";

const store = useMergeStore();

const modes: { value: MergeMode; label: string }[] = [
  { value: "Manual", label: "修正表头" },
  { value: "Intersection", label: "列名交集" },
];

const mappingEnabled = computed(() => store.options.mode === "Manual");

function onModeChange(mode: MergeMode) {
  store.setMode(mode);
}
</script>

<template>
  <div class="sf-page sf-page-rules">
    <div class="sf-tabbar">
      <el-radio-group class="sf-mode-strip" :model-value="store.options.mode" size="small" aria-label="合并方式" @change="onModeChange">
        <el-radio-button v-for="mode in modes" :key="mode.value" :value="mode.value">
          {{ mode.label }}
        </el-radio-button>
      </el-radio-group>
    </div>

    <div class="sf-merge-layout">
      <!-- 左侧：高级选项 -->
      <div class="sf-merge-advanced">
        <el-card shadow="never">
          <div class="sf-option-stack">
            <div class="sf-option-row">
              <span>来源文件列</span>
              <el-switch
                :model-value="store.options.include_source_file"
                aria-label="来源文件列"
                @change="(v: boolean | string | number) => store.setAdvanced({ include_source_file: Boolean(v) })"
              />
            </div>
            <div class="sf-option-row">
              <span>来源工作表列</span>
              <el-switch
                :model-value="store.options.include_source_sheet"
                aria-label="来源工作表列"
                @change="(v: boolean | string | number) => store.setAdvanced({ include_source_sheet: Boolean(v) })"
              />
            </div>
            <div class="sf-option-row">
              <span>删除重复行</span>
              <el-switch
                :model-value="store.options.deduplicate"
                aria-label="删除重复行"
                @change="(v: boolean | string | number) => store.setAdvanced({ deduplicate: Boolean(v) })"
              />
            </div>
          </div>
        </el-card>

        <el-card shadow="never">
          <div class="sf-card-heading">行筛选</div>
          <div class="sf-filter-row">
            <el-input
              :model-value="store.options.filter_column"
              placeholder="字段名"
              class="sf-filter-column"
              @update:model-value="(v: string) => store.setAdvanced({ filter_column: v })"
            />
            <el-input
              :model-value="store.options.filter_text"
              placeholder="包含文本"
              @update:model-value="(v: string) => store.setAdvanced({ filter_text: v })"
            />
          </div>
          <el-checkbox
            :model-value="store.options.filter_exclude"
            @change="(v: boolean | string | number) => store.setAdvanced({ filter_exclude: Boolean(v) })"
          >
            排除匹配行
          </el-checkbox>
        </el-card>

      </div>

      <!-- 右侧：字段映射与输出顺序 -->
      <div class="sf-merge-mapping">
        <el-card shadow="never">
          <template v-if="mappingEnabled">
            <div class="sf-card-heading">字段映射</div>
            <FieldBatchView />
          </template>
          <template v-else>
            <div class="sf-card-heading">输出字段</div>
            <div v-if="store.planHeaders.length === 0" class="sf-inline-empty">
              暂无输出字段
            </div>
            <div v-else class="sf-output-order">
              <div
                v-for="(header, index) in store.planHeaders"
                :key="header"
                class="sf-output-order-row"
              >
                <span class="sf-output-order-name">
                  {{ header }}
                </span>
                <el-button
                  size="small"
                  :aria-label="`上移输出字段：${header}`"
                  :disabled="index === 0"
                  @click="store.moveOutputColumn(index, -1)"
                >
                  ↑
                </el-button>
                <el-button
                  size="small"
                  :aria-label="`下移输出字段：${header}`"
                  :disabled="index === store.planHeaders.length - 1"
                  @click="store.moveOutputColumn(index, 1)"
                >
                  ↓
                </el-button>
              </div>
            </div>
          </template>
        </el-card>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sf-merge-layout {
  display: flex;
  align-items: flex-start;
  gap: var(--space-md);
  min-width: 0;
}

.sf-merge-advanced {
  display: flex;
  flex: 0 1 280px;
  flex-direction: column;
  gap: var(--space-sm);
  min-width: 250px;
}

.sf-option-stack {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
}

.sf-option-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-md);
  min-height: 2rem;
  font-size: var(--text-sm);
}

.sf-card-heading {
  margin-bottom: var(--space-xs);
  font-weight: 700;
  font-size: var(--text-sm);
}

.sf-card-heading-spaced {
  margin-top: var(--space-sm);
}

.sf-filter-row {
  display: flex;
  gap: var(--space-xs);
  margin-bottom: var(--space-xs);
}

.sf-filter-column {
  width: 7rem;
}

.sf-inline-empty {
  color: var(--sf-text-muted);
  font-size: var(--text-xs);
}

.sf-output-order {
  display: flex;
  flex-direction: column;
  gap: var(--space-xs);
  margin-bottom: var(--space-md);
}

.sf-output-order-row {
  display: flex;
  align-items: center;
  gap: var(--space-xs);
  padding: var(--space-xs) var(--space-sm);
  border: var(--rule-hair) solid var(--color-rule-2);
  border-radius: var(--radius-control);
  background: var(--color-paper-2);
  color: var(--color-ink);
}

.sf-output-order-name {
  flex: 1;
  overflow: hidden;
  font-size: var(--text-sm);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sf-merge-mapping {
  flex: 1 1 0;
  min-width: 0;
}

@media (max-width: 1080px) {
  .sf-merge-layout {
    flex-direction: column;
  }

  .sf-merge-advanced,
  .sf-merge-mapping {
    width: 100%;
    min-width: 0;
  }
}
</style>
