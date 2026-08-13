<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useMergeStore } from "../stores/merge";
import type { MergeMode } from "../types";

const store = useMergeStore();

const TRANSFORMS = ["None", "Trim", "Uppercase", "Lowercase"];
const AGGREGATES = ["First", "Sum", "UniqueJoin", "TextJoin"];

function transformIndex(value: string): number {
  return TRANSFORMS.indexOf(value);
}
function aggregateIndex(value: string): number {
  return AGGREGATES.indexOf(value);
}

const modes: { value: MergeMode; label: string }[] = [
  { value: "Union", label: "列名并集" },
  { value: "Intersection", label: "列名交集" },
  { value: "Manual", label: "修正表头" },
  { value: "Consolidate", label: "按键汇总" },
  { value: "Join", label: "横向关联" },
];

const mappingEnabled = computed(
  () =>
    store.options.mode === "Manual" ||
    store.options.mode === "Consolidate" ||
    store.options.mode === "Join"
);

const enabledList = computed(() =>
  store.sources
    .map((table, index) => ({ index, table }))
    .filter((item) => item.table.enabled)
);

const table = computed(() => store.selectedTable());

const commonKeys = computed(() => store.planCommonKeys);

const suggestions = ref<Record<string, string>>({});
watch(
  () => [store.sources, store.options.mode] as const,
  async () => {
    try {
      suggestions.value = await invoke<Record<string, string>>("get_suggestions", {
        tables: store.sources,
      });
    } catch {
      suggestions.value = {};
    }
  },
  { immediate: true }
);

const mappingRows = computed(() => {
  const t = table.value;
  if (!t) return [];
  const search = store.mappingSearch.toLowerCase();
  return t.mappings
    .map((m, index) => ({ m, index, suggestion: suggestions.value[m.source_name] ?? "" }))
    .filter((row) => {
      const commonField = commonKeys.value.has(row.m.source_name.trim().toLowerCase());
      const differs =
        row.m.source_name.trim().toLowerCase() !== row.m.target_name.trim().toLowerCase() ||
        !!row.suggestion;
      if (store.hideCommonMappings && commonField) return false;
      if (store.mismatchOnly && !differs) return false;
      if (
        search &&
        !row.m.source_name.toLowerCase().includes(search) &&
        !row.m.target_name.toLowerCase().includes(search)
      ) {
        return false;
      }
      return true;
    });
});

function onModeChange(mode: MergeMode) {
  store.setMode(mode);
  store.selectedMappingTable = store.enabledIndices[0] ?? 0;
}

function updateTableSelection(index: number) {
  store.selectedMappingTable = index;
}
</script>

