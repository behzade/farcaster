import { pathToFileURL } from "node:url";
import { createInterface } from "node:readline";

const emit = (event) => process.stdout.write(`${JSON.stringify(event)}\n`);
timingOutput = emit;
const abort = new AbortController();
const input = createInterface({ input: process.stdin });
input.on("line", () => abort.abort());
input.on("close", () => abort.abort());
const deadline = setTimeout(() => abort.abort(), 300_000);

try {
  const { Cursor, FileCredentialStore, InMemoryCredentialStore } =
    await timed("SDK.import", () => import(pathToFileURL(process.argv[1]).href));
  const pending = timedSync("InMemoryCredentialStore.new", () => new InMemoryCredentialStore());
  await timed("Cursor.auth.login", () => Cursor.auth.login({
    store: pending,
    openBrowser: false,
    onLoginUrl: (url) => emit({ type: "url", url }),
    apiKeyName: "Farcaster",
    signal: abort.signal,
  }));
  abort.signal.throwIfAborted();
  const credentials = await timed("InMemoryCredentialStore.load", () => pending.load());
  if (!credentials) throw new Error("Cursor sign-in returned no credentials");
  const saved = timedSync("FileCredentialStore.new", () => new FileCredentialStore(process.argv[2]));
  await timed("FileCredentialStore.save", () => saved.save(credentials));
  emit({ type: "complete" });
} catch (error) {
  emit({ type: "error", message: error instanceof Error ? error.message : "Cursor sign-in failed" });
  process.exitCode = 1;
} finally {
  clearTimeout(deadline);
  input.close();
  process.stdin.destroy();
}
