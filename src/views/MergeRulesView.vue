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
    <div class="sf-page-head sf-rules-head">
      <div class="sf-page-heading-copy">
        <p class="sf-page-code"><span>02</span> 规则与字段</p>
        <h1 class="sf-page-title">合并工作区</h1>
        <p class="sf-page-description">选择合并方式，再核对输出字段与筛选条件。</p>
      </div>
    </div>

    <el-radio-group class="sf-mode-strip" :model-value="store.options.mode" size="large" @change="onModeChange">
      <el-radio-button v-for="mode in modes" :key="mode.value" :value="mode.value">
        {{ mode.label }}
      </el-radio-button>
    </el-radio-group>

    <div class="sf-merge-layout">
      <!-- 左侧：高级选项 -->
      <div class="sf-merge-advanced">
        <el-card shadow="never">
          <div class="sf-option-stack">
            <el-switch
              :model-value="store.options.include_source_file"
              inline-prompt
              active-text="记录来源文件"
              inactive-text="记录来源文件"
              @change="(v: boolean | string | number) => store.setAdvanced({ include_source_file: Boolean(v) })"
            />
            <div class="sf-option-note">输出中增加文件名列</div>
            <el-switch
              :model-value="store.options.include_source_sheet"
              inline-prompt
              active-text="记录来源工作表"
              inactive-text="记录来源工作表"
              @change="(v: boolean | string | number) => store.setAdvanced({ include_source_sheet: Boolean(v) })"
            />
            <div class="sf-option-note">输出中增加 Sheet 名列</div>
            <el-switch
              :model-value="store.options.deduplicate"
              inline-prompt
              active-text="删除重复行"
              inactive-text="删除重复行"
              @change="(v: boolean | string | number) => store.setAdvanced({ deduplicate: Boolean(v) })"
            />
            <div class="sf-option-note">按完整输出行去重</div>
          </div>
        </el-card>

        <el-card shadow="never">
          <div class="sf-card-heading">筛选（字段包含文本）</div>
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
            反向：排除匹配行
          </el-checkbox>
        </el-card>

      </div>

      <!-- 右侧：字段映射与输出顺序 -->
      <div class="sf-merge-mapping">
        <el-card shadow="never">
          <template v-if="mappingEnabled">
            <div class="sf-card-heading">字段映射与输出顺序</div>
            <div class="sf-card-description">
              按来源字段集中编辑；展开字段后点击“预览源表”即可在当前工作区查看源表样本。
            </div>
            <FieldBatchView />
          </template>
          <template v-else>
            <div class="sf-card-heading">输出字段顺序</div>
            <div class="sf-card-description">
              用上下按钮调整最终输出列顺序。
            </div>
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
            <div class="sf-mode-note">
              <div class="sf-mode-note-title">当前模式不修改字段</div>
              <div class="sf-mode-note-copy">
                交集直接使用原始表头；需要改名、纠错或清洗字段时，请切换到「修正表头」模式。
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

.sf-mode-strip {
  margin-bottom: var(--space-md);
}

.sf-option-stack {
  display: flex;
  flex-direction: column;
  gap: var(--space-sm);
}

.sf-option-note {
  margin-top: calc(var(--space-xs) * -1);
  color: var(--sf-text-muted);
  font-size: var(--text-2xs);
}

.sf-card-heading {
  margin-bottom: var(--space-xs);
  font-weight: 700;
  font-size: var(--text-sm);
}

.sf-card-heading-spaced {
  margin-top: var(--space-sm);
}

.sf-card-description {
  margin-bottom: var(--space-sm);
  color: var(--sf-text-muted);
  font-size: var(--text-xs);
  line-height: 1.55;
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

.sf-mode-note {
  padding: var(--space-md) var(--space-lg) var(--space-2xs);
  text-align: center;
}

.sf-mode-note-title {
  margin-bottom: var(--space-xs);
  font-weight: 700;
  font-size: var(--text-sm);
}

.sf-mode-note-copy {
  color: var(--sf-text-muted);
  font-size: var(--text-xs);
  line-height: 1.6;
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
