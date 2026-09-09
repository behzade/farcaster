#!/usr/bin/env node
// Run with an external TypeScript installation and SDK package; never installs dependencies.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';

const [sdkDirectory, typescriptDirectory, upstreamRevision] = process.argv.slice(2);
if (!sdkDirectory || !typescriptDirectory || !upstreamRevision) {
  throw new Error('Usage: node scripts/import_claude_sdk_types.mjs SDK_DIRECTORY TYPESCRIPT_DIRECTORY UPSTREAM_REVISION');
}
const require = createRequire(import.meta.url);
const ts = require(path.resolve(typescriptDirectory));
const files = ['sdk.d.ts'];
const program = ts.createProgram(files.map(file => path.join(sdkDirectory, file)), {
  strictNullChecks: true, skipLibCheck: true, target: ts.ScriptTarget.ESNext,
  module: ts.ModuleKind.NodeNext, moduleResolution: ts.ModuleResolutionKind.NodeNext,
  typeRoots: [path.resolve(typescriptDirectory, '../@types')],
});
const checker = program.getTypeChecker();
// Check the shared Rust fixtures against the original TS declarations too.
// The virtual TS file exists only in the compiler host, outside the repository.
const fixturePath = path.join(sdkDirectory, '__farcaster_fixture_check.ts');
const fixtures = JSON.parse(fs.readFileSync('crates/claude-sdk-types/fixtures/protocol.json', 'utf8'));
const fixtureSource = `import type * as sdk from './sdk.d.ts';\n` + fixtures.map((fixture, i) => {
  return `const fixture${i} = ${JSON.stringify(fixture.value)} satisfies sdk.${fixture.rust_type};`;
}).join('\n');
const compilerHost = ts.createCompilerHost(program.getCompilerOptions());
const originalGetSourceFile = compilerHost.getSourceFile.bind(compilerHost);
compilerHost.getSourceFile = (fileName, ...rest) => fileName === fixturePath
  ? ts.createSourceFile(fileName, fixtureSource, ts.ScriptTarget.ESNext, true)
  : originalGetSourceFile(fileName, ...rest);
const fixtureProgram = ts.createProgram([fixturePath], program.getCompilerOptions(), compilerHost);
const diagnostics = ts.getPreEmitDiagnostics(fixtureProgram);
if (diagnostics.length) throw new Error(ts.formatDiagnosticsWithColorAndContext(diagnostics, {
  getCanonicalFileName: name => name, getCurrentDirectory: () => process.cwd(), getNewLine: () => '\n',
}));
const declarations = [];
for (const file of files) {
  const source = program.getSourceFile(path.join(sdkDirectory, file));
  for (const node of source.statements) {
    if (ts.isTypeAliasDeclaration(node) || ts.isInterfaceDeclaration(node) || ts.isClassDeclaration(node)) {
      declarations.push({ file, node, name: node.name.text, type: checker.getTypeAtLocation(node) });
    }
  }
}
const F = ts.TypeFlags;
function primitiveIntersection(type) {
  return type.isIntersection() ? type.types.find(t => t.flags & (F.StringLike | F.NumberLike | F.BooleanLike)) : undefined;
}

// Import the transitive JSON contract for CLI stdin/stdout, not every SDK API.
// Success response envelopes use Record<string, unknown>, so their named bodies
// and callback replies must be explicit roots rather than discovered from it.
const roots = [
  'StdoutMessage', 'SDKMessage', 'SDKUserMessage',
  'SDKControlRequest', 'SDKControlResponse',
  ...declarations.filter(d => /^SDKControl.+Response$/.test(d.name)).map(d => d.name),
  'PermissionResult', 'HookJSONOutput', 'ElicitationResult', 'UserDialogResult',
  'McpServerStatus', 'McpSetServersResult', 'RewindFilesResult',
];
const reachable = new Set();
function visit(type) {
  if (reachable.has(type.id)) return;
  reachable.add(type.id);
  if (primitiveIntersection(type)) return;
  if (type.getCallSignatures().length || type.getConstructSignatures().length ||
      type.flags & (F.TypeParameter | F.UniqueESSymbol)) {
    throw new Error(`Non-JSON type in CLI protocol: ${checker.typeToString(type)}`);
  }
  if (type.isUnion()) type.types.forEach(visit);
  else if (checker.isArrayType(type) || checker.isTupleType(type)) checker.getTypeArguments(type).forEach(visit);
  else if (type.flags & (F.Object | F.Intersection)) {
    for (const property of checker.getPropertiesOfType(type)) visit(checker.getTypeOfSymbol(property));
    for (const index of checker.getIndexInfosOfType(type)) visit(index.type);
  }
}
for (const name of roots) {
  const declaration = declarations.find(d => d.name === name);
  if (!declaration) throw new Error(`Missing CLI root: ${name}`);
  visit(declaration.type);
}
const selected = declarations.filter(d => reachable.has(d.type.id));

