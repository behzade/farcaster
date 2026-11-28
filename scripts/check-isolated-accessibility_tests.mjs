import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { checkIsolatedAccessibility, findControl } from './check-isolated-accessibility.mjs';

test('AX matching uses current indices and exact labels', () => {
  assert.deepEqual(findControl('  93 button Settings\n  4 button Close settings', 'button', 'Settings'), { index: 93, on: false });
  assert.equal(findControl('7 tab (settable, boolean) Workers, Value: on', 'tab (settable, boolean)', 'Workers').on, true);
  assert.equal(findControl('1 button Settings', 'button', 'Settings').index, 1);
});

test('ambiguous, missing and disabled controls fail without an action', () => {
  assert.throws(() => findControl('1 button Settings\n2 button Settings', 'button', 'Settings'), /found 2/);
  assert.throws(() => findControl('1 button Close settings', 'button', 'Settings'), /found 0/);
  assert.throws(() => findControl('1 button (disabled) Settings', 'button', 'Settings'), /Refusing disabled/);
});

test('runner refuses a missing workspace before clicking or typing', async () => {
  const calls = [];
  const sky = {
    get_app_state: async args => {
      calls.push(args);
      return { text: '0 system dialog Project trust' };
    },
    click: async () => assert.fail('No click permitted before preflight passes'),
    press_key: async () => assert.fail('No keyboard fallback permitted'),
  };
  await assert.rejects(checkIsolatedAccessibility(sky), /Workspace did not return/);
  assert.deepEqual(calls, [{ app: 'io.github.behzade.farcaster.isolated', disableDiff: true }]);
});

test('runner bounds repetitions before accessing the app', async () => {
  const sky = { get_app_state: async () => assert.fail('Invalid options must not touch the app') };
  for (const cycles of [0, 11, 1.5]) await assert.rejects(checkIsolatedAccessibility(sky, { cycles }), /cycles must be/);
});

test('runner restores the temporary preference when a later capture fails', async () => {
  let modal = false;
  let grouped = false;
  let selected = 'General';
  const pages = ['General', 'Workers', 'Appearance', 'Connections'];
  const sky = {
    get_app_state: async () => ({ text: modal
      ? '0 dialog Settings\n1 tab group Settings pages\n'
        + pages.map((name, i) => `${i + 2} tab (settable, boolean) ${name}, Value: ${selected === name ? 'on' : 'off'}`).join('\n')
        + `\n6 toggle button (settable, boolean) Group sessions by project, Value: ${grouped ? 'on' : 'off'}`
      : '0 container Workspace\n1 text field Search sessions\n2 text field Filter changed files\n'
        + '3 tab (settable, boolean) Chat composer (Cmd+G in app views; Ctrl+G Ctrl+G anywhere), Value: on\n'
        + '4 tab (settable, boolean) Neovim (Cmd+E in app views; Ctrl-G e anywhere), Value: off\n5 button Settings' }),
    click: async ({ element_index: i }) => {
      if (!modal) { assert.equal(i, 5); modal = true; }
      else if (i === 6) grouped = !grouped;
      else selected = pages[i - 2];
    },
  };
  await assert.rejects(checkIsolatedAccessibility(sky, {
    record: async name => { if (name === 'chat-1-toggle') throw new Error('capture failed'); },
  }), /capture failed/);
  assert.equal(grouped, false, 'the demo preference must return to its original value');
});
