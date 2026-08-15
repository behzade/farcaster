import assert from "node:assert/strict";
import test from "node:test";
import { selectThemeWhenAvailable } from "../extensions/dense-tools/theme-selection.ts";

test("theme selection is skipped when the host exposes no themes", () => {
  let selected = false;
  let notified = false;
  selectThemeWhenAvailable(
    {
      getAllThemes: () => [],
      setTheme: () => {
        selected = true;
        return { success: false };
      },
      notify: () => {
        notified = true;
      },
    },
    "pi-dense",
  );
  assert.equal(selected, false);
  assert.equal(notified, false);
});

test("an available theme is selected and real failures stay visible", () => {
  const notifications: string[] = [];
  selectThemeWhenAvailable(
    {
      getAllThemes: () => [{ name: "pi-dense" }],
      setTheme: () => ({ success: false, error: "bad theme" }),
      notify: (message) => notifications.push(message),
    },
    "pi-dense",
  );
  assert.deepEqual(notifications, ["bad theme"]);
});