<template>
  <div>
    <h1 style="font-size: 18px; font-weight: 600; margin: 0 0 14px">设置合并规则</h1>

    <el-radio-group :model-value="store.options.mode" size="large" style="margin-bottom: 14px" @change="onModeChange">
      <el-radio-button v-for="mode in modes" :key="mode.value" :value="mode.value">
        {{ mode.label }}
      </el-radio-button>
    </el-radio-group>

    <div style="display: flex; gap: 14px; align-items: flex-start">
      <!-- 左侧：高级选项 -->
      <div style="width: 330px; flex-shrink: 0; display: flex; flex-direction: column; gap: 10px">
        <el-card shadow="never">
          <div style="display: flex; flex-direction: column; gap: 10px">
            <el-switch
              :model-value="store.options.include_source_file"
              inline-prompt
              active-text="记录来源文件"
              inactive-text="记录来源文件"
              @change="(v: boolean | string | number) => store.setAdvanced({ include_source_file: Boolean(v) })"
            />
            <div style="font-size: 10.5px; color: var(--sf-text-muted); margin-top: -6px">输出中增加文件名列</div>
            <el-switch
              :model-value="store.options.include_source_sheet"
              inline-prompt
              active-text="记录来源工作表"
              inactive-text="记录来源工作表"
              @change="(v: boolean | string | number) => store.setAdvanced({ include_source_sheet: Boolean(v) })"
            />
            <div style="font-size: 10.5px; color: var(--sf-text-muted); margin-top: -6px">输出中增加 Sheet 名列</div>
            <el-switch
              :model-value="store.options.deduplicate"
              inline-prompt
              active-text="删除重复行"
              inactive-text="删除重复行"
              @change="(v: boolean | string | number) => store.setAdvanced({ deduplicate: Boolean(v) })"
            />
            <div style="font-size: 10.5px; color: var(--sf-text-muted); margin-top: -6px">
              有键字段时按键，否则按整行
            </div>
          </div>
        </el-card>

        <el-card shadow="never">
          <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">键字段（多个用英文逗号分隔）</div>
          <el-input
            :model-value="store.options.key_columns.join(', ')"
            placeholder="例如：订单号, 日期"
            @update:model-value="(v: string) => store.setAdvanced({ key_columns: v.split(/[,，]/).map((s) => s.trim()).filter(Boolean) })"
          />
          <template v-if="store.options.mode === 'Join'">
            <div style="font-size: 12px; font-weight: 600; margin: 12px 0 8px">关联方式</div>
            <el-radio-group
              :model-value="store.options.join_kind"
              @change="(v: string | number | boolean | undefined) => store.setAdvanced({ join_kind: String(v) as never })"
            >
              <el-radio-button value="Left">左关联</el-radio-button>
              <el-radio-button value="Inner">内关联</el-radio-button>
              <el-radio-button value="Full">全关联</el-radio-button>
            </el-radio-group>
          </template>
        </el-card>

        <el-card shadow="never">
          <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">筛选（字段包含文本）</div>
          <div style="display: flex; gap: 8px; margin-bottom: 8px">
            <el-input
              :model-value="store.options.filter_column"
              placeholder="字段名"
              style="width: 110px"
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

        <el-card shadow="never" style="flex: 1">
          <div style="font-size: 12px; font-weight: 600; margin-bottom: 8px">输出字段顺序</div>
          <div v-if="store.planHeaders.length === 0" style="color: var(--sf-text-muted); font-size: 11px">
            暂无输出字段
          </div>
          <div v-else style="display: flex; flex-direction: column; gap: 6px">
            <div
              v-for="(header, index) in store.planHeaders"
              :key="header"
              style="display: flex; align-items: center; gap: 8px; background: #f5f7fa; border-radius: 4px; padding: 6px 10px"
            >
              <span style="flex: 1; font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">
                {{ header }}
              </span>
              <el-button size="small" :disabled="index === 0" @click="store.moveOutputColumn(index, -1)">
                ↑
              </el-button>
              <el-button
                size="small"
                :disabled="index === store.planHeaders.length - 1"
                @click="store.moveOutputColumn(index, 1)"
              >
                ↓
              </el-button>
            </div>
          </div>
        </el-card>
      </div>

      <!-- 右侧：字段映射 / 引导 -->
      <div style="flex: 1; min-width: 0">
        <el-card v-if="!mappingEnabled" shadow="never" style="text-align: center; padding: 40px 20px">
          <div style="font-size: 14px; font-weight: 600; margin-bottom: 8px">并集 / 交集模式不修改字段</div>
          <div style="font-size: 12px; color: var(--sf-text-muted)">
            输出字段直接取自各表的原始表头。需要改名、纠错或清洗字段时，请切换到「修正表头」模式。
          </div>
        </el-card>
        <el-card v-else shadow="never">
          <div style="display: flex; gap: 8px; margin-bottom: 10px">
            <el-select
              :model-value="store.selectedMappingTable"
              style="flex: 1"
              @change="updateTableSelection"
            >
              <el-option
                v-for="item in enabledList"
                :key="item.index"
                :label="store.displayName(item.table)"
                :value="item.index"
              />
            </el-select>
            <el-input
              v-model="store.mappingSearch"
              placeholder="搜索字段"
              style="width: 170px"
              clearable
            />
            <el-button @click="store.mismatchOnly = !store.mismatchOnly">
              {{ store.mismatchOnly ? "显示全部" : "只看差异" }}
            </el-button>
            <el-button @click="store.applySuggestions()">应用建议</el-button>
            <el-button @click="store.resetMapping()">恢复本表</el-button>
          </div>
          <div style="display: flex; align-items: center; margin-bottom: 8px">
            <span style="flex: 1; font-size: 11px; color: var(--sf-text-muted)">字段映射与清洗</span>
            <el-button size="small" text @click="store.toggleCommonFields()">
              {{ store.hideCommonMappings ? "显示共有字段" : "隐藏共有字段" }}
            </el-button>
          </div>
          <el-table :data="mappingRows" size="small" :max-height="440" border>
            <el-table-column label="启用" width="64">
              <template #default="{ row }">
                <el-switch
                  :model-value="row.m.enabled"
                  size="small"
                  @change="(v: boolean | string | number) => store.setMapping(row.index, Boolean(v), row.m.target_name)"
                />
              </template>
            </el-table-column>
            <el-table-column label="源字段" min-width="140" show-overflow-tooltip>
              <template #default="{ row }">
                <div style="font-weight: 600">{{ row.m.source_name }}</div>
                <div v-if="row.suggestion" style="color: var(--sf-primary); font-size: 10.5px">
                  建议 → {{ row.suggestion }}
                </div>
              </template>
            </el-table-column>
            <el-table-column label="目标字段" min-width="160">
              <template #default="{ row }">
                <el-input
                  :model-value="row.m.target_name"
                  size="small"
                  :disabled="!row.m.enabled"
                  @update:model-value="(v: string) => store.setMapping(row.index, row.m.enabled, v)"
                />
              </template>
            </el-table-column>
            <el-table-column label="清洗" width="120">
              <template #default="{ row }">
                <el-select
                  :model-value="row.m.transform"
                  size="small"
                  @change="(v: string) => store.setMappingOperation(row.index, transformIndex(v), aggregateIndex(row.m.aggregate))"
                >
                  <el-option label="不处理" value="None" />
                  <el-option label="去空格" value="Trim" />
                  <el-option label="转大写" value="Uppercase" />
                  <el-option label="转小写" value="Lowercase" />
                </el-select>
              </template>
            </el-table-column>
            <el-table-column v-if="store.options.mode === 'Consolidate'" label="汇总方式" width="130">
              <template #default="{ row }">
                <el-select
                  :model-value="row.m.aggregate"
                  size="small"
                  @change="(v: string) => store.setMappingOperation(row.index, transformIndex(row.m.transform), aggregateIndex(v))"
                >
                  <el-option label="取首值" value="First" />
                  <el-option label="求和" value="Sum" />
                  <el-option label="唯一拼接" value="UniqueJoin" />
                  <el-option label="文本拼接" value="TextJoin" />
                </el-select>
              </template>
            </el-table-column>
          </el-table>
        </el-card>
      </div>
    </div>
  </div>
</template>
