import { describe, expect, it } from "vitest";
import { relativeTime } from "./EnvironmentProof";

// 只测纯函数：组件本体依赖若干 API，取值/渲染交给项目既有的面板测试覆盖。
describe("最近验证的相对时间", () => {
  const now = Date.UTC(2026, 8, 12, 12, 0, 0);

  it("按量级给出人话", () => {
    expect(relativeTime(now / 1000, now)).toBe("刚刚");
    expect(relativeTime(now / 1000 - 30, now)).toBe("刚刚");
    expect(relativeTime(now / 1000 - 5 * 60, now)).toBe("5 分钟前");
    expect(relativeTime(now / 1000 - 3 * 3600, now)).toBe("3 小时前");
    expect(relativeTime(now / 1000 - 2 * 86400, now)).toBe("2 天前");
  });

  it("时间戳来自未来时不显示负数分钟前", () => {
    // 系统时钟被改过 / 记录来自未来：宁可显示"刚刚"，也不要说"-40 分钟前"
    expect(relativeTime(now / 1000 + 600, now)).toBe("刚刚");
  });
});
