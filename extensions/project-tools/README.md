# Project tools

This global Pi extension loads trusted project tools from `.pi/tools` after project trust. It imports each tool into Pi's host process. Project tools have the same user rights as project extensions. The native sandbox and its broker do not take part.

Pi scans tools on startup and `/reload`. It does not watch tool files.
It loads at most 32 valid tools from one project.

## Layout

```text
.pi/tools/example/
├── tool.json
└── main.ts
```

`tool.json` contains only the model-facing data and strict JSON schemas:

```json
{
  "version": 1,
  "name": "example",
  "label": "Example",
  "description": "Return a greeting",
  "entrypoint": "main.ts",
  "parameters": {
    "type": "object",
    "additionalProperties": false,
    "required": ["name"],
    "properties": {
      "name": { "type": "string" }
    }
  },
  "result": {
    "type": "object",
    "additionalProperties": false,
    "required": ["greeting"],
    "properties": {
      "greeting": { "type": "string" }
    }
  }
}
```

`main.ts` exports one Effect v4 function. It may also export a layer that supplies the function's services:

```ts
import { Context, Data, Effect, Layer } from "effect";

interface Arguments {
  readonly name: string;
}

interface Result {
  readonly greeting: string;
}

class GreetingError extends Data.TaggedError("GreetingError")<{
  readonly message: string;
}> {}

interface Greeter {
  readonly greet: (name: string) => string;
}

const Greeter = Context.Service<Greeter>("Example/Greeter");

export const dependencies = Layer.succeed(Greeter)({
  greet: (name) => `Hello, ${name}`,
});

export const execute = (args: Arguments): Effect.Effect<Result, GreetingError, Greeter> =>
  Effect.gen(function* () {
    if (args.name.length === 0) {
      return yield* new GreetingError({ message: "name is empty" });
    }
    const greeter = yield* Greeter;
    return { greeting: greeter.greet(args.name) };
  });
```

The Effect `Exit` decides success or error. On success, the loader validates the value against `result`. A plain string succeeds only when `result` accepts a string. On failure, the typed Effect error becomes the Pi tool error. The loader also passes `{ toolCallId, projectRoot, signal }` as the second argument to `execute` and ties the Effect fiber to Pi's abort signal.

Project tools may import project files and packages. Imports run with full host rights. Run `/reload` after any manifest, source, or dependency change.
