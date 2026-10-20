import { ref, computed } from "vue";
import { invoke } from "../../native";
import {
  defaultOptions,
  type ColumnMapping,
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
  const fieldGroups = computed<FieldGroup[]>(() => {
    const byKey = new Map<
      string,
      {
        id: string;
        key: string;
        tables: FieldGroupSource[];
        targets: Map<string, string>;
        enables: Set<boolean>;
        transforms: Set<TransformOp>;
      }
    >();
    for (const [index, table] of sources.value.entries()) {
      if (!table.enabled) continue;
      for (const [mappingIndex, mapping] of table.mappings.entries()) {
        // 同名来源只有在目标也相同时才属于一组，拆出后所有批量操作都按组隔离。
        const key = JSON.stringify([headerKey(mapping.source_name), headerKey(mapping.target_name)]);
        let group = byKey.get(key);
        if (!group) {
          group = {
            id: key,
            key: mapping.source_name.trim(),
            tables: [],
            targets: new Map(),
            enables: new Set(),
            transforms: new Set(),
          };
          byKey.set(key, group);
        }
        group.tables.push({
          id: fieldSourceId(table, mapping, mappingIndex),
          index,
          name: displayName(table),
          sourceIndex: mapping.source_index,
          sourceName: mapping.source_name,
          target: mapping.target_name,
        });
        const target = mapping.target_name.trim();
        group.targets.set(headerKey(target), target);
        group.enables.add(mapping.enabled);
        group.transforms.add(mapping.transform);
      }
    }
    const outputPositions = new Map(
      planHeaders.value.map((header, index) => [headerKey(header), index]),
    );
    const outputPosition = (value: string) =>
      outputPositions.get(headerKey(value)) ?? Number.MAX_SAFE_INTEGER;
    const groupPosition = (group: {
      key: string;
      targets: Map<string, string>;
    }) =>
      Math.min(
        ...[...group.targets.values()].map((target) => outputPosition(target)),
      );
    return [...byKey.values()]
      .sort((a, b) => {
        const position = groupPosition(a) - groupPosition(b);
        return (
          position ||
          b.tables.length - a.tables.length ||
          a.key.localeCompare(b.key, "zh")
        );
      })
      .map((g) => ({
        id: g.id,
        key: g.key,
        count: new Set(g.tables.map((source) => source.index)).size,
        tables: g.tables,
        uniformTarget: g.targets.size === 1 ? [...g.targets.values()][0] : null,
        uniformEnabled: g.enables.size === 1 ? [...g.enables][0] : null,
        uniformTransform: g.transforms.size === 1 ? [...g.transforms][0] : null,
      }));
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
  function setMode(mode: MergeOptions["mode"]) {
    options.value.mode = mode;
    void refreshPlan();
  }
  function setAdvanced(patch: Partial<MergeOptions>) {
    Object.assign(options.value, patch);
    void refreshPlan();
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
    void refreshPlan();
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
  function setSourceFieldTargets(ids: string[], target: string) {
    target = target.trim();
    if (!target) return;
    const mappings = sourceMappings(ids);
    if (!mappings.length) return;
    const changes = mappings.map((mapping) => ({ previous: mapping.target_name, next: target }));
    for (const mapping of mappings) mapping.target_name = target;
    updateOutputOrder(changes);
    void refreshPlan();
  }
  function setFieldTarget(id: string, target: string) {
    const group = fieldGroups.value.find((group) => group.id === id);
    if (group) setSourceFieldTargets(group.tables.map((source) => source.id), target);
  }
  function setFieldEnabled(id: string, enabled: boolean) {
    for (const mapping of groupMappings(id)) mapping.enabled = enabled;
    void refreshPlan();
  }
  function setFieldTransform(id: string, transform: TransformOp) {
    for (const mapping of groupMappings(id)) mapping.transform = transform;
    void refreshPlan();
  }
  function resetField(id: string) {
    const mappings = groupMappings(id);
    const changes = mappings.map((mapping) => ({ previous: mapping.target_name, next: mapping.source_name }));
    for (const mapping of mappings) {
      mapping.target_name = mapping.source_name;
      mapping.enabled = true;
      mapping.transform = "None";
    }
    updateOutputOrder(changes);
    void refreshPlan();
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
