use super::*;

#[test]
fn descriptor_does_not_collapse_independent_features() {
    let descriptor = descriptor();
    assert_eq!(
        descriptor.capabilities.turns.steer,
        CapabilitySupport::Available
    );
    assert_eq!(
        descriptor.capabilities.turns.follow_up,
        CapabilitySupport::Available
    );
    assert_eq!(
        descriptor.capabilities.configuration.modes,
        CapabilitySupport::Unsupported
    );
    assert_eq!(
        descriptor.capabilities.observation.child_agents,
        CapabilitySupport::Unsupported
    );
}

#[test]
#[ignore = "requires Node.js"]
fn native_tool_bridge_preserves_schema_and_structured_results() {
    let output = std::process::Command::new("node")
        .args([
            "--input-type=module",
            "-e",
            r#"
import assert from "node:assert/strict";

const schema = {type: "object", properties: {value: {type: "integer"}}};
const results = [
  {content: [{type: "text", text: "value=7"}], structuredContent: {value: 7}},
  {content: [{type: "text", text: "failed"}], structuredContent: {error: "failed"}, isError: true},
  {content: [{type: "text", text: '{"farcaster_review":{"version":1}}'}],
    structuredContent: {farcaster_review: {version: 1}}},
  {content: [{type: "text", text: "plain result"}]},
];
globalThis.fetch = async (_url, options) => {
  const request = JSON.parse(options.body);
  if (request.method === "notifications/initialized") return new Response("", {status: 202});
  let result;
  switch (request.method) {
    case "initialize": result = {}; break;
    case "tools/list": result = {tools: [
      {name: "structured", inputSchema: {type: "object"}, outputSchema: schema},
      {name: "plain", inputSchema: {type: "object"}},
    ]}; break;
    case "tools/call": result = results[request.params.arguments.index]; break;
    default: assert.fail(`Unexpected bridge request: ${request.method}`);
  }
  return new Response(JSON.stringify({jsonrpc: "2.0", id: request.id, result}), {
    headers: {"content-type": "application/json"},
  });
};
const tools = new Map();
const {default: register} = await import("data:text/javascript;base64," +
  Buffer.from(process.env.FARCASTER_TEST_PI_BRIDGE).toString("base64"));
await register({on() {}, registerCommand() {}, registerTool(tool) {tools.set(tool.name, tool);}});
assert.deepEqual(tools.get("farcaster_structured").outputSchema, schema);
assert.equal(Object.hasOwn(tools.get("farcaster_plain"), "outputSchema"), false);
for (const [index, expected] of results.entries()) {
  const tool = tools.get(index === 3 ? "farcaster_plain" : "farcaster_structured");
  const actual = await tool.execute("call", {index});
  assert.deepEqual(actual.structuredContent, expected.structuredContent);
  assert.deepEqual(actual.details, expected.structuredContent ?? {});
  assert.equal(actual.isError, Boolean(expected.isError));
  assert.deepEqual(actual.content[0], expected.content[0]);
  assert.equal(actual.content.length, [2, 2, 1, 1][index]);
}
"#,
        ])
        .env("FARCASTER_TEST_PI_BRIDGE", include_str!("farcaster.js"))
        .env("FARCASTER_PROMPT_BOUNDARY_URL", "")
        .env("FARCASTER_MCP_URL", "https://fixture.invalid/mcp")
        .env("FARCASTER_MCP_CALLER", "fixture")
        .output()
        .expect("run Pi bridge contract test with Node");
    assert!(
        output.status.success(),
        "Pi bridge contract failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
