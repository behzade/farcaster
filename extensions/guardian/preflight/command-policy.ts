export type Decision = "allow" | "prompt" | "forbid";

export interface CommandRule {
  id: string;
  pattern: string[];
  decision: Decision;
  reason?: string;
  allFlags?: string[];
}

export interface CommandPolicy {
  defaultDecision: Decision;
  rules: CommandRule[];
}

export interface ParsedCommand {
  argv: string[];
}

export interface PolicyMatch {
  decision: Decision;
  ruleId: string;
  reason: string;
  command: ParsedCommand;
}

const OPERATORS = new Set([";", "&&", "||", "|", "&", "\n"]);

export function parseShellCommands(source: string): ParsedCommand[] {
  const commands: ParsedCommand[] = [];
  let argv: string[] = [];
  let word = "";
  let quote: "'" | '"' | undefined;
  let escaped = false;

  const finishWord = () => {
    if (word) argv.push(word);
    word = "";
  };
  const finishCommand = () => {
    finishWord();
    if (argv.length) commands.push({ argv });
    argv = [];
  };

  for (let i = 0; i < source.length; i += 1) {
    const char = source[i];
    if (escaped) {
      word += char;
      escaped = false;
      continue;
    }
    if (char === "\\" && quote !== "'") {
      escaped = true;
      continue;
    }
    if (quote) {
      if (char === quote) quote = undefined;
      else word += char;
      continue;
    }
    if (char === "'" || char === '"') {
      quote = char;
      continue;
    }
    if (char === "#" && !word) {
      while (i + 1 < source.length && source[i + 1] !== "\n") i += 1;
      continue;
    }
    if (/\s/.test(char)) {
      finishWord();
      if (char === "\n") finishCommand();
      continue;
    }
    const pair = source.slice(i, i + 2);
    if (OPERATORS.has(pair)) {
      finishCommand();
      i += 1;
      continue;
    }
    if (OPERATORS.has(char) || char === "(" || char === ")" || char === "`") {
      finishCommand();
      continue;
    }
    word += char;
  }
  if (escaped) word += "\\";
  finishCommand();
  return commands;
}

function commandArgv(argv: string[]): string[] {
  let index = 0;
  while (index < argv.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(argv[index])) index += 1;
  if (["command", "builtin", "nohup"].includes(argv[index])) index += 1;
  if (argv[index] === "env") {
    index += 1;
    while (index < argv.length && (argv[index].startsWith("-") || argv[index].includes("="))) index += 1;
  }
  return argv.slice(index);
}

function flags(argv: string[]): Set<string> {
  const result = new Set<string>();
  for (const arg of argv.slice(1)) {
    if (/^--[^=]+/.test(arg)) result.add(arg.slice(2).split("=", 1)[0]);
    else if (/^-[^-]/.test(arg)) for (const flag of arg.slice(1)) result.add(flag);
  }
  return result;
}

function matches(rule: CommandRule, argv: string[]): boolean {
  if (rule.pattern.some((part, index) => argv[index] !== part)) return false;
  const present = flags(argv);
  return (rule.allFlags ?? []).every((flag) => present.has(flag));
}

const rank: Record<Decision, number> = { allow: 0, prompt: 1, forbid: 2 };

function commandsIncludingShellEval(source: string, depth = 0): ParsedCommand[] {
  const parsed = parseShellCommands(source).map((command) => ({ argv: commandArgv(command.argv) }));
  if (depth >= 3) return parsed;

  const nested: ParsedCommand[] = [];
  for (const command of parsed) {
    const [executable, ...args] = command.argv;
    if (["bash", "sh", "zsh"].includes(executable)) {
      const commandIndex = args.findIndex((arg) => arg === "-c" || arg === "--command");
      if (commandIndex >= 0 && args[commandIndex + 1]) {
        nested.push(...commandsIncludingShellEval(args[commandIndex + 1], depth + 1));
      }
    } else if (executable === "eval" && args.length) {
      nested.push(...commandsIncludingShellEval(args.join(" "), depth + 1));
    }

    const helperIndex = command.argv.findIndex((arg) =>
      arg.endsWith("/background-jobs/scripts/job.sh")
    );
    if (helperIndex >= 0 && command.argv[helperIndex + 1] === "start") {
      const backgroundCommand = command.argv[helperIndex + 4];
      if (backgroundCommand) {
        nested.push(...commandsIncludingShellEval(backgroundCommand, depth + 1));
      }
    }
  }
  return [...parsed, ...nested];
}

export function evaluateCommand(source: string, policy: CommandPolicy): PolicyMatch[] {
  return commandsIncludingShellEval(source).map((command) => {
    const argv = command.argv;
    const candidates = policy.rules.filter((rule) => matches(rule, argv));
    candidates.sort((a, b) => {
      const specificity = b.pattern.length + (b.allFlags?.length ?? 0) - (a.pattern.length + (a.allFlags?.length ?? 0));
      return specificity || rank[b.decision] - rank[a.decision];
    });
    const rule = candidates[0];
    return {
      decision: rule?.decision ?? policy.defaultDecision,
      ruleId: rule?.id ?? "default",
      reason: rule?.reason ?? `Default policy: ${policy.defaultDecision}`,
      command: { argv },
    };
  });
}

export function overallDecision(matches: PolicyMatch[]): PolicyMatch | undefined {
  return [...matches].sort((a, b) => rank[b.decision] - rank[a.decision])[0];
}
