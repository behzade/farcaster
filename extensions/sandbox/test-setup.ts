import { mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ensureDevelopmentCacheDirectories } from "./development-caches.ts";

const testRoot = mkdtempSync(join(tmpdir(), "pi-sandbox-test-"));
const testHome = join(testRoot, "home");
const testTmp = join(testRoot, "tmp");
mkdirSync(testHome);
mkdirSync(testTmp);
process.env.HOME = realpathSync.native(testHome);
process.env.TMPDIR = realpathSync.native(testTmp);
process.env.TMP = process.env.TMPDIR;
process.env.TEMP = process.env.TMPDIR;
ensureDevelopmentCacheDirectories();

process.once("exit", () => {
	rmSync(testRoot, { recursive: true, force: true });
});
