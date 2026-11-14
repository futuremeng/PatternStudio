import { errMsg } from "./editorUtils.js";

/**
 * Engine error codes (see `EngineError::code` in patternstudio-engine) that
 * have a localized classification sentence. `cancelled` / `busy` are absent
 * on purpose: they are control-flow outcomes the UI handles silently.
 */
const CODE_KEYS = {
  dependency: "error.dependency",
  invalid_input: "error.invalidInput",
  io: "error.io",
  internal: "error.internal",
};

/**
 * Error text with the machine-readable code classified when we know one:
 * `"Invalid image: failed to decode image: ..."`. Falls back to the raw
 * message for codeless errors (string rejections, older servers), so this is
 * always safe to drop in wherever `errMsg` was used.
 */
export function errorText(e, t) {
  const msg = errMsg(e);
  const key = e?.code && CODE_KEYS[e.code];
  return key ? t(key, { msg }) : msg;
}
