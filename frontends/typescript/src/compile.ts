import { createHash } from "node:crypto";

import ts from "typescript";

import { canonicalStringify } from "./canonical.ts";
import {
  ENGINE_FORMAT_VERSION,
  FRONTEND_ID,
  FRONTEND_VERSION,
  LANGUAGE_SEMANTICS_VERSION,
  type Artifact,
  type CompileOptions,
  type FunctionDecl,
  type Instruction,
} from "./types.ts";

const SDK_SPECIFIER = "@trigora/sdk";
const SDK_PATH = "/__tcc/@trigora/sdk/index.d.ts";
const INPUT_PATH = "/__tcc/input.ts";

const SDK_SOURCE = `export declare function effect<T>(key: string, fn: () => T | Promise<T>): Promise<T>;
export declare function waitForEvent(name: string): Promise<unknown>;
`;

export class CompileError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CompileError";
  }
}

/** Compile supported TypeScript into a TCC artifact. */
export function compile(source: string, options: CompileOptions = {}): Artifact {
  const filename = options.filename ?? "input.ts";
  const { program, sourceFile, checker } = createProgram(source);
  const diagnostics = program.getSyntacticDiagnostics(sourceFile);
  if (diagnostics.length > 0) {
    throw new CompileError(formatDiagnostic(diagnostics[0]!, filename));
  }

  const entry = findDefaultExport(sourceFile);
  const functionDecl = lowerFunction(entry, sourceFile, checker, filename);
  const artifact: Artifact = {
    envelope: {
      artifact_hash: "",
      frontend_id: FRONTEND_ID,
      frontend_version: FRONTEND_VERSION,
      language_semantics_version: LANGUAGE_SEMANTICS_VERSION,
      engine_format_version: ENGINE_FORMAT_VERSION,
      required_engine_features: [
        "ts.control_flow",
        "durable.effect",
        "durable.wait_for_event",
      ],
      required_host_capabilities: [
        "host.persist_checkpoint",
        "host.effect",
        "host.event",
      ],
      runtime_modules: [],
    },
    program: {
      entry: 0,
      functions: [functionDecl],
    },
  };
  artifact.envelope.artifact_hash = hashArtifact(artifact);
  return artifact;
}

function hashArtifact(artifact: Artifact): string {
  const copy: Artifact = structuredClone(artifact);
  copy.envelope.artifact_hash = "";
  return createHash("sha256").update(canonicalStringify(copy)).digest("hex");
}

function createProgram(source: string): {
  program: ts.Program;
  sourceFile: ts.SourceFile;
  checker: ts.TypeChecker;
} {
  const files = new Map<string, string>([
    [INPUT_PATH, source],
    [SDK_PATH, SDK_SOURCE],
  ]);
  const compilerOptions: ts.CompilerOptions = {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext,
    strict: true,
    noEmit: true,
    skipLibCheck: true,
    paths: {
      [SDK_SPECIFIER]: [SDK_PATH],
    },
    baseUrl: "/",
  };
  const host = ts.createCompilerHost(compilerOptions, true);
  const defaultGetSourceFile = host.getSourceFile.bind(host);
  const defaultFileExists = host.fileExists.bind(host);
  const defaultReadFile = host.readFile.bind(host);
  host.fileExists = (path) => files.has(normalize(path)) || defaultFileExists(path);
  host.readFile = (path) => files.get(normalize(path)) ?? defaultReadFile(path);
  host.getCurrentDirectory = () => "/";
  host.getSourceFile = (fileName, languageVersion, onError, shouldCreateNewSourceFile) => {
    const virtual = files.get(normalize(fileName));
    if (virtual !== undefined) {
      return ts.createSourceFile(
        fileName,
        virtual,
        languageVersion,
        true,
        fileName.endsWith(".d.ts") ? ts.ScriptKind.TS : ts.ScriptKind.TS,
      );
    }
    return defaultGetSourceFile(fileName, languageVersion, onError, shouldCreateNewSourceFile);
  };
  const program = ts.createProgram([INPUT_PATH], compilerOptions, host);
  const sourceFile = program.getSourceFile(INPUT_PATH);
  if (!sourceFile) {
    throw new CompileError("failed to parse source");
  }
  return { program, sourceFile, checker: program.getTypeChecker() };
}

