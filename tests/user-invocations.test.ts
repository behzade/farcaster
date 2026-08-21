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

test("leading dollar invocations compose in user order", async () => {
  const stack = parseInvocationStack("$simplify $commit keep tests focused", commands);
  assert.ok(stack);
  assert.deepEqual(stack.commands.map((item) => item.invocationName), ["simplify", "commit"]);
  assert.equal(stack.argumentsText, "keep tests focused");

  const expanded = await expandInvocationStack(stack);
  const simplify = expanded.indexOf("Refine your implementation");
  const commit = expanded.indexOf("Commit your changes");
  assert.ok(simplify >= 0 && commit > simplify);
  assert.match(expanded, /keep tests focused$/);
});

test("skill invocations retain shared trailing instructions", async () => {
  const stack = parseInvocationStack("$frontend-design focus on the composer", commands);
  assert.ok(stack);
  const expanded = await expandInvocationStack(stack);
  assert.match(expanded, /<skill name="frontend-design"/);
  assert.match(expanded, /focus on the composer$/);
});

test("unknown, embedded, and escaped dollars remain ordinary input", () => {
  assert.equal(parseInvocationStack("$missing", commands), undefined);
  assert.equal(parseInvocationStack("explain $simplify", commands), undefined);
  assert.equal(parseInvocationStack("\\$simplify", commands), undefined);
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
