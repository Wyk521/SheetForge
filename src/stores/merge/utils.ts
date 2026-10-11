import type { SourceTable, TransformOp } from "../../types";
export interface FieldGroupSource {
    index: number;
    name: string;
}
export interface FieldGroup {
    key: string;
    count: number;
    tables: FieldGroupSource[];
    uniformTarget: string | null;
    uniformEnabled: boolean | null;
    uniformTransform: TransformOp | null;
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
