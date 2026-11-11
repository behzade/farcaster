// Run through Computer Use's node_repl, with its supported @oai/sky client:
// var ax = await import('/absolute/repo/scripts/check-isolated-accessibility.mjs');
// var report = await ax.checkIsolatedAccessibility(sky);
// nodeRepl.write(JSON.stringify(report, null, 2));
//
// Use a seeded Isolated instance with Neovim available and a completed demo
// session open. This never types, submits prompts, selects files, or changes
// projects. It restores the one temporary preference change and leaves Chat
// open. Failures include the last AX tree; no retry masks a first-read failure.

const APP = 'io.github.behzade.farcaster.isolated';

export function findControl(text, role, label) {
  const rows = text.split('\n').filter(line => {
    const match = line.match(/^\s*(\d+) (.+)$/);
    if (!match || !match[2].startsWith(role + ' ')) return false;
    const description = match[2];
    return description.endsWith(' ' + label)
      || description.endsWith(' ' + label + ', Value: on')
      || description.endsWith(' ' + label + ', Value: off');
  });
  if (rows.length !== 1) throw new Error(`Expected one ${role} "${label}", found ${rows.length}`);
  if (rows[0].includes('(disabled)')) throw new Error(`Refusing disabled control "${label}"`);
  return { index: Number(rows[0].trim().split(' ')[0]), on: rows[0].endsWith(', Value: on') };
}

export async function checkIsolatedAccessibility(sky, { cycles = 3, record = async () => {} } = {}) {
  if (!Number.isInteger(cycles) || cycles < 1 || cycles > 10) throw new Error('cycles must be 1–10');
  const checks = [];
  const unverified = [];
  let last;
  const state = async name => {
    last = await sky.get_app_state({ app: APP, disableDiff: true });
    await record(name, last);
    return last.text;
  };
  const expect = (value, message) => {
    if (!value) throw new Error(`${message}\n${last?.text ?? ''}`);
  };
  const press = async (role, label, name) => {
    const before = await state('before-' + name);
    const control = findControl(before, role, label);
    await sky.click({ app: APP, element_index: control.index });
    return state(name);
  };
  const workspace = text => {
    expect(text.includes('container Workspace'), 'Workspace did not return');
    expect(text.includes('Search sessions'), 'Left search missing after redraw');
    expect(text.includes('Filter changed files'), 'Right search missing after redraw');
  };
  const selectedTab = (text, label) => {
    expect(findControl(text, 'tab (settable, boolean)', label).on, `Tab "${label}" not selected`);
  };
  const chat = 'Chat composer (Cmd+G in app views; Ctrl+G Ctrl+G anywhere)';
  const editor = 'Neovim (Cmd+E in app views; Ctrl-G e anywhere)';
  workspace(await state('initial'));
  selectedTab(last.text, chat);
  findControl(last.text, 'tab (settable, boolean)', editor);

  let restoreGrouping;
  try {
    for (const surface of ['chat', 'editor']) {
      if (surface === 'editor') {
        selectedTab(await press('tab (settable, boolean)', editor, 'editor'), editor);
      }
      for (let cycle = 1; cycle <= cycles; cycle++) {
        const prefix = `${surface}-${cycle}`;
        let text = await press('button', 'Settings', prefix + '-settings-first-read');
        expect(text.includes('dialog Settings'), 'Settings absent on first read');
        expect(text.includes('tab group Settings pages'), 'Settings controls absent on first read');
        if (surface === 'editor') expect(text.includes('system dialog Farcaster — Dialog'), 'Expected native modal above Neovim');
        expect(!text.includes('Search sessions'), 'Covered workspace leaked into modal tree');
        selectedTab(text, 'General');
        for (const label of ['Workers', 'Appearance', 'Connections', 'General']) {
          text = await press('tab (settable, boolean)', label, prefix + '-' + label);
          selectedTab(text, label);
          expect(!text.includes('Search sessions'), 'Covered workspace returned after tab change');
        }
        const label = 'Group sessions by project';
        const original = findControl(text, 'toggle button (settable, boolean)', label).on;
        restoreGrouping = original;
        text = await press('toggle button (settable, boolean)', label, prefix + '-toggle');
        expect(findControl(text, 'toggle button (settable, boolean)', label).on !== original, 'Standard button AX press had no effect');
        text = await press('toggle button (settable, boolean)', label, prefix + '-restore-toggle');
        expect(findControl(text, 'toggle button (settable, boolean)', label).on === original, 'Preference did not restore');
        restoreGrouping = undefined;
        workspace(await press('button', 'Close settings', prefix + '-close'));
        if (surface === 'editor') selectedTab(last.text, editor);
        checks.push({ surface, cycle, firstRead: true, tabs: true, button: true, modalScope: true, restored: true });
      }
      // The editor uses a separate native popup; Chat uses an inline menu.
      const menu = await press('button', 'Repository actions', surface + '-dropdown-first-read');
      expect(menu.includes('Expand all folders') && menu.includes('Collapse all folders'), 'Dropdown absent on first read');
      await sky.press_key({ app: APP, key: 'Escape' });
      workspace(await state(surface + '-dropdown-closed'));
      checks.push({ surface, dropdown: true, restored: true });
    }
    selectedTab(await press('tab (settable, boolean)', chat, 'restore-chat'), chat);
    // Exercise the other shared trigger (a popover rather than a menu).
    const runtime = await press('button', 'Runtime', 'runtime-first-read');
    findControl(runtime, 'text field (settable)', 'Search models…');
    await sky.press_key({ app: APP, key: 'Escape' });
    workspace(await state('runtime-closed'));
    checks.push({ runtimePopover: true, restored: true });
    // Focus must return to the composer after a modal closes. Use no typing:
    // the focused AX line itself is the assertion.
    const focusedComposer = text => /The focused UI element is .*text entry area.*What would you like to work on\?/.test(text);
    const focused = await press('text entry area (settable)', 'What would you like to work on?', 'focus-composer');
    if (focusedComposer(focused)) {
      await press('button', 'Settings', 'focus-settings');
      const restored = await press('button', 'Close settings', 'focus-restored');
      expect(focusedComposer(restored), 'Composer focus not restored');
      checks.push({ composerFocus: true });
    } else {
      // Some macOS/AX sessions report only the window even before opening a
      // modal. That cannot establish a restoration failure or a passing check.
      unverified.push('Composer focus restoration: AX did not report composer focus before opening a modal');
    }
    return { app: APP, passed: true, checks, unverified };
  } finally {
    // Restore the only preference this check changes, even on assertion failure.
    if (restoreGrouping !== undefined) {
      const text = await state('cleanup');
      const label = 'Group sessions by project';
      if (findControl(text, 'toggle button (settable, boolean)', label).on !== restoreGrouping) {
        await press('toggle button (settable, boolean)', label, 'cleanup-restore-toggle');
      }
    }
  }
}