function normalize(path: string): string {
  return path.replace(/\\/g, "/");
}

function findDefaultExport(sourceFile: ts.SourceFile): ts.FunctionDeclaration {
  let entry: ts.FunctionDeclaration | undefined;
  for (const statement of sourceFile.statements) {
    if (ts.isImportDeclaration(statement)) {
      const spec = importSpecifier(statement);
      if (spec !== SDK_SPECIFIER) {
        throw new CompileError(`unsupported import from \`${spec ?? "?"}\``);
      }
      continue;
    }
    if (
      ts.isFunctionDeclaration(statement) &&
      hasModifier(statement, ts.SyntaxKind.ExportKeyword) &&
      hasModifier(statement, ts.SyntaxKind.DefaultKeyword)
    ) {
      if (entry) {
        throw new CompileError("multiple default exports");
      }
      entry = statement;
      continue;
    }
    throw new CompileError(`unsupported top-level statement: ${kindName(statement)}`);
  }
  if (!entry) {
    throw new CompileError("missing default export async function");
  }
  if (!hasModifier(entry, ts.SyntaxKind.AsyncKeyword)) {
    throw new CompileError("default export must be an async function");
  }
  if (entry.parameters.length > 0) {
    throw new CompileError("entry function must not take parameters");
  }
  if (!entry.body) {
    throw new CompileError("entry function is missing a body");
  }
  return entry;
}

function lowerFunction(
  entry: ts.FunctionDeclaration,
  sourceFile: ts.SourceFile,
  checker: ts.TypeChecker,
  filename: string,
): FunctionDecl {
  const locals = new Map<string, number>();
  const instructions: Instruction[] = [];
  const spans: FunctionDecl["spans"] = [];

  const emit = (instruction: Instruction, node: ts.Node) => {
    instructions.push(instruction);
    spans.push(spanOf(sourceFile, filename, node));
  };

  for (const statement of entry.body!.statements) {
    if (ts.isVariableStatement(statement)) {
      if (statement.declarationList.declarations.length !== 1) {
        throw new CompileError("declare one local per statement");
      }
      const declaration = statement.declarationList.declarations[0]!;
      if (!ts.isIdentifier(declaration.name)) {
        throw new CompileError("locals must be simple identifiers");
      }
      if (!declaration.initializer) {
        throw new CompileError(`local \`${declaration.name.text}\` needs an initializer`);
      }
      const local = locals.size;
      locals.set(declaration.name.text, local);
      lowerAwaitInit(declaration.initializer, checker, emit);
      emit({ op: "StoreLocal", local }, declaration);
      continue;
    }
    if (ts.isReturnStatement(statement)) {
      if (!statement.expression) {
        emit({ op: "Return" }, statement);
        continue;
      }
      lowerReturnValue(statement.expression, locals, emit);
      emit({ op: "Return" }, statement);
      continue;
    }
    throw new CompileError(`unsupported statement: ${kindName(statement)}`);
  }

  return {
    id: 0,
    name: entry.name?.text ?? "default",
    param_count: 0,
    local_count: locals.size,
    instructions,
    spans,
  };
}

function lowerAwaitInit(
  expression: ts.Expression,
  checker: ts.TypeChecker,
  emit: (instruction: Instruction, node: ts.Node) => void,
): void {
  if (!ts.isAwaitExpression(expression) || !ts.isCallExpression(expression.expression)) {
    throw new CompileError("locals must be initialized with `await effect(...)` or `await waitForEvent(...)`");
  }
  const call = expression.expression;
  const durable = resolveDurable(call.expression, checker);
  if (durable === "effect") {
    if (call.arguments.length !== 2) {
      throw new CompileError("`effect` takes a key and a callback");
    }
    const key = stringLiteral(call.arguments[0]!, "effect key");
    emit({ op: "LoadConst", value: { t: "string", v: key } }, call);
    emit({ op: "Effect" }, call);
    return;
  }
  if (durable === "waitForEvent") {
    if (call.arguments.length !== 1) {
      throw new CompileError("`waitForEvent` takes an event name");
    }
    const name = stringLiteral(call.arguments[0]!, "event name");
    emit({ op: "LoadConst", value: { t: "string", v: name } }, call);
    emit({ op: "WaitForEvent" }, call);
    return;
  }
  throw new CompileError("call is not a resolved `@trigora/sdk` durable operation");
}

