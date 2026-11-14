import { describe, it, expect } from "vitest";
import { errorText } from "./errorText.js";

// Test translator mirroring i18n interpolation: "key:msg".
const t = (key, vars) => `${key}:${vars?.msg ?? ""}`;

describe("errorText", () => {
  it("classifies engine error codes into localized sentences", () => {
    const invalid = new Error("failed to decode image: bad bytes");
    invalid.code = "invalid_input";
    expect(errorText(invalid, t)).toBe(
      "error.invalidInput:failed to decode image: bad bytes"
    );
    const dep = new Error("realesrgan-ncnn-vulkan binary not found");
    dep.code = "dependency";
    expect(errorText(dep, t)).toBe(
      "error.dependency:realesrgan-ncnn-vulkan binary not found"
    );
    for (const [code, key] of [["io", "error.io"], ["internal", "error.internal"]]) {
      const err = new Error("m");
      err.code = code;
      expect(errorText(err, t)).toBe(`${key}:m`);
    }
  });

  it("falls back to the raw message for codeless, unknown, or string errors", () => {
    expect(errorText(new Error("plain"), t)).toBe("plain");
    expect(errorText("legacy string rejection", t)).toBe("legacy string rejection");
    const unknown = { message: "ipc rejection", code: "some_future_code" };
    expect(errorText(unknown, t)).toBe("ipc rejection");
  });
});
