const HOST_SCRIPT_MAX_OUTPUT_BYTES = 64 * 1024;

export interface HostScriptResult {
  status: "success" | "failure";
  exitCode: number | null;
  output: string;
  truncated: boolean;
}

interface HostScriptOperations {
  exec(command: string, cwd: string, options: {
    onData: (data: Buffer) => void;
    signal?: AbortSignal;
  }): Promise<{ exitCode: number | null }>;
}

export async function executeHostScript(
  operations: HostScriptOperations,
  script: string,
  cwd: string,
  signal?: AbortSignal,
  maxOutputBytes = HOST_SCRIPT_MAX_OUTPUT_BYTES,
): Promise<HostScriptResult> {
  let output = Buffer.alloc(0);
  let truncated = false;
  const append = (data: Buffer) => {
    if (data.length >= maxOutputBytes) {
      output = Buffer.from(data.subarray(data.length - maxOutputBytes));
      truncated = true;
      return;
    }
    const combined = Buffer.concat([output, data]);
    if (combined.length > maxOutputBytes) {
      output = Buffer.from(combined.subarray(combined.length - maxOutputBytes));
      truncated = true;
    } else {
      output = combined;
    }
  };

  let exitCode: number | null = null;
  try {
    ({ exitCode } = await operations.exec(script, cwd, { onData: append, signal }));
  } catch (error) {
    append(Buffer.from(error instanceof Error ? error.message : String(error)));
  }

  const text = output.toString("utf8").trimEnd();
  return {
    status: exitCode === 0 ? "success" : "failure",
    exitCode,
    output: truncated ? `[output truncated to last ${maxOutputBytes} bytes]\n${text}` : text,
    truncated,
  };
}
