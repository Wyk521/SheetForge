import { describe, expect, it } from "vitest";
import { visibleRange } from "../src/utils/previewWindow";
describe("预览可见行", () => {
  it("大量来源只显示窗口附近的行，并可滚到最后一行", () => {
    expect(visibleRange(5000, 0, 320, 32)).toEqual([0, 12]);
    expect(visibleRange(5000, 4999 * 32, 320, 32)).toEqual([4988, 5000]);
  });
  it("更换为短表或空表后仍返回有效窗口", () => {
    expect(visibleRange(2, 900000, 320, 32)).toEqual([0, 2]);
    expect(visibleRange(0, 900000, 320, 32)).toEqual([0, 0]);
  });
});
