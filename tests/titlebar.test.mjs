import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { compileTemplate, parse } from "vue/compiler-sfc";
import * as Vue from "vue";

const titleBarPath = new URL("../src/components/TitleBar.vue", import.meta.url);

function compileTitleBarRender(source) {
  const { descriptor, errors: parseErrors } = parse(source, { filename: "TitleBar.vue" });
  assert.equal(parseErrors.length, 0, parseErrors.join("\n"));

  const result = compileTemplate({
    id: "titlebar-regression",
    filename: "TitleBar.vue",
    source: descriptor.template.content,
  });
  assert.equal(result.errors.length, 0, result.errors.join("\n"));

  const importMatch = result.code.match(/^import \{ ([^\n]+) \} from "vue"\r?\n\r?\n/);
  assert.ok(importMatch, "Vue helper import should be present in the compiled template");
  const bindings = importMatch[1]
    .split(", ")
    .map((binding) => {
      const [name, alias] = binding.split(" as ");
      return `${name}: ${alias ?? name}`;
    })
    .join(", ");
  const executableCode = result.code
    .replace(importMatch[0], `const { ${bindings} } = Vue;\n`)
    .replace("export function render", "function render");

  return new Function("Vue", `${executableCode}; return render;`)(Vue);
}

function findVNode(node, predicate) {
  if (!node || typeof node !== "object") return undefined;
  if (predicate(node)) return node;
  if (!Array.isArray(node.children)) return undefined;
  for (const child of node.children) {
    const match = findVNode(child, predicate);
    if (match) return match;
  }
  return undefined;
}

test("the Tauri drag region owns titlebar double-click maximize", async () => {
  const source = await readFile(titleBarPath, "utf8");
  const render = compileTitleBarRender(source);
  const appWindow = {
    close() {},
    minimize() {},
    toggleMaximize() {},
  };
  const vnode = render({ appWindow, isMax: false }, []);
  const dragRegion = findVNode(
    vnode,
    (candidate) => candidate.props?.class === "sf-titlebar-drag",
  );
  const maximizeButton = findVNode(
    vnode,
    (candidate) => candidate.props?.class === "sf-win-btn" && candidate.props?.title === "最大化",
  );

  assert.ok(dragRegion, "titlebar drag region should be rendered");
  assert.equal(
    dragRegion.props?.onDblclick,
    undefined,
    "the drag region must not register a second maximize toggle",
  );
  assert.ok(maximizeButton, "maximize button should be rendered");
  assert.equal(typeof maximizeButton.props?.onClick, "function");
});
