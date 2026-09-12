import { describe, expect, it } from "vitest";
import { buildUsageProfileOptions } from "./usageProfileOptions";

describe("buildUsageProfileOptions", () => {
  it("includes newly created profiles even before they have usage rows", () => {
    expect(buildUsageProfileOptions([], ["new-space"])).toEqual([
      { value: "__all__", label: "全部环境" },
      { value: "new-space", label: "new-space" },
    ]);
  });

  it("unions configured and historical profiles without duplicates", () => {
    expect(
      buildUsageProfileOptions(
        [{ profile: "old-space" }, { profile: "__main__" }, { profile: "new-space" }],
        ["new-space"],
      ),
    ).toEqual([
      { value: "__all__", label: "全部环境" },
      { value: "__main__", label: "默认 Claude" },
      { value: "new-space", label: "new-space" },
      { value: "old-space", label: "old-space" },
    ]);
  });
});
