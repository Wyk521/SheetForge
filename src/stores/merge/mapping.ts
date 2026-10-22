import { ref, computed, type ComputedRef } from "vue";
import { invoke } from "../../native";
import { ElMessage } from "element-plus";
import {
  defaultOptions,
  type ColumnMapping,
  type SourceTable,
  type MergeOptions,
  type TransformOp,
} from "../../types";
import {
  headerKey,
  displayName,
  fieldSourceId,
  type FieldGroup,
  type FieldGroupSource,
} from "./utils";
import type { MergeContext } from "./context";
export function createMappingState() {
  const options = ref<MergeOptions>(defaultOptions());
  const mappingSearch = ref("");
  const onlyMultiField = ref(false);
  const planHeaders = ref<string[]>([]);
  return { options, mappingSearch, onlyMultiField, planHeaders };
}
export function createMappingActions(
  context: Pick<
    MergeContext,
    | "sources"
    | "options"
    | "planHeaders"
    | "invalidateResults"
  >,
) {
  const {
    sources,
    options,
    planHeaders,
    invalidateResults,
  } = context;
  let planRequest = 0;
  interface Fragment {
    key: string; name: string; target: string; targetKey: string;
    enabled: boolean; transform: TransformOp; source: FieldGroupSource;
  }
  const fragments = new WeakMap<SourceTable, { index: number; value: ComputedRef<Fragment[]> }>();
  const collator = new Intl.Collator("zh");
  function tableFragments(table: SourceTable, index: number) {
    let cached = fragments.get(table);
    if (!cached || cached.index !== index) {
      cached = { index, value: computed(() => {
        if (!table.enabled) return [];
        const name = displayName(table);
        const targets = new Map<string, number>();
        for (const mapping of table.mappings) {
          if (!mapping.enabled) continue;
          const key = headerKey(mapping.target_name);
          if (key) targets.set(key, (targets.get(key) ?? 0) + 1);
        }
        return table.mappings.map((mapping, mappingIndex) => {
          const target = mapping.target_name.trim();
          const targetKey = headerKey(target);
          return { key: JSON.stringify([headerKey(mapping.source_name), targetKey]),
            name: mapping.source_name.trim(), target, targetKey, enabled: mapping.enabled,
            transform: mapping.transform,
            source: { id: fieldSourceId(table, mapping, mappingIndex), index, name,
              sourceIndex: mapping.source_index, sourceName: mapping.source_name, target: mapping.target_name,
              conflict: mapping.enabled && (targets.get(targetKey) ?? 0) > 1 } };
        });
      }) };
      fragments.set(table, cached);
    }
    return cached.value.value;
  }
  const fieldGroups = computed<FieldGroup[]>(() => {
    const outputPositions = new Map(planHeaders.value.map((header, index) => [headerKey(header), index]));
    const byKey = new Map<string, { id: string; key: string; tables: FieldGroupSource[];
      targets: Map<string, string>; enables: Set<boolean>; transforms: Set<TransformOp>; position: number; counts: Set<number> }>();
    for (const [index, table] of sources.value.entries()) {
      for (const item of tableFragments(table, index)) {
        let group = byKey.get(item.key);
        if (!group) {
          group = { id: item.key, key: item.name, tables: [], targets: new Map(), enables: new Set(), transforms: new Set(),
            position: outputPositions.get(item.targetKey) ?? Number.MAX_SAFE_INTEGER, counts: new Set() };
          byKey.set(item.key, group);
        }
        group.tables.push(item.source);
        group.counts.add(index);
        group.targets.set(item.targetKey, item.target);
        group.enables.add(item.enabled);
        group.transforms.add(item.transform);
      }
    }
    return [...byKey.values()]
      .sort((a, b) => a.position - b.position || b.tables.length - a.tables.length || collator.compare(a.key, b.key))
      .map(g => ({ id: g.id, key: g.key, count: g.counts.size, tables: g.tables,
        uniformTarget: g.targets.size === 1 ? [...g.targets.values()][0] : null,
        uniformEnabled: g.enables.size === 1 ? [...g.enables][0] : null,
        uniformTransform: g.transforms.size === 1 ? [...g.transforms][0] : null }));
  });
  async function refreshPlan() {
    const request = ++planRequest;
    invalidateResults();
    try {
      const plan = await invoke("get_plan", {
        tables: sources.value,
        options: options.value,
      });
      if (request === planRequest) planHeaders.value = plan.headers;
    } catch (error) {
      if (request === planRequest) planHeaders.value = [];
    }
  }
  let planScheduled = false;
  function schedulePlan() {
    invalidateResults();
    // Immediately invalidate older in-flight replies, before the next microtask.
    planRequest += 1;
    if (planScheduled) return;
    planScheduled = true;
    queueMicrotask(() => { planScheduled = false; void refreshPlan(); });
  }
  function setMode(mode: MergeOptions["mode"]) {
    options.value.mode = mode;
    schedulePlan();
  }
  function setAdvanced(patch: Partial<MergeOptions>) {
    Object.assign(options.value, patch);
    // These are the only options that can change output headers.
    if (patch.mode !== undefined || patch.output_order !== undefined
      || patch.include_source_file !== undefined || patch.include_source_sheet !== undefined) {
      schedulePlan();
    } else {
      invalidateResults();
    }
  }
  function updateOutputOrder(changes: { previous: string; next: string }[]) {
    if (options.value.output_order.length === 0) return;
    const renamed = new Map<string, Set<string>>();
    for (const { previous, next } of changes) {
      const key = headerKey(previous);
      if (key === headerKey(next)) continue;
      const targets = renamed.get(key) ?? new Set<string>();
      targets.add(next.trim());
      renamed.set(key, targets);
    }
    // 部分来源改名时仍须保留旧输出列；已有的新目标保持原来的位置。
    const activeTargets = new Set(sources.value.filter((table) => table.enabled)
      .flatMap((table) => table.mappings.filter((mapping) => mapping.enabled)
        .map((mapping) => headerKey(mapping.target_name))));
    const originalKeys = new Set(options.value.output_order.map(headerKey));
    const seen = new Set<string>();
    const updated: string[] = [];
    function append(value: string) {
      const key = headerKey(value);
      if (key && !seen.has(key)) {
        seen.add(key);
        updated.push(value);
      }
    }
    for (const name of options.value.output_order) {
      const key = headerKey(name);
      const targets = renamed.get(key);
      if (!targets || activeTargets.has(key)) append(name);
      for (const target of targets ?? []) {
        if (!originalKeys.has(headerKey(target))) append(target);
      }
    }
    options.value.output_order = updated;
  }
  function moveOutputColumn(index: number, direction: number) {
    const headers = [...planHeaders.value];
    if (index < 0 || index >= headers.length) return;
    const target = Math.min(Math.max(index + direction, 0), headers.length - 1);
    [headers[index], headers[target]] = [headers[target], headers[index]];
    options.value.output_order = headers;
    schedulePlan();
  }
  function moveOutputColumnByName(name: string, direction: number) {
    const index = planHeaders.value.findIndex(
      (header) => headerKey(header) === headerKey(name),
    );
    if (index >= 0) moveOutputColumn(index, direction);
  }
  function sourceMappings(ids: string[]): ColumnMapping[] {
    const wanted = new Set(ids);
    const result: ColumnMapping[] = [];
    for (const table of sources.value) {
      if (!table.enabled) continue;
      for (const [mappingIndex, mapping] of table.mappings.entries()) {
        if (wanted.has(fieldSourceId(table, mapping, mappingIndex))) result.push(mapping);
      }
    }
    return result;
  }
  function groupMappings(id: string) {
    const group = fieldGroups.value.find((group) => group.id === id);
    return sourceMappings(group?.tables.map((source) => source.id) ?? []);
  }
  // Validate the complete proposed batch before changing any mapping or output order.
  function canApplyMappings(mappings: ColumnMapping[], patch: (mapping: ColumnMapping) => Partial<ColumnMapping>) {
    const changes = new Map(mappings.map((mapping) => [mapping, patch(mapping)]));
    for (const table of sources.value) {
      if (!table.enabled || !table.mappings.some((mapping) => changes.has(mapping))) continue;
      const targets = new Map<string, { mapping: ColumnMapping; changed: boolean }>();
      for (const original of table.mappings) {
        const proposed = { ...original, ...changes.get(original) };
        if (!proposed.enabled) continue;
        const key = headerKey(proposed.target_name);
        if (!key) continue;
        const previous = targets.get(key);
        const changed = changes.has(original);
        if (previous && (changed || previous.changed)) {
          ElMessage.warning(`${displayName(table)}：第 ${previous.mapping.source_index + 1} 列「${previous.mapping.source_name}」与第 ${proposed.source_index + 1} 列「${proposed.source_name}」不能同时映射到「${proposed.target_name.trim()}」`);
          return false;
        }
        targets.set(key, { mapping: proposed, changed });
      }
    }
    return true;
  }
  function setSourceFieldTargets(ids: string[], target: string) {
    target = target.trim();
    if (!target) return false;
    const mappings = sourceMappings(ids);
    if (!mappings.length || !canApplyMappings(mappings, () => ({ target_name: target }))) return false;
    const changes = mappings.map((mapping) => ({ previous: mapping.target_name, next: target }));
    for (const mapping of mappings) mapping.target_name = target;
    updateOutputOrder(changes);
    schedulePlan();
    return true;
  }
  function setFieldTarget(id: string, target: string) {
    const group = fieldGroups.value.find((group) => group.id === id);
    return group ? setSourceFieldTargets(group.tables.map((source) => source.id), target) : false;
  }
  function setFieldEnabled(id: string, enabled: boolean) {
    const mappings = groupMappings(id);
    if (!canApplyMappings(mappings, () => ({ enabled }))) return false;
    for (const mapping of mappings) mapping.enabled = enabled;
    schedulePlan();
    return true;
  }
  function setFieldTransform(id: string, transform: TransformOp) {
    for (const mapping of groupMappings(id)) mapping.transform = transform;
    schedulePlan();
  }
  function resetField(id: string) {
    const mappings = groupMappings(id);
    if (!canApplyMappings(mappings, (mapping) => ({ target_name: mapping.source_name, enabled: true }))) return false;
    const changes = mappings.map((mapping) => ({ previous: mapping.target_name, next: mapping.source_name }));
    for (const mapping of mappings) {
      mapping.target_name = mapping.source_name;
      mapping.enabled = true;
      mapping.transform = "None";
    }
    updateOutputOrder(changes);
    schedulePlan();
    return true;
  }
  return {
    fieldGroups,
    refreshPlan,
    setMode,
    setAdvanced,
    moveOutputColumn,
    moveOutputColumnByName,
    setSourceFieldTargets,
    setFieldTarget,
    setFieldEnabled,
    setFieldTransform,
    resetField,
  };
}
