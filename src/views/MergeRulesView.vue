<script setup lang="ts">
import { computed } from "vue";
import { useMergeStore } from "../stores/merge";
import FieldBatchView from "../components/FieldBatchView.vue";
import type { MergeMode } from "../types";

const store = useMergeStore();

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

function onModeChange(mode: MergeMode) {
  store.setMode(mode);
}
</script>

<template>
  <div>
    <h1 style="font-size: 18px; font-weight: 600; margin: 0 0 14px">合并工作区</h1>

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

      </div>

      <!-- 右侧：字段映射与输出顺序 -->
      <div style="flex: 1; min-width: 0">
        <el-card shadow="never">
          <template v-if="mappingEnabled">
            <div style="font-size: 12px; font-weight: 600; margin-bottom: 4px">字段映射与输出顺序</div>
            <div style="font-size: 11px; color: var(--sf-text-muted); margin-bottom: 10px">
              按来源字段集中编辑；展开字段即可直接预览来源表，预览不会离开当前工作区。
            </div>
            <FieldBatchView />
          </template>
          <template v-else>
            <div style="font-size: 12px; font-weight: 600; margin-bottom: 4px">输出字段顺序</div>
            <div style="font-size: 11px; color: var(--sf-text-muted); margin-bottom: 10px">
              用上下按钮调整最终输出列顺序。
            </div>
            <div v-if="store.planHeaders.length === 0" style="color: var(--sf-text-muted); font-size: 11px">
              暂无输出字段
            </div>
            <div v-else style="display: flex; flex-direction: column; gap: 6px; margin-bottom: 14px">
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
            <div style="text-align: center; padding: 14px 20px 2px">
              <div style="font-size: 13px; font-weight: 600; margin-bottom: 6px">当前模式不修改字段</div>
              <div style="font-size: 12px; color: var(--sf-text-muted)">
                并集 / 交集直接使用原始表头；需要改名、纠错或清洗字段时，请切换到「修正表头」模式。
              </div>
            </div>
          </template>
        </el-card>
      </div>
    </div>
  </div>
</template>
