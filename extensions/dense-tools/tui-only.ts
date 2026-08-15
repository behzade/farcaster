export type ExtensionMode = "tui" | "rpc" | "json" | "print";

export function registerOnceForTui(register: () => void): (mode: ExtensionMode) => boolean {
  let registered = false;
  return (mode) => {
    if (mode !== "tui" || registered) return false;
    register();
    registered = true;
    return true;
  };
}
