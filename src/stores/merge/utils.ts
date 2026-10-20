import type { ColumnMapping, SourceTable, TransformOp } from "../../types";
export interface FieldGroupSource {
  id: string;
  index: number;
  name: string;
  sourceIndex: number;
  sourceName: string;
  target: string;
}
export interface FieldGroup {
  id: string;
  key: string;
  count: number;
  tables: FieldGroupSource[];
  uniformTarget: string | null;
  uniformEnabled: boolean | null;
  uniformTransform: TransformOp | null;
}
export function fieldSourceId(table: SourceTable, mapping: ColumnMapping, mappingIndex: number): string {
  return JSON.stringify([table.path, table.sheet_name, mapping.source_index, mapping.source_name, mappingIndex]);
}
export function headerKey(value: string): string {
  return value
    .split(/\r\n?|\n|\u2028|\u2029/)
    .map((part) => part.trim())
    .join("")
    .toLowerCase();
}
export function displayName(table: SourceTable): string {
  const file = table.path.split(/[\\/]/).pop() ?? "";
  return `${file}  /  ${table.sheet_name}`;
}
export const formatNumber = (value: number) => value.toLocaleString("en-US");
