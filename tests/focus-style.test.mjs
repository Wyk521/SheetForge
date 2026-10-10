import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const hallmarkPath = new URL("../src/hallmark.css", import.meta.url);

test("表单控件聚焦时只显示一层焦点框", async () => {
  const css = await readFile(hallmarkPath, "utf8");
  const baseRule = css.match(
    /\.el-input__wrapper,[\s\S]*?\.el-input-number \.el-input__wrapper \{([\s\S]*?)\}/,
  );
  const focusRule = css.match(
    /\.el-input__wrapper\.is-focus,[\s\S]*?\.el-textarea__inner:focus \{([\s\S]*?)\}/,
  );

  assert.ok(baseRule, "应保留统一的表单控件基础样式");
  assert.ok(focusRule, "应保留统一的表单控件焦点样式");
  assert.match(
    css,
    /\[tabindex\]:not\(input\):not\(select\):not\(textarea\):focus-visible/,
  );
  assert.doesNotMatch(css, /\n\[tabindex\]:focus-visible/);
  assert.doesNotMatch(baseRule[1], /outline/);
  assert.match(focusRule[1], /box-shadow:\s*0 0 0 2px var\(--color-focus\) inset/);
  assert.doesNotMatch(focusRule[1], /outline/);
});
