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
    | "checkIssues"
    | "checkRan"
    | "clearMergedPreview"
  >,
) {
  const {
    sources,
    options,
    planHeaders,
    checkIssues,
    checkRan,
    clearMergedPreview,
  } = context;
  let planRequest = 0;
  const fieldGroups = computed<FieldGroup[]>(() => {
    const byKey = new Map<
      string,
      {
        key: string;
        tables: FieldGroupSource[];
        targets: Map<string, string>;
        enables: Set<boolean>;
        transforms: Set<TransformOp>;
      }
    >();
    for (const [index, table] of sources.value.entries()) {
      if (!table.enabled) continue;
      for (const mapping of table.mappings) {
        const key = headerKey(mapping.source_name);
        let group = byKey.get(key);
        if (!group) {
          group = {
            key: mapping.source_name.trim(),
            tables: [],
            targets: new Map(),
            enables: new Set(),
            transforms: new Set(),
          };
          byKey.set(key, group);
        }
        group.tables.push({ index, name: displayName(table) });
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
        outputPosition(group.key),
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
        key: g.key,
        count: g.tables.length,
        tables: g.tables,
        uniformTarget: g.targets.size === 1 ? [...g.targets.values()][0] : null,
        uniformEnabled: g.enables.size === 1 ? [...g.enables][0] : null,
        uniformTransform: g.transforms.size === 1 ? [...g.transforms][0] : null,
      }));
  });
  async function refreshPlan() {
    const request = ++planRequest;
    clearMergedPreview();
    checkIssues.value = [];
    checkRan.value = false;
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
  function replaceOutputOrderName(previous: string, next: string) {
    const previousKey = headerKey(previous);
    if (!previousKey || options.value.output_order.length === 0) return;
    const nextValue = next.trim();
    const seen = new Set<string>();
    let changed = false;
    const updated: string[] = [];
    for (const name of options.value.output_order) {
      let value = name;
      if (headerKey(name) === previousKey) {
        value = nextValue;
        changed = true;
      }
      const key = headerKey(value);
      if (!key) {
        changed = true;
        continue;
      }
      if (seen.has(key)) {
        changed = true;
        continue;
      }
      seen.add(key);
      updated.push(value);
    }
    if (changed) options.value.output_order = updated;
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
  function forEachFieldMapping(
    key: string,
    fn: (mapping: ColumnMapping) => void,
  ) {
    const wantedKey = headerKey(key);
    for (const table of sources.value) {
      if (!table.enabled) continue;
      for (const mapping of table.mappings) {
        if (headerKey(mapping.source_name) === wantedKey) fn(mapping);
      }
    }
  }
  function setFieldTarget(key: string, target: string) {
    if (!target.trim()) return;
    const previousTargets = new Set<string>();
    forEachFieldMapping(key, (mapping) =>
      previousTargets.add(mapping.target_name),
    );
    for (const previous of previousTargets)
      replaceOutputOrderName(previous, target);
    forEachFieldMapping(key, (mapping) => {
      mapping.target_name = target.trim();
    });
    void refreshPlan();
  }
  function setFieldEnabled(key: string, enabled: boolean) {
    forEachFieldMapping(key, (mapping) => {
      mapping.enabled = enabled;
    });
    void refreshPlan();
  }
  function setFieldTransform(key: string, transform: TransformOp) {
    forEachFieldMapping(key, (mapping) => {
      mapping.transform = transform;
    });
    void refreshPlan();
  }
  function resetField(key: string) {
    forEachFieldMapping(key, (mapping) =>
      replaceOutputOrderName(mapping.target_name, mapping.source_name),
    );
    forEachFieldMapping(key, (mapping) => {
      mapping.target_name = mapping.source_name;
      mapping.enabled = true;
      mapping.transform = "None";
    });
    void refreshPlan();
  }
  return {
    fieldGroups,
    refreshPlan,
    setMode,
    setAdvanced,
    replaceOutputOrderName,
    moveOutputColumn,
    moveOutputColumnByName,
    forEachFieldMapping,
    setFieldTarget,
    setFieldEnabled,
    setFieldTransform,
    resetField,
  };
}
