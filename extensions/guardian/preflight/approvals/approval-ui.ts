import type { ExtensionContext } from "@earendil-works/pi-coding-agent";
import { DynamicBorder, keyHint, rawKeyHint } from "@earendil-works/pi-coding-agent";
import {
	Container,
	Spacer,
	Text,
	getKeybindings,
	matchesKey,
	type KeyId,
	type TUI,
} from "@earendil-works/pi-tui";
import { buildToolCallExplanation } from "../explain.js";
import { buildRuleSuggestion } from "../rule-suggestions.js";
import { isScopeOutsideWorkspace } from "../utils/path.js";
import type {
	ApprovalDecision,
	DebugLogger,
	PreflightConfig,
	RuleConflictAction,
	RuleConsistencyResult,
	RuleContextSnapshot,
	ToolPreflightMetadata,
	ToolCallSummary,
	ToolCallsContext,
	ToolDecision,
} from "../types.js";

const ANSI_RESET = "\u001b[0m";
const ANSI_ACTION = "\u001b[38;5;110m";
const ANSI_DESTRUCTIVE = "\u001b[1;38;5;203m";
const ANSI_SCOPE_WARNING = "\u001b[38;5;222m";
const ANSI_MUTED = "\u001b[38;5;244m";

export async function requestApproval(
	event: ToolCallsContext,
	toolCall: ToolCallSummary,
	metadata: ToolPreflightMetadata | undefined,
	decision: ToolDecision | undefined,
	existingRules: RuleContextSnapshot,
	ctx: ExtensionContext,
	config: PreflightConfig,
	logDebug: DebugLogger,
): Promise<ApprovalDecision> {
	const summary = metadata?.summary ?? "Review requested action";
	const destructive = metadata?.destructive ?? true;
	const scopeDetails = buildScopeDetails(metadata, ctx.cwd);
	const scopeLine = scopeDetails ? formatScopeLine(scopeDetails.text, scopeDetails.warn) : undefined;
	const policyLine = buildPolicyLine(decision?.policy);
	const middleLine = combineApprovalLines([policyLine, scopeLine]);
	const titleLine = formatTitleLine("Agent wants to:");
	const fallbackMessage = buildApprovalMessage(summary, destructive, middleLine);
	const policyDenied = Boolean(decision?.policy && decision.policy.decision === "deny");
	const baseOptions = [
		{ label: policyDenied ? "Allow once" : "Yes", action: "allow" as const },
		{ label: "Always (this workspace)", action: "allow-persist" as const },
		{ label: policyDenied ? "Keep blocked" : "No", action: "deny" as const },
	];
	const customRuleIndex = baseOptions.length;

	try {
		const selection = await ctx.ui.custom<ApprovalDecision | undefined>((tui, theme, _keybindings, done) => {
			let explanation: string | undefined;
			let status: "idle" | "loading" | "error" = "idle";
			let statusMessage: string | undefined;
			let explainController: AbortController | undefined;
			let ruleSuggestions: string[] = [];
			let ruleSuggestionIndex = 0;
			let ruleStatus: "idle" | "loading" | "error" = "idle";
			let ruleController: AbortController | undefined;
			let customRuleInput = "";
			let customRuleCursor = 0;
			let customRuleTouched = false;
			let customRuleUsesSuggestion = false;
			const ruleHistory: string[] = [];

			const explainKeys = normalizeKeyIds(config.explainKey);
			const hasExplain = explainKeys.length > 0;

			const hasCustomRuleInput = (): boolean => customRuleInput.trim().length > 0;

			const getCurrentRuleSuggestion = (): string | undefined => {
				if (ruleSuggestions.length === 0) return undefined;
				const index = Math.min(ruleSuggestionIndex, ruleSuggestions.length - 1);
				return ruleSuggestions[index];
			};

			const resolveRuleSuggestion = (): string | undefined => {
				if (hasCustomRuleInput()) return undefined;
				return getCurrentRuleSuggestion();
			};

			const resolveRuleSuggestionEnabled = (): boolean => {
				return canCycleRuleSuggestion(
					ruleStatus,
					customRuleInput,
					ruleSuggestions.length,
					customRuleTouched,
					customRuleUsesSuggestion,
				);
			};

			const resolveMiddleLine = (): string | undefined => {
				if (status === "loading") return formatMutedLine("Fetching explanation...");
				if (status === "error" && statusMessage) return formatWarningLine(statusMessage);
				if (explanation) return formatExplainLine(explanation);
				return middleLine;
			};

			const buildOptionLabels = (): string[] => {
				return [
					...baseOptions.map((option) => option.label),
					buildCustomRuleOptionLabel(
						customRuleInput,
						resolveRuleSuggestion(),
						ruleStatus,
						customRuleTouched,
					),
				];
			};

			let selector: ApprovalSelectorComponent;

			const updateOptions = (): void => {
				selector.setOptions(buildOptionLabels());
				selector.setRuleSuggestionEnabled(resolveRuleSuggestionEnabled());
				selector.setCustomRuleCursor(customRuleCursor, customRuleTouched, hasCustomRuleInput());
			};

			const handleCustomRuleInput = (keyData: string): boolean => {
				const kb = getKeybindings();
				if (kb.matches(keyData, "tui.input.tab")) {
					if (!resolveRuleSuggestionEnabled()) return false;
					const hasInput = hasCustomRuleInput();

					if (!hasInput) {
						const suggestion = getCurrentRuleSuggestion();
						if (!suggestion) return false;
						customRuleTouched = true;
						customRuleUsesSuggestion = true;
						customRuleInput = suggestion;
						customRuleCursor = codePointLength(customRuleInput);
						updateOptions();
						tui.requestRender();
						return true;
					}

					if (!customRuleUsesSuggestion) return false;

					if (ruleSuggestionIndex < ruleSuggestions.length - 1) {
						ruleSuggestionIndex += 1;
						const nextSuggestion = getCurrentRuleSuggestion();
						if (nextSuggestion) {
							customRuleInput = nextSuggestion;
							customRuleCursor = codePointLength(customRuleInput);
						}
						updateOptions();
						tui.requestRender();
						return true;
					}

					startRuleSuggestionFetch();
					return true;
				}

				if (kb.matches(keyData, "tui.editor.cursorLeft")) {
					customRuleTouched = true;
					customRuleCursor = Math.max(0, customRuleCursor - 1);
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.cursorRight")) {
					customRuleTouched = true;
					customRuleCursor = Math.min(codePointLength(customRuleInput), customRuleCursor + 1);
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.cursorLineStart")) {
					customRuleTouched = true;
					customRuleCursor = 0;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.cursorLineEnd")) {
					customRuleTouched = true;
					customRuleCursor = codePointLength(customRuleInput);
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.cursorWordLeft")) {
					customRuleTouched = true;
					customRuleCursor = moveWordCursorLeft(customRuleInput, customRuleCursor);
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.cursorWordRight")) {
					customRuleTouched = true;
					customRuleCursor = moveWordCursorRight(customRuleInput, customRuleCursor);
					updateOptions();
					tui.requestRender();
					return true;
				}

				if (kb.matches(keyData, "tui.editor.deleteCharBackward")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeCharBackward(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.deleteCharForward")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeCharForward(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.deleteWordBackward")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeWordBackward(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.deleteWordForward")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeWordForward(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.deleteToLineStart")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeToLineStart(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}
				if (kb.matches(keyData, "tui.editor.deleteToLineEnd")) {
					customRuleTouched = true;
					customRuleUsesSuggestion = false;
					const next = removeToLineEnd(customRuleInput, customRuleCursor);
					customRuleInput = next.value;
					customRuleCursor = next.cursor;
					updateOptions();
					tui.requestRender();
					return true;
				}

				if (!isPrintableInput(keyData)) return false;
				customRuleTouched = true;
				customRuleUsesSuggestion = false;
				const next = insertAtCursor(customRuleInput, customRuleCursor, keyData);
				customRuleInput = next.value;
				customRuleCursor = next.cursor;
				updateOptions();
				tui.requestRender();
				return true;
			};

			const handleSelection = (index: number): void => {
				if (index !== customRuleIndex) {
					const selected = baseOptions[index];
					done(selected ? { action: selected.action } : { action: "deny" });
					return;
				}
				const resolvedRule = resolveCustomRule(customRuleInput, resolveRuleSuggestion(), {
					acceptSuggestion: false,
				});
				if (!resolvedRule) {
					ruleStatus = "error";
					updateOptions();
					tui.requestRender();
					return;
				}
				done({ action: "custom-rule", rule: resolvedRule });
			};

			selector = new ApprovalSelectorComponent({
				title: buildApprovalTitle(titleLine, summary, destructive, middleLine),
				options: buildOptionLabels(),
				theme,
				tui,
				explainKeys,
				ruleSuggestionEnabled: resolveRuleSuggestionEnabled(),
				customRuleIndex,
				onSelect: (index) => handleSelection(index),
				onCancel: () => done({ action: "deny" }),
				onExplain: hasExplain ? () => startExplain() : undefined,
				onCustomRuleKey: (keyData) => handleCustomRuleInput(keyData),
			});
			selector.setCustomRuleCursor(customRuleCursor, customRuleTouched, hasCustomRuleInput());

			const updateTitle = (): void => {
				const middleLine = resolveMiddleLine();
				selector.setTitle(buildApprovalTitle(titleLine, summary, destructive, middleLine));
			};

			const fetchExplanation = async (signal: AbortSignal): Promise<void> => {
				const result = await buildToolCallExplanation(
					event,
					toolCall,
					metadata,
					ctx,
					config,
					logDebug,
					signal,
				);
				if (signal.aborted) return;

				if (result.status === "ok") {
					explanation = result.text;
					status = "idle";
					statusMessage = undefined;
				} else {
					status = "error";
					statusMessage = result.reason;
				}

				updateTitle();
				tui.requestRender();
			};

			const startExplain = (): void => {
				if (!hasExplain || status === "loading") return;
				status = "loading";
				statusMessage = undefined;
				explanation = undefined;
				updateTitle();
				tui.requestRender();

				explainController?.abort();
				explainController = new AbortController();
				void fetchExplanation(explainController.signal);
			};

			const addRuleHistory = (suggestions: string[]): void => {
				for (const suggestion of suggestions) {
					if (!ruleHistory.includes(suggestion)) {
						ruleHistory.push(suggestion);
					}
				}
			};

			const applyRuleSuggestions = (suggestions: string[]): void => {
				ruleSuggestions = suggestions;
				ruleSuggestionIndex = 0;
				addRuleHistory(suggestions);
			};

			const fetchRuleSuggestions = async (signal: AbortSignal): Promise<void> => {
				const result = await buildRuleSuggestion(
					event,
					toolCall,
					metadata,
					ctx,
					config,
					logDebug,
					existingRules,
					ruleHistory,
					signal,
				);
				if (signal.aborted) return;

				if (result.status === "ok") {
					applyRuleSuggestions(result.suggestions);
					ruleStatus = "idle";
					if (customRuleUsesSuggestion) {
						const nextSuggestion = getCurrentRuleSuggestion();
						if (nextSuggestion) {
							customRuleInput = nextSuggestion;
							customRuleCursor = codePointLength(customRuleInput);
						}
					}
				} else {
					ruleStatus = "error";
				}

				updateOptions();
				tui.requestRender();
			};

			const startRuleSuggestionFetch = (): void => {
				if (ruleStatus === "loading") return;
				if (hasCustomRuleInput() && !customRuleUsesSuggestion) {
					return;
				}
				ruleStatus = "loading";
				updateOptions();
				tui.requestRender();

				ruleController?.abort();
				ruleController = new AbortController();
				void fetchRuleSuggestions(ruleController.signal);
			};

			updateTitle();
			startRuleSuggestionFetch();

			return selector;
		});

		return selection ?? { action: "deny" };
	} catch (error) {
		const message = error instanceof Error ? error.message : String(error);
		logDebug(`Approval dialog failed: ${message}`);
		const allow = await ctx.ui.confirm(titleLine, fallbackMessage);
		return allow ? { action: "allow" } : { action: "deny" };
	}
}

export async function requestRuleConflictAction(
	toolCall: ToolCallSummary,
	candidateRule: string,
	consistency: RuleConsistencyResult,
	ctx: ExtensionContext,
): Promise<RuleConflictAction> {
	if (!ctx.hasUI) {
		return "save-anyway";
	}

	const title = buildConflictTitle(candidateRule, consistency);

	const selection = await ctx.ui.select(title, [
		"Edit rule",
		"Save anyway",
		"Cancel",
	]);

	if (!selection || selection.startsWith("Edit")) {
		return "edit-rule";
	}
	if (selection.startsWith("Save")) {
		return "save-anyway";
	}
	return "cancel";
}

function buildConflictTitle(
	candidateRule: string,
	consistency: RuleConsistencyResult,
): string {
	const lines: string[] = [
		`${ANSI_SCOPE_WARNING}⚠ Rule conflict${ANSI_RESET}`,
		"",
		`${ANSI_MUTED}New rule:${ANSI_RESET}  ${candidateRule}`,
	];

	if (consistency.conflictsWith.length > 0) {
		lines.push(
			`${ANSI_MUTED}Conflicts:${ANSI_RESET} ${consistency.conflictsWith.join(", ")}`,
		);
	}

	if (consistency.reason.trim()) {
		const short = truncateReason(consistency.reason, 120);
		lines.push(`${ANSI_MUTED}Reason:${ANSI_RESET}    ${short}`);
	}

	return lines.join("\n");
}

function truncateReason(reason: string, maxLength: number): string {
	const oneLine = reason.replace(/\s+/g, " ").trim();
	if (oneLine.length <= maxLength) return oneLine;
	return `${oneLine.slice(0, maxLength - 1)}…`;
}

class ApprovalSelectorComponent extends Container {
	private options: string[];
	private selectedIndex = 0;
	private listContainer: Container;
	private titleText: Text;
	private hintText: Text;
	private theme: ExtensionContext["ui"]["theme"];
	private tui: TUI;
	private explainKeys: KeyId[];
	private ruleSuggestionEnabled: boolean;
	private customRuleIndex?: number;
	private customRuleCursor = 0;
	private customRuleTouched = false;
	private customRuleHasInput = false;
	private onSelect: (index: number) => void;
	private onCancel: () => void;
	private onExplain?: () => void;
	private onCustomRuleKey?: (keyData: string) => boolean;
	private title: string;

	constructor(options: {
		title: string;
		options: string[];
		theme: ExtensionContext["ui"]["theme"];
		tui: TUI;
		explainKeys: KeyId[];
		ruleSuggestionEnabled: boolean;
		customRuleIndex?: number;
		onSelect: (index: number) => void;
		onCancel: () => void;
		onExplain?: () => void;
		onCustomRuleKey?: (keyData: string) => boolean;
	}) {
		super();
		this.options = options.options;
		this.theme = options.theme;
		this.tui = options.tui;
		this.explainKeys = options.explainKeys;
		this.ruleSuggestionEnabled = options.ruleSuggestionEnabled;
		this.customRuleIndex = options.customRuleIndex;
		this.onSelect = options.onSelect;
		this.onCancel = options.onCancel;
		this.onExplain = options.onExplain;
		this.onCustomRuleKey = options.onCustomRuleKey;
		this.title = options.title;

		this.addChild(new DynamicBorder((s: string) => this.theme.fg("border", s)));
		this.addChild(new Spacer(1));

		this.titleText = new Text("", 1, 0);
		this.addChild(this.titleText);
		this.addChild(new Spacer(1));

		this.listContainer = new Container();
		this.addChild(this.listContainer);
		this.addChild(new Spacer(1));

		this.hintText = new Text("", 1, 0);
		this.addChild(this.hintText);
		this.addChild(new Spacer(1));
		this.addChild(new DynamicBorder((s: string) => this.theme.fg("border", s)));

		this.updateTitle();
		this.updateList();
		this.updateHints();
	}

	setTitle(title: string): void {
		this.title = title;
		this.updateTitle();
	}

	setOptions(options: string[]): void {
		this.options = options;
		if (this.selectedIndex >= options.length) {
			this.selectedIndex = Math.max(0, options.length - 1);
		}
		this.updateList();
		this.updateHints();
	}

	setRuleSuggestionEnabled(enabled: boolean): void {
		this.ruleSuggestionEnabled = enabled;
		this.updateHints();
	}

	setCustomRuleCursor(cursor: number, touched: boolean, hasInput: boolean): void {
		this.customRuleCursor = Math.max(0, cursor);
		this.customRuleTouched = touched;
		this.customRuleHasInput = hasInput;
		this.updateList();
	}

	override invalidate(): void {
		super.invalidate();
		this.updateTitle();
		this.updateList();
		this.updateHints();
	}

	handleInput(keyData: string): void {
		const kb = getKeybindings();
		const customRuleSelected = this.customRuleIndex === this.selectedIndex;
		if (kb.matches(keyData, "tui.select.up") || (!customRuleSelected && keyData === "k")) {
			this.selectedIndex = Math.max(0, this.selectedIndex - 1);
			this.updateList();
			this.updateHints();
			this.tui.requestRender();
			return;
		}
		if (kb.matches(keyData, "tui.select.down") || (!customRuleSelected && keyData === "j")) {
			this.selectedIndex = Math.min(this.options.length - 1, this.selectedIndex + 1);
			this.updateList();
			this.updateHints();
			this.tui.requestRender();
			return;
		}
		if (kb.matches(keyData, "tui.select.confirm") || keyData === "\n") {
			this.onSelect(this.selectedIndex);
			return;
		}
		if (kb.matches(keyData, "tui.select.cancel")) {
			this.onCancel();
			return;
		}
		if (this.explainKeys.length > 0 && matchesKeyList(keyData, this.explainKeys)) {
			this.onExplain?.();
			return;
		}
		if (this.customRuleIndex === this.selectedIndex && this.onCustomRuleKey) {
			const handled = this.onCustomRuleKey(keyData);
			if (handled) return;
		}
	}

	private updateTitle(): void {
		this.titleText.setText(this.theme.fg("accent", this.title));
	}

	private updateList(): void {
		this.listContainer.clear();
		for (let i = 0; i < this.options.length; i++) {
			const option = this.options[i] ?? "";
			const isSelected = i === this.selectedIndex;
			const isCustomRuleOption = this.customRuleIndex === i;
			if (isSelected && isCustomRuleOption) {
				const rendered = this.renderCustomRuleOption(option);
				this.listContainer.addChild(new Text(this.theme.fg("accent", "→ ") + rendered, 1, 0));
				continue;
			}
			const text = isSelected
				? this.theme.fg("accent", "→ ") + this.theme.fg("accent", option)
				: `  ${this.theme.fg("text", option)}`;
			this.listContainer.addChild(new Text(text, 1, 0));
		}
	}

	private renderCustomRuleOption(option: string): string {
		if (!this.customRuleHasInput) {
			return renderCursorAtFirstVisible(option);
		}
		return renderInputCursor(option, this.customRuleCursor);
	}

	private updateHints(): void {
		const ruleHint =
			this.ruleSuggestionEnabled && this.customRuleIndex === this.selectedIndex
				? `  ${rawKeyHint("Tab", "accept/next suggestion")}`
				: "";
		const explainHint =
			this.explainKeys.length > 0
				? `  ${rawKeyHint(formatKeyList(this.explainKeys), "explain")}`
				: "";
		const hintLine =
			rawKeyHint("↑↓", "navigate") +
			"  " +
			keyHint("tui.select.confirm", "select") +
			"  " +
			keyHint("tui.select.cancel", "cancel") +
			ruleHint +
			explainHint;
		this.hintText.setText(hintLine);
	}
}

export function canCycleRuleSuggestion(
	status: "idle" | "loading" | "error",
	input: string,
	suggestionsCount: number,
	_touched: boolean = false,
	usesSuggestionInput: boolean = false,
): boolean {
	if (status === "loading") return false;
	if (suggestionsCount === 0) return false;
	if (input.trim().length > 0 && !usesSuggestionInput) return false;
	return true;
}

export function buildCustomRuleOptionLabel(
	input: string,
	suggestion: string | undefined,
	status: "idle" | "loading" | "error",
	_touched: boolean = false,
): string {
	const trimmedInput = input.trim();
	if (trimmedInput) return input;
	if (status === "loading") return formatMutedLine("Fetching suggestion...");
	if (suggestion) return formatMutedLine(suggestion);
	return formatMutedLine("Type custom rule");
}

export function resolveCustomRule(
	input: string,
	suggestion: string | undefined,
	options?: { acceptSuggestion?: boolean },
): string | undefined {
	const trimmedInput = input.trim();
	if (trimmedInput) return trimmedInput;
	if (!options?.acceptSuggestion) return undefined;
	const trimmedSuggestion = suggestion?.trim();
	return trimmedSuggestion ? trimmedSuggestion : undefined;
}

function isPrintableInput(data: string): boolean {
	if (!data) return false;
	return ![...data].some((ch) => {
		const code = ch.charCodeAt(0);
		return code < 32 || code === 0x7f || (code >= 0x80 && code <= 0x9f);
	});
}

function codePointLength(value: string): number {
	return Array.from(value).length;
}

function insertAtCursor(value: string, cursor: number, text: string): { value: string; cursor: number } {
	const chars = Array.from(value);
	const insert = Array.from(text);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const next = [...chars.slice(0, clampedCursor), ...insert, ...chars.slice(clampedCursor)];
	return {
		value: next.join(""),
		cursor: clampedCursor + insert.length,
	};
}

function removeCharBackward(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	if (cursor <= 0 || chars.length === 0) {
		return { value, cursor: Math.max(0, cursor) };
	}
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	if (clampedCursor === 0) {
		return { value, cursor: 0 };
	}
	const next = [...chars.slice(0, clampedCursor - 1), ...chars.slice(clampedCursor)];
	return {
		value: next.join(""),
		cursor: clampedCursor - 1,
	};
}

function removeCharForward(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	if (clampedCursor >= chars.length) {
		return { value, cursor: clampedCursor };
	}
	const next = [...chars.slice(0, clampedCursor), ...chars.slice(clampedCursor + 1)];
	return {
		value: next.join(""),
		cursor: clampedCursor,
	};
}

function removeWordBackward(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const before = chars.slice(0, clampedCursor).join("");
	const after = chars.slice(clampedCursor).join("");
	const reduced = before.replace(/\s*\S+\s*$/u, "");
	return {
		value: `${reduced}${after}`,
		cursor: codePointLength(reduced),
	};
}

function removeWordForward(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const before = chars.slice(0, clampedCursor).join("");
	const after = chars.slice(clampedCursor).join("");
	const reducedAfter = after.replace(/^\s*\S+\s*/u, "");
	return {
		value: `${before}${reducedAfter}`,
		cursor: clampedCursor,
	};
}

function removeToLineStart(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const after = chars.slice(clampedCursor).join("");
	return { value: after, cursor: 0 };
}

function removeToLineEnd(value: string, cursor: number): { value: string; cursor: number } {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const before = chars.slice(0, clampedCursor).join("");
	return { value: before, cursor: clampedCursor };
}

function moveWordCursorLeft(value: string, cursor: number): number {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const before = chars.slice(0, clampedCursor).join("");
	const reduced = before.replace(/\s+$/u, "").replace(/\S+$/u, "");
	return codePointLength(reduced);
}

function moveWordCursorRight(value: string, cursor: number): number {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const after = chars.slice(clampedCursor).join("");
	const match = after.match(/^\s*\S+\s*/u);
	if (!match || !match[0]) return clampedCursor;
	return clampedCursor + codePointLength(match[0]);
}

function renderInputCursor(value: string, cursor: number): string {
	const chars = Array.from(value);
	const clampedCursor = Math.max(0, Math.min(cursor, chars.length));
	const before = chars.slice(0, clampedCursor).join("");
	const at = chars[clampedCursor] ?? " ";
	const after = chars.slice(clampedCursor + (chars[clampedCursor] ? 1 : 0)).join("");
	return `${before}${formatCursorCell(at)}${after}`;
}

function formatCursorCell(value: string = " "): string {
	return `\u001b[7m${value}\u001b[27m`;
}

function renderCursorAtFirstVisible(value: string): string {
	let index = 0;
	while (index < value.length) {
		if (value[index] === "\u001b") {
			const match = /^\u001b\[[0-9;]*m/.exec(value.slice(index));
			if (match) {
				index += match[0].length;
				continue;
			}
		}
		break;
	}

	if (index >= value.length) {
		return formatCursorCell(" ");
	}

	const nextSymbol = Array.from(value.slice(index))[0];
	if (!nextSymbol) {
		return formatCursorCell(" ");
	}
	const symbolLength = nextSymbol.length;
	return `${value.slice(0, index)}${formatCursorCell(nextSymbol)}${value.slice(index + symbolLength)}`;
}

function buildApprovalTitle(
	titleLine: string,
	summary: string,
	destructive: boolean,
	middleLine?: string,
): string {
	return `${titleLine}\n${buildApprovalMessage(summary, destructive, middleLine)}`;
}

function buildApprovalMessage(summary: string, destructive: boolean, middleLine?: string): string {
	const lines = [formatActionLine(summary, destructive)];
	if (middleLine) {
		lines.push("", middleLine);
	}
	return lines.join("\n");
}

function buildScopeDetails(
	metadata: ToolPreflightMetadata | undefined,
	cwd: string,
): { text: string; warn: boolean } | undefined {
	if (!metadata?.scope?.length) return undefined;
	const text = `Scope: ${metadata.scope.join(", ")}`;
	const warn = isScopeOutsideWorkspace(metadata.scope, cwd);
	return { text, warn };
}

function buildPolicyLine(policy: ToolDecision["policy"] | undefined): string | undefined {
	if (!policy || policy.decision !== "deny") return undefined;
	const reason = policy.reason ? `: ${policy.reason}` : "";
	return formatWarningLine(`Policy blocked by custom rules${reason}`);
}

function combineApprovalLines(lines: Array<string | undefined>): string | undefined {
	const filtered = lines.filter((line): line is string => Boolean(line));
	return filtered.length > 0 ? filtered.join("\n") : undefined;
}

function normalizeKeyIds(keys: KeyId | KeyId[]): KeyId[] {
	return Array.isArray(keys) ? keys : [keys];
}

function matchesKeyList(data: string, keys: KeyId[]): boolean {
	for (const key of keys) {
		if (matchesKey(data, key)) return true;
	}
	return false;
}

function formatKeyList(keys: KeyId[]): string {
	return keys.join("/");
}

function formatMutedLine(text: string): string {
	return `${ANSI_MUTED}${text}${ANSI_RESET}`;
}

function formatTitleLine(text: string): string {
	return `${ANSI_MUTED}${text}${ANSI_RESET}`;
}

function formatActionLine(text: string, destructive: boolean): string {
	const color = destructive ? ANSI_DESTRUCTIVE : ANSI_ACTION;
	return `${color}${text}${ANSI_RESET}`;
}

function formatScopeLine(text: string, warn: boolean): string {
	const color = warn ? ANSI_SCOPE_WARNING : ANSI_MUTED;
	return `${color}${text}${ANSI_RESET}`;
}

function formatExplainLine(text: string): string {
	const lines = text.split("\n");
	return lines.map((line) => formatExplainLineSegment(line)).join("\n");
}

function formatExplainLineSegment(line: string): string {
	const match = line.match(/^(low|med|high)\s+risk:\s*/i);
	if (!match) {
		return `${ANSI_RESET}${line}`;
	}
	const prefix = match[0];
	const rest = line.slice(prefix.length);
	return `${ANSI_RESET}${ANSI_SCOPE_WARNING}${prefix}${ANSI_RESET}${rest}`;
}

function formatWarningLine(text: string): string {
	return `${ANSI_SCOPE_WARNING}${text}${ANSI_RESET}`;
}
