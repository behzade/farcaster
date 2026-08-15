interface ThemeSelectionUi {
  getAllThemes(): { name: string }[];
  setTheme(name: string): { success: boolean; error?: string };
  notify(message: string, tone: "error"): void;
}

export function selectThemeWhenAvailable(ui: ThemeSelectionUi, name: string): void {
  if (!ui.getAllThemes().some((theme) => theme.name === name)) return;
  const selected = ui.setTheme(name);
  if (!selected.success) ui.notify(selected.error ?? `Could not select ${name}`, "error");
}
