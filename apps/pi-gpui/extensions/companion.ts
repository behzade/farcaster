import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

import titleGeneration from "./title-generation.ts";
import workgraph from "./workgraph.ts";

export default function companion(pi: ExtensionAPI): void {
  titleGeneration(pi);
  workgraph(pi);
}