const rustName = name => name.replace(/[^a-zA-Z0-9]+/g, '_').replace(/^([0-9])/, 'N$1');
const fieldName = name => {
  const snake = name.replace(/([A-Z]+)([A-Z][a-z])/g, '$1_$2').replace(/([a-z0-9])([A-Z])/g, '$1_$2').replace(/[^a-zA-Z0-9]+/g, '_').toLowerCase();
  if (['self', 'super', 'crate', 'Self'].includes(snake)) return `${snake}_field`;
  if (['type', 'match', 'ref', 'use', 'in', 'loop', 'move', 'mod', 'async', 'await', 'dyn', 'fn', 'pub', 'struct', 'enum', 'impl', 'trait', 'where', 'const', 'static', 'return', 'continue', 'break', 'if', 'else', 'for', 'while', 'let', 'mut', 'as', 'unsafe', 'extern', 'true', 'false', 'box', 'yield', 'try', 'abstract', 'become', 'do', 'final', 'macro', 'override', 'priv', 'typeof', 'unsized', 'virtual'].includes(snake)) return `r#${snake}`;
  return /^[0-9]/.test(snake) ? `n_${snake}` : snake;
};
const variantName = value => {
  const words = value.replace(/([a-z0-9])([A-Z])/g, '$1 $2').split(/[^a-zA-Z0-9]+/).filter(Boolean);
  let name = words.map(word => word[0].toUpperCase() + word.slice(1).toLowerCase()).join('') || 'Empty';
  if (/^[0-9]/.test(name)) name = 'N' + name;
  if (name === 'Self') name = 'SelfValue';
  return name;
};
const typeNames = new Map();
const usedNames = new Set();
function allocateName(proposed) {
  const base = rustName(proposed);
  let name = base;
  for (let suffix = 2; usedNames.has(name); suffix++) name = `${base}${suffix}`;
  usedNames.add(name);
  return name;
}
for (const declaration of selected) {
  declaration.rust = allocateName(declaration.name);
  if (!typeNames.has(declaration.type.id)) typeNames.set(declaration.type.id, declaration.rust);
}
const pending = [];
const emitted = new Set();
const output = [];
const jsonSlots = [];
function withoutUndefined(type) {
  if (!type.isUnion()) return type;
  const members = type.types.filter(t => !(t.flags & F.Undefined));
  return members.length === 1 ? members[0] : checker.getUnionType(members);
}
function named(type, hint) {
  if (typeNames.has(type.id)) return typeNames.get(type.id);
  const symbolName = type.aliasSymbol?.getName() ?? type.getSymbol()?.getName();
  const name = allocateName(symbolName && !symbolName.startsWith('__') && !['Array', 'ReadonlyArray'].includes(symbolName) ? symbolName : hint);
  typeNames.set(type.id, name);
  pending.push({type, name});
  return name;
}
function rustType(type, hint, defining = false) {
  if (!defining && typeNames.has(type.id)) return `Box<${typeNames.get(type.id)}>`;
  if (primitiveIntersection(type)) return rustType(primitiveIntersection(type), hint, defining);
  if (type.flags & (F.Any | F.Unknown)) {
    if (type.intrinsicName === 'error') throw new Error(`Unresolved TypeScript type at ${hint}`);
    jsonSlots.push({path: hint, source: checker.typeToString(type)});
    return 'serde_json::Value';
  }
  if (type.flags & F.String) return 'String';
  if (type.flags & F.Number) return 'serde_json::Number';
  if (type.flags & F.Boolean) return 'bool';
  if (type.flags & F.Null) return '()';
  if (type.flags & F.TemplateLiteral) {
    // UUID is a string on the wire. No format constraint exists in the JSON protocol.
    return 'String';
  }
  if (checker.isArrayType(type)) return `Vec<${rustType(checker.getTypeArguments(type)[0], hint + 'Item')}>`;
  if (checker.isTupleType(type)) {
    const types = checker.getTypeArguments(type);
    if (type.target.elementFlags.some(flag => flag !== ts.ElementFlags.Required)) throw new Error(`Unsupported optional/rest tuple at ${hint}: ${checker.typeToString(type)}`);
    return `(${types.map((t, i) => rustType(t, hint + 'Item' + i)).join(', ')},)`;
  }
  if (type.isUnion()) {
    const defined = type.types.filter(t => !(t.flags & F.Undefined));
    if (defined.length !== type.types.length) return rustType(defined.length === 1 ? defined[0] : checker.getUnionType(defined), hint + 'Defined');
    const nonNull = type.types.filter(t => !(t.flags & F.Null));
    if (nonNull.length !== type.types.length) {
      const inner = nonNull.length === 1 ? nonNull[0] : checker.getUnionType(nonNull);
      return `Option<${rustType(inner, hint + 'Value')}>`;
    }
    if (type.types.every(t => t.flags & F.BooleanLiteral)) return 'bool';
  }
  if (type.flags & (F.StringLiteral | F.NumberLiteral | F.BooleanLiteral | F.Union | F.Object | F.Intersection)) {
    return `Box<${named(type, hint)}>`;
  }
  throw new Error(`Unsupported type ${checker.typeToString(type)} (${type.flags}) at ${hint}`);
}
function literal(type) {
  if (type.flags & (F.StringLiteral | F.NumberLiteral)) return type.value;
  if (type.flags & F.BooleanLiteral) return type.intrinsicName === 'true';
  return undefined;
}
function specificity(type) {
  return checker.getPropertiesOfType(type).reduce((score, property) => {
    if (property.flags & ts.SymbolFlags.Optional) return score;
    const ptype = checker.getTypeOfSymbol(property);
    return score + (literal(ptype) !== undefined ? 100 : 1);
  }, 0);
}
function emit(type, name) {
  if (emitted.has(name)) return;
  emitted.add(name);
  const canonical = typeNames.get(type.id);
  if (canonical && canonical !== name) {
    output.push(`pub type ${name} = ${canonical};`);
    return;
  }
  const value = literal(type);
  if (typeof value === 'string' || (type.isUnion() && type.types.every(t => t.flags & F.StringLiteral))) {
    const values = type.isUnion() ? type.types.map(t => t.value) : [value];
    const seen = new Set();
    output.push(`#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]\npub enum ${name} {\n${values.map((v, i) => {
      let variant = variantName(v);
      if (seen.has(variant)) variant += i;
      seen.add(variant);
      return `    #[serde(rename = ${JSON.stringify(v)})]\n    ${variant},`;
    }).join('\n')}\n}`);
  } else if (value !== undefined) {
    output.push(`crate::literal_type!(${name}, ${JSON.stringify(value)});`);
  } else if (type.isUnion() && !type.types.some(t => t.flags & (F.Null | F.Undefined)) && !type.types.every(t => t.flags & F.BooleanLiteral)) {
    const variants = [...type.types].sort((a, b) => specificity(b) - specificity(a));
    const variantNames = new Set();
    const fields = variants.map((t, i) => {
      const ty = rustType(t, name + 'Variant' + i);
      let variant = ty.startsWith('Box<') ? ty.slice(4, -1) : `V${i}`;
      if (variant === name || variantNames.has(variant)) variant = `V${i}`;
      variantNames.add(variant);
      return `    ${variant}(${ty}),`;
    });
    output.push(`#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]\n#[serde(untagged)]\npub enum ${name} {\n${fields.join('\n')}\n}`);
  } else if ((type.flags & (F.Object | F.Intersection)) && !primitiveIntersection(type) && !checker.isArrayType(type) && !checker.isTupleType(type)) {
    const props = checker.getPropertiesOfType(type);
    const indexes = checker.getIndexInfosOfType(type);
    if (indexes.length > 1 && indexes.some(i => i.type.id !== indexes[0].type.id)) throw new Error(`Conflicting index signatures at ${name}`);
    const fields = props.map(property => {
      const key = property.getName();
      const optional = !!(property.flags & ts.SymbolFlags.Optional);
      const propertyType = checker.getTypeOfSymbol(property);
      let ty = rustType(optional ? withoutUndefined(propertyType) : propertyType, name + '_' + key);
      const field = fieldName(key);
      let attrs = field.replace(/^r#/, '') === key ? '' : `    #[serde(rename = ${JSON.stringify(key)})]\n`;
      if (optional) {
        attrs += '    #[serde(default, skip_serializing_if = "crate::Presence::is_missing")]\n';
        ty = `crate::Presence<${ty}>`;
      } else {
        // serde_json::Value and Option accept absent fields by default; the SDK
        // requires the property itself even when its value can be null.
        attrs += '    #[serde(deserialize_with = "crate::required")]\n';
        // The public API declares nullable metadata as required, while CLI and
        // provider streams omit it. Result usage also drops optional details
        // despite the SDK's NonNullable mapping. Keep strict types by default;
        // opt-in CLI decoding preserves missing metadata without fabricating it.
        const apiMetadata = property.declarations?.some(declaration =>
          declaration.getSourceFile().fileName.includes('/@anthropic-ai/sdk/'));
        // A thinking block starts before its signature_delta arrives.
        if ((apiMetadata && ty.startsWith('Option<')) ||
            (name === 'BetaThinkingBlock' && key === 'signature') ||
            (name === 'NonNullableUsage' && !['input_tokens', 'output_tokens'].includes(key))) {
          attrs += '    #[cfg_attr(feature = "cli-compat", serde(default, skip_serializing_if = "crate::Presence::is_missing"))]\n';
          ty = `crate::WireMetadata<${ty}>`;
        }
      }
      return `${attrs}    pub ${field}: ${ty},`;
    });
    const extraType = indexes.length ? rustType(indexes[0].type, name + 'Extra') : 'serde_json::Value';
    let extraName = 'extra';
    while (props.some(p => fieldName(p.getName()) === extraName)) extraName += '_';
    fields.push(`    #[serde(flatten)]\n    pub ${extraName}: std::collections::BTreeMap<String, ${extraType}>,`);
    output.push(`#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]\npub struct ${name} {\n${fields.join('\n')}\n}`);
  } else {
    const ty = rustType(type, name, true);
    if (ty === `Box<${name}>`) throw new Error(`Recursive alias ${name}`);
    output.push(`pub type ${name} = ${ty};`);
  }
}
for (const declaration of selected) pending.push({type: declaration.type, name: declaration.rust});
for (let i = 0; i < pending.length; i++) emit(pending[i].type, pending[i].name);

