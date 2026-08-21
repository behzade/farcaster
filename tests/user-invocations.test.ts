import assert from "node:assert/strict";
import test from "node:test";
import type { SlashCommandInfo } from "@earendil-works/pi-coding-agent";
import registerUserInvocations, {
  expandInvocationStack,
  invocableCommands,
  parseInvocationStack,
} from "../extensions/user-invocations.ts";

const command = (
  name: string,
  source: "prompt" | "skill",
  path: URL,
): SlashCommandInfo => ({
  name,
  source,
  description: `${name} description`,
  sourceInfo: {
    path: path.pathname,
    source: "local",
    scope: "user",
    origin: "top-level",
    baseDir: new URL(".", path).pathname,
  },
});

const commands = [
  command("simplify", "prompt", new URL("../prompts/simplify.md", import.meta.url)),
  command("commit", "prompt", new URL("../prompts/commit.md", import.meta.url)),
  command("skill:frontend-design", "skill", new URL("../skills/frontend-design/SKILL.md", import.meta.url)),
];

test("the extension preserves compact display metadata outside model content", async () => {
  const handlers = new Map<string, (...arguments_: any[]) => any>();
  registerUserInvocations({
    getCommands: () => commands,
    on: (event: string, handler: (...arguments_: any[]) => any) => handlers.set(event, handler),
  } as never);

  const transformed = await handlers.get("input")?.(
    { source: "interactive", text: "$simplify $commit" },
    { ui: { notify() {} } },
  );
  assert.equal(transformed.action, "transform");
  assert.doesNotMatch(transformed.text, /pi:user-invocation/);

  const message = {
    role: "user",
    content: [{ type: "text", text: transformed.text }],
  };
  const finalized = await handlers.get("message_end")?.({ message });
  assert.equal(finalized.message.piUserInvocation, "$simplify $commit");
  assert.deepEqual(finalized.message.content, message.content);
});

test("autocomplete opens after a token boundary and includes skills", async () => {
  const handlers = new Map<string, (...arguments_: any[]) => any>();
  let provider: any;
  registerUserInvocations({
    getCommands: () => commands,
    on: (event: string, handler: (...arguments_: any[]) => any) => handlers.set(event, handler),
  } as never);
  handlers.get("session_start")?.({}, {
    mode: "tui",
    ui: {
      addAutocompleteProvider(factory: (current: any) => any) {
        provider = factory({
          async getSuggestions() { return null; },
          applyCompletion() { throw new Error("not used"); },
        });
      },
    },
  });

  const result = await provider.getSuggestions(["please $"], 0, "please $".length, {});
  assert.deepEqual(
    result.items.map((item: { value: string }) => item.value),
    ["$simplify", "$commit", "$frontend-design"],
  );
});

test("leading dollar invocations compose in user order", async () => {
  const stack = parseInvocationStack("$simplify $commit keep tests focused", commands);
  assert.ok(stack);
  assert.deepEqual(stack.commands.map((item) => item.invocationName), ["simplify", "commit"]);
  assert.equal(stack.argumentsText, "keep tests focused");

  const expanded = await expandInvocationStack(stack);
  assert.match(expanded, /keep tests focused$/);
});

test("skill invocations retain shared trailing instructions", async () => {
  const stack = parseInvocationStack("$frontend-design focus on the composer", commands);
  assert.ok(stack);
  const expanded = await expandInvocationStack(stack);
  assert.match(expanded, /<skill name="frontend-design"/);
  assert.match(expanded, /focus on the composer$/);
});

test("invocations expand at token boundaries within instructions", async () => {
  const stack = parseInvocationStack("please $simplify this $commit", commands);
  assert.ok(stack);
  assert.deepEqual(stack.commands.map((item) => item.invocationName), ["simplify", "commit"]);
  assert.equal(stack.argumentsText, "please this");
  assert.match(await expandInvocationStack(stack), /please this$/);
});

test("unknown and escaped dollars remain ordinary input", () => {
  assert.equal(parseInvocationStack("$missing", commands), undefined);
  assert.equal(parseInvocationStack("explain \\$simplify", commands), undefined);
});

test("prompt and skill name collisions require source-qualified invocations", () => {
  const colliding = [
    command("review", "prompt", new URL("../prompts/commit.md", import.meta.url)),
    command("review", "prompt", new URL("../prompts/simplify.md", import.meta.url)),
    command("skill:review", "skill", new URL("../skills/frontend-design/SKILL.md", import.meta.url)),
  ];
  assert.deepEqual(
    invocableCommands(colliding).map((item) => item.invocationName),
    ["prompt:review", "skill:review"],
  );
  assert.equal(parseInvocationStack("$review", colliding), undefined);
  assert.ok(parseInvocationStack("$prompt:review $skill:review", colliding));
});
