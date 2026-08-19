import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import applicationExit from "./application-exit.ts";
import titleGeneration from "./title-generation.ts";
import workgraph from "./workgraph.ts";

export default function companion(pi: ExtensionAPI): void {
  applicationExit(pi);
  titleGeneration(pi);
  workgraph(pi);
}