const packageJson = JSON.parse(fs.readFileSync(path.join(sdkDirectory, 'package.json'), 'utf8'));
const dependencyPackages = new Map();
for (const source of program.getSourceFiles()) {
  let directory = path.dirname(source.fileName);
  while (directory !== path.dirname(directory)) {
    const packageFile = path.join(directory, 'package.json');
    if (fs.existsSync(packageFile)) {
      const dependency = JSON.parse(fs.readFileSync(packageFile, 'utf8'));
      if (dependency.name && dependency.version) {
        dependencyPackages.set(dependency.name, dependency.version);
        break;
      }
    }
    directory = path.dirname(directory);
  }
}
const report = {
  package: packageJson.name, version: packageJson.version, claudeCodeVersion: packageJson.claudeCodeVersion,
  upstream: packageJson.repository.url, upstreamReferenceRevision: upstreamRevision,
  note: 'CLI stdin/stdout message types and named control/callback response bodies, plus transitive dependencies. The public repository does not contain these declarations; the package hash pins the actual source.',
  roots,
  typescriptVersion: ts.version,
  dependencyVersions: Object.fromEntries([...dependencyPackages].sort(([a], [b]) => a.localeCompare(b))),
  sourceHashes: Object.fromEntries(files.map(file => [file, crypto.createHash('sha256').update(fs.readFileSync(path.join(sdkDirectory, file))).digest('hex')])),
  declarations: selected.map(d => ({source: d.file, line: d.node.getSourceFile().getLineAndCharacterOfPosition(d.node.getStart()).line + 1, name: d.name, status: 'generated', rust: d.rust})),
  generatedTypes: emitted.size,
  openJsonSlots: jsonSlots,
};
const artifacts = {
  'crates/claude-sdk-types/src/generated.rs': '// Generated by scripts/import_claude_sdk_types.mjs. Do not edit by hand.\n// Anthropic Claude Agent SDK ' + packageJson.version + '; see ../source-manifest.json.\n#![allow(non_camel_case_types, clippy::large_enum_variant)]\n\n' + `pub const SDK_VERSION: &str = ${JSON.stringify(packageJson.version)};\npub const CLAUDE_CODE_VERSION: &str = ${JSON.stringify(packageJson.claudeCodeVersion)};\n\n` + output.join('\n\n') + '\n',
  'crates/claude-sdk-types/source-manifest.json': JSON.stringify(report, null, 2) + '\n',
};
artifacts['crates/claude-sdk-types/src/generated.rs'] = execFileSync('rustfmt', ['--edition', '2024', '--emit', 'stdout'], {
  input: artifacts['crates/claude-sdk-types/src/generated.rs'], encoding: 'utf8', maxBuffer: 32 * 1024 * 1024,
});
if (process.argv.includes('--check')) {
  for (const [file, contents] of Object.entries(artifacts)) {
    if (fs.readFileSync(file, 'utf8') !== contents) throw new Error(`Generated file is stale: ${file}`);
  }
  console.log(`Verified ${selected.length} CLI declaration entries and ${fixtures.length} TypeScript fixtures; generated files match.`);
  process.exit(0);
}
// File changes go through apply_patch, including regeneration of checked-in files.
let patch = '*** Begin Patch\n';
for (const [file, contents] of Object.entries(artifacts)) {
  if (fs.existsSync(file)) patch += `*** Delete File: ${file}\n`;
  patch += `*** Add File: ${file}\n${contents.trimEnd().split('\n').map(line => '+' + line).join('\n')}\n`;
}
process.stdout.write(patch + '*** End Patch\n');