function lowerReturnValue(
  expression: ts.Expression,
  locals: Map<string, number>,
  emit: (instruction: Instruction, node: ts.Node) => void,
): void {
  if (ts.isIdentifier(expression)) {
    emit({ op: "LoadLocal", local: localId(locals, expression.text) }, expression);
    return;
  }
  if (ts.isObjectLiteralExpression(expression)) {
    emit({ op: "NewObject" }, expression);
    for (const property of expression.properties) {
      if (!ts.isShorthandPropertyAssignment(property) && !ts.isPropertyAssignment(property)) {
        throw new CompileError("object returns support identifier properties only");
      }
      const key = property.name.getText();
      if (!ts.isIdentifier(property.name)) {
        throw new CompileError("object keys must be identifiers");
      }
      const valueExpr = ts.isShorthandPropertyAssignment(property)
        ? property.name
        : property.initializer;
      if (!ts.isIdentifier(valueExpr)) {
        throw new CompileError("object values must be locals");
      }
      emit({ op: "LoadLocal", local: localId(locals, valueExpr.text) }, valueExpr);
      emit({ op: "SetProp", key }, property);
    }
    return;
  }
  throw new CompileError("return a local or a shallow object of locals");
}

function resolveDurable(
  expression: ts.Expression,
  checker: ts.TypeChecker,
): "effect" | "waitForEvent" | undefined {
  const symbol = checker.getSymbolAtLocation(expression);
  if (!symbol) {
    return undefined;
  }
  const resolved = symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
  const name = resolved.getName();
  if (name !== "effect" && name !== "waitForEvent") {
    return undefined;
  }
  const declaration = resolved.getDeclarations()?.[0];
  if (!declaration) {
    return undefined;
  }
  const file = declaration.getSourceFile().fileName.replace(/\\/g, "/");
  if (!file.endsWith("@trigora/sdk/index.d.ts") && file !== SDK_PATH) {
    return undefined;
  }
  return name;
}

function localId(locals: Map<string, number>, name: string): number {
  const id = locals.get(name);
  if (id === undefined) {
    throw new CompileError(`unknown local \`${name}\``);
  }
  return id;
}

function stringLiteral(node: ts.Expression, label: string): string {
  if (!ts.isStringLiteral(node) && !ts.isNoSubstitutionTemplateLiteral(node)) {
    throw new CompileError(`${label} must be a string literal`);
  }
  return node.text;
}

function importSpecifier(statement: ts.ImportDeclaration): string | undefined {
  const spec = statement.moduleSpecifier;
  if (ts.isStringLiteral(spec)) {
    return spec.text;
  }
  return undefined;
}

function hasModifier(node: ts.HasModifiers, kind: ts.SyntaxKind): boolean {
  return Boolean(ts.getModifiers(node)?.some((modifier) => modifier.kind === kind));
}

function kindName(node: ts.Node): string {
  return ts.SyntaxKind[node.kind] ?? "unknown";
}

function spanOf(
  sourceFile: ts.SourceFile,
  filename: string,
  node: ts.Node,
): FunctionDecl["spans"][number] {
  const start = sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile));
  const end = sourceFile.getLineAndCharacterOfPosition(node.getEnd());
  return {
    file: filename,
    start_line: start.line + 1,
    start_column: start.character + 1,
    end_line: end.line + 1,
    end_column: end.character + 1,
  };
}

function formatDiagnostic(diagnostic: ts.Diagnostic, filename: string): string {
  const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n");
  return `${filename}: ${message}`;
}
