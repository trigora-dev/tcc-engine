import { createHash } from "node:crypto";

import ts from "typescript";

import { canonicalStringify } from "./canonical.ts";
import { analyzeCaptures, type FnRec } from "./closures.ts";
import {
  ENGINE_FORMAT_VERSION,
  FRONTEND_ID,
  FRONTEND_VERSION,
  LANGUAGE_SEMANTICS_VERSION,
  type Artifact,
  type CompileOptions,
  type ConstValue,
  type EngineFeature,
  type FunctionDecl,
  type HostCapability,
  type Instruction,
} from "./types.ts";

const SDK_SPECIFIER = "@trigora/sdk";
const PRIMITIVES_SPECIFIER = "@tcc-engine/primitives";
const DURABLE_SPECIFIERS = new Set([SDK_SPECIFIER, PRIMITIVES_SPECIFIER]);
const SDK_PATH = "/__tcc/@trigora/sdk/index.d.ts";
const PRIMITIVES_PATH = "/__tcc/@tcc-engine/primitives/index.d.ts";
const INPUT_PATH = "/__tcc/input.ts";

const DURABLE_SOURCE = `export declare function effect<T>(key: string, fn: () => T | Promise<T>): Promise<T>;
export declare function waitForEvent(name: string): Promise<unknown>;
export declare function sleep(ms: number): Promise<void>;
export declare function invoke(name: string, ...args: unknown[]): Promise<unknown>;
`;

type DurableName = "effect" | "waitForEvent" | "sleep" | "invoke";

type Binding = { slot: number; kind: "const" | "let"; cell: boolean; outerIndex?: number };

type Loop = {
  breaks: number[];
  continues: number[];
  continueTarget?: number;
  unwatch: boolean;
};

type FnInfo = { id: number; paramCount: number; closure: boolean };

export type DiagnosticSpan = {
  file: string;
  start_line: number;
  start_column: number;
  end_line?: number;
  end_column?: number;
};

export class CompileError extends Error {
  readonly file: string;
  readonly span: DiagnosticSpan | null;
  readonly why: string | null;
  readonly alternative: string | null;
  readonly frontendId = FRONTEND_ID;
  readonly frontendVersion = FRONTEND_VERSION;
  readonly languageSemanticsVersion = LANGUAGE_SEMANTICS_VERSION;

  constructor(
    message: string,
    file = "input.ts",
    span: DiagnosticSpan | null = null,
    options: { why?: string; alternative?: string } = {},
  ) {
    super(message);
    this.name = "CompileError";
    this.file = file;
    this.span = span;
    this.why = options.why ?? null;
    this.alternative = options.alternative ?? null;
  }
}

const WHY_SUBSET = "it cannot cross a durable checkpoint in the current TypeScript subset";

/** Compile supported TypeScript into a TCC artifact. */
export function compile(source: string, options: CompileOptions = {}): Artifact {
  const filename = options.filename ?? "input.ts";
  const { program, sourceFile, checker } = createProgram(source);
  const diagnostics = program.getSyntacticDiagnostics(sourceFile);
  if (diagnostics.length > 0) {
    const info = diagnosticInfo(diagnostics[0]!, filename);
    throw new CompileError(info.message, filename, info.span);
  }

  const { entry, helpers } = collectFunctions(sourceFile, filename);
  const analysis = analyzeCaptures(sourceFile, entry, helpers);
  const functions = new Map<string, FnInfo>();
  helpers.forEach((helper) => {
    const rec = analysis.recs.get(helper)!;
    functions.set(helper.name!.text, {
      id: rec.id,
      paramCount: rec.paramCount,
      closure: rec.closure,
    });
  });
  const lowered = [entry, ...helpers].map((fn) =>
    lowerFunction(fn, sourceFile, checker, filename, analysis.recs.get(fn)!, functions, analysis.recs, entry),
  );
  const arrows = [...analysis.recs.values()].filter((rec) => ts.isArrowFunction(rec.node));
  const loweredArrows = arrows.map((rec) =>
    lowerFunction(rec.node, sourceFile, checker, filename, rec, functions, analysis.recs, entry),
  );
  const all = [...lowered, ...loweredArrows].sort((left, right) => left.id - right.id);
  const { engine, host } = requiredFrom(all.flatMap((item) => item.instructions));
  const artifact: Artifact = {
    envelope: {
      artifact_hash: "",
      frontend_id: FRONTEND_ID,
      frontend_version: FRONTEND_VERSION,
      language_semantics_version: LANGUAGE_SEMANTICS_VERSION,
      engine_format_version: ENGINE_FORMAT_VERSION,
      required_engine_features: engine,
      required_host_capabilities: host,
      runtime_modules: [],
    },
    program: {
      entry: analysis.recs.get(entry)!.id,
      functions: all,
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
    [SDK_PATH, DURABLE_SOURCE],
    [PRIMITIVES_PATH, DURABLE_SOURCE],
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
      [PRIMITIVES_SPECIFIER]: [PRIMITIVES_PATH],
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

function collectFunctions(
  sourceFile: ts.SourceFile,
  filename: string,
): { entry: ts.FunctionDeclaration; helpers: ts.FunctionDeclaration[] } {
  let entry: ts.FunctionDeclaration | undefined;
  const helpers: ts.FunctionDeclaration[] = [];
  for (const statement of sourceFile.statements) {
    if (ts.isImportDeclaration(statement)) {
      const spec = importSpecifier(statement);
      if (!spec || !DURABLE_SPECIFIERS.has(spec)) {
        fail(filename, sourceFile, statement, `unsupported import from \`${spec ?? "?"}\``, WHY_SUBSET);
      }
      continue;
    }
    if (
      ts.isFunctionDeclaration(statement) &&
      hasModifier(statement, ts.SyntaxKind.ExportKeyword) &&
      hasModifier(statement, ts.SyntaxKind.DefaultKeyword)
    ) {
      if (entry) {
        fail(filename, sourceFile, statement, "multiple default exports", WHY_SUBSET);
      }
      entry = statement;
      continue;
    }
    if (ts.isFunctionDeclaration(statement) && statement.name && !hasModifier(statement, ts.SyntaxKind.ExportKeyword)) {
      if (hasModifier(statement, ts.SyntaxKind.AsyncKeyword)) {
        fail(filename, sourceFile, statement, "helper functions cannot be async", WHY_SUBSET, "keep durable operations in the program entry");
      }
      if (!statement.body) {
        fail(filename, sourceFile, statement, "helper function is missing a body", WHY_SUBSET);
      }
      helpers.push(statement);
      continue;
    }
    fail(
      filename,
      sourceFile,
      statement,
      `unsupported top-level statement: ${kindName(statement)}`,
      WHY_SUBSET,
    );
  }
  if (!entry) {
    throw new CompileError("missing default export async function", filename, {
      file: filename,
      start_line: 1,
      start_column: 1,
    }, { why: WHY_SUBSET });
  }
  if (!hasModifier(entry, ts.SyntaxKind.AsyncKeyword)) {
    fail(filename, sourceFile, entry, "default export must be an async function", WHY_SUBSET);
  }
  if (!entry.body) {
    fail(filename, sourceFile, entry, "entry function is missing a body", WHY_SUBSET);
  }
  return { entry, helpers };
}

function bindEntryParams(
  entry: ts.FunctionDeclaration | ts.ArrowFunction,
  lower: Lowerer,
  sourceFile: ts.SourceFile,
  filename: string,
): number {
  const names: string[] = [];
  for (const param of entry.parameters) {
    if (param.dotDotDotToken || param.questionToken || !ts.isIdentifier(param.name)) {
      fail(filename, sourceFile, param, "parameter must be a plain identifier", WHY_SUBSET, "use an earlier parameter or a literal");
    }
    names.push(param.name.text);
    lower.declare(param.name.text, "let");
  }
  entry.parameters.forEach((param, index) => {
    if (!param.initializer || !ts.isIdentifier(param.name)) {
      return;
    }
    const earlier = new Set(names.slice(0, index));
    assertEarlierOnly(param.initializer, earlier, filename, sourceFile);
    const slot = lower.lookup(param.name.text).slot;
    lower.expression(param.name);
    lower.emit({ op: "LoadConst", value: { t: "undefined" } }, param);
    lower.emit({ op: "StrictEq" }, param);
    const skip = lower.emit({ op: "JumpIfFalse", target: 0 }, param);
    lower.expression(param.initializer);
    lower.emit({ op: "StoreLocal", local: slot }, param);
    lower.patch(skip, lower.pc());
  });
  return entry.parameters.length;
}

function assertEarlierOnly(
  node: ts.Node,
  earlier: Set<string>,
  filename: string,
  sourceFile: ts.SourceFile,
): void {
  if (ts.isIdentifier(node) && node.text !== "undefined" && !earlier.has(node.text)) {
    fail(
      filename,
      sourceFile,
      node,
      "a default may use only earlier parameters",
      WHY_SUBSET,
    );
  }
  ts.forEachChild(node, (child) => assertEarlierOnly(child, earlier, filename, sourceFile));
}

function lowerFunction(
  fn: ts.FunctionDeclaration | ts.ArrowFunction,
  sourceFile: ts.SourceFile,
  checker: ts.TypeChecker,
  filename: string,
  rec: FnRec,
  functions: Map<string, FnInfo>,
  recs: Map<ts.Node, FnRec>,
  programEntry: ts.Node,
): FunctionDecl {
  const lower = new Lowerer(sourceFile, checker, filename, rec.node === programEntry, functions, rec, recs);
  lower.pushScope();
  if (rec.closure) {
    lower.alloc();
  }
  rec.captured.forEach((captured, index) => lower.installOuter(captured.name, captured.kind, index));
  const paramCount = bindEntryParams(fn, lower, sourceFile, filename);
  for (const param of fn.parameters) {
    if (ts.isIdentifier(param.name) && rec.capturedLocals.has(param.name)) {
      lower.boxExisting(param.name.text, param);
    }
  }
  const body = fn.body;
  if (body && ts.isBlock(body)) {
    for (const statement of body.statements) {
      lower.statement(statement);
    }
  } else if (body) {
    lower.expression(body);
    lower.emit({ op: "Return" }, fn);
  }
  if (lower.needsImplicitReturn()) {
    lower.emit({ op: "LoadConst", value: { t: "undefined" } }, fn);
    lower.emit({ op: "Return" }, fn);
  }
  lower.popScope();
  lower.seal();
  return {
    id: rec.id,
    name: rec.name,
    param_count: paramCount,
    local_count: lower.maxSlots,
    instructions: lower.instructions,
    spans: lower.spans,
  };
}

class Lowerer {
  readonly instructions: Instruction[] = [];
  readonly spans: FunctionDecl["spans"] = [];
  readonly scopes: Array<Map<string, Binding>> = [];
  readonly loops: Loop[] = [];
  readonly sourceFile: ts.SourceFile;
  readonly checker: ts.TypeChecker;
  readonly filename: string;
  readonly allowDurable: boolean;
  readonly functions: Map<string, FnInfo>;
  readonly fn: FnRec;
  readonly recs: Map<ts.Node, FnRec>;
  nextSlot = 0;
  maxSlots = 0;

  constructor(
    sourceFile: ts.SourceFile,
    checker: ts.TypeChecker,
    filename: string,
    durable: boolean,
    functions: Map<string, FnInfo>,
    fn: FnRec,
    recs: Map<ts.Node, FnRec>,
  ) {
    this.sourceFile = sourceFile;
    this.checker = checker;
    this.filename = filename;
    this.allowDurable = durable;
    this.functions = functions;
    this.fn = fn;
    this.recs = recs;
  }

  fail(node: ts.Node, message: string, why?: string, alternative?: string): never {
    fail(this.filename, this.sourceFile, node, message, why, alternative);
  }

  pc(): number {
    return this.instructions.length;
  }

  emit(instruction: Instruction, node: ts.Node): number {
    const index = this.instructions.length;
    this.instructions.push(instruction);
    this.spans.push(spanOf(this.sourceFile, this.filename, node));
    return index;
  }

  patch(index: number, target: number): void {
    const instruction = this.instructions[index];
    if (!instruction || !("target" in instruction)) {
      throw new CompileError("internal: patch target missing");
    }
    instruction.target = target;
  }

  pushScope(): void {
    this.scopes.push(new Map());
  }

  popScope(): void {
    if (!this.scopes.pop()) {
      throw new CompileError("internal: scope underflow");
    }
  }

  alloc(): number {
    const slot = this.nextSlot;
    this.nextSlot += 1;
    this.maxSlots = Math.max(this.maxSlots, this.nextSlot);
    return slot;
  }

  declare(name: string, kind: "const" | "let"): number {
    const scope = this.scopes[this.scopes.length - 1];
    if (!scope) {
      throw new CompileError("internal: no scope");
    }
    if (scope.has(name)) {
      throw new CompileError(`duplicate binding \`${name}\``);
    }
    const slot = this.alloc();
    scope.set(name, { slot, kind, cell: false });
    return slot;
  }

  lookup(name: string): Binding {
    for (let index = this.scopes.length - 1; index >= 0; index -= 1) {
      const found = this.scopes[index]?.get(name);
      if (found) {
        return found;
      }
    }
    throw new CompileError(`unknown local \`${name}\``);
  }

  statement(statement: ts.Statement): void {
    if (ts.isEmptyStatement(statement)) {
      return;
    }
    if (ts.isBlock(statement)) {
      this.pushScope();
      for (const child of statement.statements) {
        this.statement(child);
      }
      this.popScope();
      return;
    }
    if (ts.isVariableStatement(statement)) {
      this.variableStatement(statement);
      return;
    }
    if (ts.isExpressionStatement(statement)) {
      this.expressionStatement(statement.expression);
      return;
    }
    if (ts.isReturnStatement(statement)) {
      this.unwatchAll();
      if (!statement.expression) {
        this.emit({ op: "LoadConst", value: { t: "undefined" } }, statement);
      } else {
        this.expression(statement.expression);
      }
      this.emit({ op: "Return" }, statement);
      return;
    }
    if (ts.isIfStatement(statement)) {
      this.ifStatement(statement);
      return;
    }
    if (ts.isWhileStatement(statement)) {
      this.whileStatement(statement);
      return;
    }
    if (ts.isForStatement(statement)) {
      this.forStatement(statement);
      return;
    }
    if (ts.isBreakStatement(statement)) {
      if (statement.label) {
        this.fail(statement, "labeled break is not supported", WHY_SUBSET, "use an unlabeled `break`");
      }
      const loop = this.loops[this.loops.length - 1];
      if (!loop) {
        throw new CompileError("`break` outside a loop");
      }
      if (loop.unwatch) {
        this.emit({ op: "UnwatchIter" }, statement);
      }
      loop.breaks.push(this.emit({ op: "Jump", target: 0 }, statement));
      return;
    }
    if (ts.isContinueStatement(statement)) {
      if (statement.label) {
        this.fail(statement, "labeled continue is not supported", WHY_SUBSET, "use an unlabeled `continue`");
      }
      const loop = this.loops[this.loops.length - 1];
      if (!loop) {
        throw new CompileError("`continue` outside a loop");
      }
      if (loop.continueTarget !== undefined) {
        this.emit({ op: "Jump", target: loop.continueTarget }, statement);
      } else {
        loop.continues.push(this.emit({ op: "Jump", target: 0 }, statement));
      }
      return;
    }
    if (ts.isThrowStatement(statement)) {
      this.expression(statement.expression);
      this.emit({ op: "Throw" }, statement);
      return;
    }
    if (ts.isTryStatement(statement)) {
      this.tryStatement(statement);
      return;
    }
    if (ts.isForInStatement(statement)) {
      this.fail(statement, "for-in is not supported", WHY_SUBSET, "use for-of on an array");
    }
    if (ts.isForOfStatement(statement)) {
      this.forOfStatement(statement);
      return;
    }
    if (ts.isLabeledStatement(statement)) {
      this.fail(statement, "labeled statements are not supported", WHY_SUBSET);
    }
    this.fail(statement, `unsupported statement: ${kindName(statement)}`, WHY_SUBSET);
  }

  variableStatement(statement: ts.VariableStatement): void {
    this.variableList(statement.declarationList);
  }

  variableList(list: ts.VariableDeclarationList): void {
    if (list.declarations.length !== 1) {
      throw new CompileError("declare one local per statement");
    }
    const declaration = list.declarations[0]!;
    if (ts.isArrayBindingPattern(declaration.name)) {
      if (declaration.initializer && isAwaitPromiseAll(declaration.initializer)) {
        this.destructurePromiseAll(list, declaration);
      } else {
        this.destructureArray(list, declaration);
      }
      return;
    }
    if (ts.isObjectBindingPattern(declaration.name)) {
      this.destructureObject(list, declaration);
      return;
    }
    if (!ts.isIdentifier(declaration.name)) {
      throw new CompileError("locals must be simple identifiers");
    }
    if (!declaration.initializer) {
      throw new CompileError(`local \`${declaration.name.text}\` needs an initializer`);
    }
    const kind = (list.flags & ts.NodeFlags.Const) !== 0 ? "const" : "let";
    this.declare(declaration.name.text, kind);
    this.expression(declaration.initializer);
    this.initLocal(declaration.name, declaration);
  }

  expressionStatement(expression: ts.Expression): void {
    this.expression(expression);
    this.emit({ op: "Pop" }, expression);
  }

  ifStatement(statement: ts.IfStatement): void {
    this.expression(statement.expression);
    const jumpFalse = this.emit({ op: "JumpIfFalse", target: 0 }, statement.expression);
    this.statement(statement.thenStatement);
    if (statement.elseStatement) {
      const jumpEnd = this.emit({ op: "Jump", target: 0 }, statement);
      this.patch(jumpFalse, this.pc());
      this.statement(statement.elseStatement);
      this.patch(jumpEnd, this.pc());
    } else {
      this.patch(jumpFalse, this.pc());
    }
  }

  whileStatement(statement: ts.WhileStatement): void {
    const start = this.pc();
    const loop: Loop = { breaks: [], continues: [], continueTarget: start, unwatch: false };
    this.loops.push(loop);
    this.expression(statement.expression);
    const jumpFalse = this.emit({ op: "JumpIfFalse", target: 0 }, statement.expression);
    this.statement(statement.statement);
    this.emit({ op: "Jump", target: start }, statement);
    const end = this.pc();
    this.patch(jumpFalse, end);
    for (const index of loop.breaks) {
      this.patch(index, end);
    }
    this.loops.pop();
  }

  forStatement(statement: ts.ForStatement): void {
    this.pushScope();
    if (statement.initializer) {
      if (ts.isVariableDeclarationList(statement.initializer)) {
        this.variableList(statement.initializer);
      } else {
        this.expressionStatement(statement.initializer);
      }
    }
    const condPc = this.pc();
    const loop: Loop = { breaks: [], continues: [], unwatch: false };
    this.loops.push(loop);
    if (statement.condition) {
      this.expression(statement.condition);
    } else {
      this.emit({ op: "LoadConst", value: { t: "bool", v: true } }, statement);
    }
    const jumpFalse = this.emit({ op: "JumpIfFalse", target: 0 }, statement);
    this.statement(statement.statement);
    const incrPc = this.pc();
    loop.continueTarget = incrPc;
    for (const index of loop.continues) {
      this.patch(index, incrPc);
    }
    if (statement.incrementor) {
      this.expressionStatement(statement.incrementor);
    }
    this.emit({ op: "Jump", target: condPc }, statement);
    const end = this.pc();
    this.patch(jumpFalse, end);
    for (const index of loop.breaks) {
      this.patch(index, end);
    }
    this.loops.pop();
    this.popScope();
  }

  tryStatement(statement: ts.TryStatement): void {
    if (!statement.catchClause && !statement.finallyBlock) {
      throw new CompileError("try requires catch or finally");
    }
    const push = this.emit({ op: "PushTry", catch: 0, finally: null }, statement);
    this.statement(statement.tryBlock);
    this.emit({ op: "PopTry" }, statement);
    const jumpJoin = this.emit({ op: "Jump", target: 0 }, statement);
    const handlerPc = this.pc();
    const pushInstr = this.instructions[push];
    if (pushInstr && pushInstr.op === "PushTry") {
      pushInstr.catch = handlerPc;
    }
    if (statement.catchClause) {
      this.pushScope();
      const variable = statement.catchClause.variableDeclaration;
      if (variable) {
        if (!ts.isIdentifier(variable.name)) {
          throw new CompileError("catch binding must be a simple identifier");
        }
        this.declare(variable.name.text, "let");
        this.initLocal(variable.name, variable);
      } else {
        this.emit({ op: "Pop" }, statement.catchClause);
      }
      this.statement(statement.catchClause.block);
      this.popScope();
    } else if (statement.finallyBlock) {
      const ex = this.alloc();
      this.emit({ op: "StoreLocal", local: ex }, statement);
      this.statement(statement.finallyBlock);
      this.emit({ op: "LoadLocal", local: ex }, statement);
      this.emit({ op: "Throw" }, statement);
    }
    this.patch(jumpJoin, this.pc());
    if (statement.finallyBlock) {
      this.statement(statement.finallyBlock);
    }
  }

  expression(expression: ts.Expression): void {
    expression = unwrap(expression);
    if (ts.isIdentifier(expression)) {
      if (expression.text === "undefined" && !this.hasBinding("undefined")) {
        this.emit({ op: "LoadConst", value: { t: "undefined" } }, expression);
        return;
      }
      if (this.hasBinding(expression.text)) {
        this.loadName(expression.text, expression);
        return;
      }
      const helper = this.functions.get(expression.text);
      if (helper?.closure) {
        this.emit({ op: "LoadFunc", func: helper.id }, expression);
        return;
      }
      this.fail(expression, `unknown local \`${expression.text}\``, WHY_SUBSET);
    }
    if (ts.isArrowFunction(expression)) {
      this.arrowExpression(expression);
      return;
    }
    const literal = constValue(expression);
    if (literal) {
      this.emit({ op: "LoadConst", value: literal }, expression);
      return;
    }
    if (ts.isAwaitExpression(expression)) {
      this.durable(expression);
      return;
    }
    if (ts.isPrefixUnaryExpression(expression) && expression.operator === ts.SyntaxKind.ExclamationToken) {
      this.expression(expression.operand);
      this.emit({ op: "Not" }, expression);
      return;
    }
    if (ts.isPrefixUnaryExpression(expression) && expression.operator === ts.SyntaxKind.MinusToken) {
      if (ts.isNumericLiteral(expression.operand)) {
        this.emit(
          { op: "LoadConst", value: { t: "number", v: -Number(expression.operand.text) } },
          expression,
        );
        return;
      }
      this.expression(expression.operand);
      this.emit({ op: "Neg" }, expression);
      return;
    }
    if (ts.isConditionalExpression(expression)) {
      this.expression(expression.condition);
      const jumpElse = this.emit({ op: "JumpIfFalse", target: 0 }, expression);
      this.expression(expression.whenTrue);
      const jumpEnd = this.emit({ op: "Jump", target: 0 }, expression);
      this.patch(jumpElse, this.pc());
      this.expression(expression.whenFalse);
      this.patch(jumpEnd, this.pc());
      return;
    }
    if (ts.isElementAccessExpression(expression)) {
      this.expression(expression.expression);
      this.expression(expression.argumentExpression);
      this.emit({ op: "GetIndex" }, expression);
      return;
    }
    if (ts.isBinaryExpression(expression)) {
      this.binary(expression);
      return;
    }
    if (ts.isObjectLiteralExpression(expression)) {
      this.objectLiteral(expression);
      return;
    }
    if (ts.isArrayLiteralExpression(expression)) {
      this.emit({ op: "NewArray" }, expression);
      for (const element of expression.elements) {
        if (ts.isSpreadElement(element)) {
          throw new CompileError("spread is not supported");
        }
        this.expression(element);
        this.emit({ op: "ArrayPush" }, element);
      }
      return;
    }
    if (ts.isCallExpression(expression)) {
      if (this.lowerHelperOrMethod(expression) || this.lowerArrayMethod(expression)) {
        return;
      }
      const method = promiseMethod(expression);
      if (method === "all") {
        throw new CompileError("await Promise.all directly; do not store the promise");
      }
      if (method === "race") {
        throw new CompileError("await Promise.race directly; do not store the promise");
      }
      if (method) {
        throw new CompileError(`Promise.${method} is not supported`);
      }
      this.callValue(expression);
      return;
    }
    if (ts.isPropertyAccessExpression(expression)) {
      if (ts.isIdentifier(expression.name) && expression.name.text === "length") {
        this.expression(expression.expression);
        this.emit({ op: "Length" }, expression);
        return;
      }
      this.expression(expression.expression);
      if (!ts.isIdentifier(expression.name)) {
        throw new CompileError("property names must be identifiers");
      }
      this.emit({ op: "GetProp", key: expression.name.text }, expression);
      return;
    }
    this.fail(expression, `unsupported expression: ${kindName(expression)}`, WHY_SUBSET);
  }

  binary(expression: ts.BinaryExpression): void {
    const op = expression.operatorToken.kind;
    if (
      op === ts.SyntaxKind.EqualsToken ||
      op === ts.SyntaxKind.PlusEqualsToken ||
      op === ts.SyntaxKind.MinusEqualsToken ||
      op === ts.SyntaxKind.AsteriskEqualsToken ||
      op === ts.SyntaxKind.SlashEqualsToken ||
      op === ts.SyntaxKind.PercentEqualsToken ||
      op === ts.SyntaxKind.AsteriskAsteriskEqualsToken
    ) {
      this.assign(expression);
      return;
    }
    if (op === ts.SyntaxKind.AmpersandAmpersandToken || op === ts.SyntaxKind.BarBarToken) {
      this.shortCircuit(expression, op === ts.SyntaxKind.AmpersandAmpersandToken);
      return;
    }
    const map: Partial<Record<ts.SyntaxKind, Instruction["op"]>> = {
      [ts.SyntaxKind.EqualsEqualsEqualsToken]: "StrictEq",
      [ts.SyntaxKind.ExclamationEqualsEqualsToken]: "StrictNeq",
      [ts.SyntaxKind.LessThanToken]: "Lt",
      [ts.SyntaxKind.LessThanEqualsToken]: "Le",
      [ts.SyntaxKind.GreaterThanToken]: "Gt",
      [ts.SyntaxKind.GreaterThanEqualsToken]: "Ge",
      [ts.SyntaxKind.PlusToken]: "Add",
      [ts.SyntaxKind.MinusToken]: "Sub",
      [ts.SyntaxKind.AsteriskToken]: "Mul",
      [ts.SyntaxKind.SlashToken]: "Div",
      [ts.SyntaxKind.PercentToken]: "Rem",
      [ts.SyntaxKind.AsteriskAsteriskToken]: "Pow",
    };
    const instruction = map[op];
    if (!instruction) {
      throw new CompileError(`unsupported operator: ${kindName(expression.operatorToken)}`);
    }
    this.expression(expression.left);
    this.expression(expression.right);
    this.emit({ op: instruction } as Instruction, expression);
  }

  objectLiteral(expression: ts.ObjectLiteralExpression): void {
    this.emit({ op: "NewObject" }, expression);
    for (const property of expression.properties) {
      if (!ts.isShorthandPropertyAssignment(property) && !ts.isPropertyAssignment(property)) {
        throw new CompileError("object literals support identifier properties only");
      }
      if (!ts.isIdentifier(property.name)) {
        throw new CompileError("object keys must be identifiers");
      }
      const valueExpr = ts.isShorthandPropertyAssignment(property)
        ? property.name
        : property.initializer;
      this.expression(valueExpr);
      this.emit({ op: "SetProp", key: property.name.text }, property);
    }
  }

  destructurePromiseAll(list: ts.VariableDeclarationList, declaration: ts.VariableDeclaration): void {
    if (!declaration.initializer || !isAwaitPromiseAll(declaration.initializer)) {
      throw new CompileError("array destructuring is only supported for await Promise.all");
    }
    if (!ts.isArrayBindingPattern(declaration.name)) {
      throw new CompileError("array destructuring is only supported for await Promise.all");
    }
    const kind = (list.flags & ts.NodeFlags.Const) !== 0 ? "const" : "let";
    for (const element of declaration.name.elements) {
      if (
        !ts.isBindingElement(element) ||
        element.dotDotDotToken ||
        !ts.isIdentifier(element.name)
      ) {
        throw new CompileError("Promise.all bindings must be simple identifiers");
      }
      this.declare(element.name.text, kind);
    }
    this.expression(declaration.initializer);
    declaration.name.elements.forEach((element, index) => {
      if (!ts.isBindingElement(element) || !ts.isIdentifier(element.name)) {
        return;
      }
      this.emit({ op: "ArrayIndex", index }, declaration);
      this.initLocal(element.name, declaration);
    });
    this.emit({ op: "Pop" }, declaration);
  }

  durable(expression: ts.AwaitExpression): void {
    if (!this.allowDurable) {
      this.fail(
        expression,
        "v0.1.0 helper calls are non-suspending. Durable boundaries are only allowed in the program entry",
        WHY_SUBSET,
        "move the durable operation into the program entry",
      );
    }
    const inner = unwrap(expression.expression);
    if (ts.isCallExpression(inner)) {
      const method = promiseMethod(inner);
      if (method === "all" || method === "race") {
        this.promiseJoin(expression, inner, method);
        return;
      }
      if (method) {
        throw new CompileError(`Promise.${method} is not supported`);
      }
    }
    if (!ts.isCallExpression(inner)) {
      throw new CompileError("await a durable operation");
    }
    this.emitDurableCall(inner, expression);
  }

  promiseJoin(
    expression: ts.AwaitExpression,
    call: ts.CallExpression,
    method: "all" | "race",
  ): void {
    const label = method === "all" ? "Promise.all" : "Promise.race";
    if (call.arguments.length !== 1) {
      throw new CompileError(`await ${label} of one array literal`);
    }
    const argument = unwrap(call.arguments[0]!);
    if (!ts.isArrayLiteralExpression(argument)) {
      throw new CompileError(`${label} argument must be an array literal`);
    }
    if (argument.elements.length === 0 || argument.elements.length > 32) {
      throw new CompileError(`${label} supports 1 to 32 branches`);
    }
    const fork = this.emit({ op: "Fork", count: argument.elements.length, join_pc: 0 }, expression);
    for (const element of argument.elements) {
      if (ts.isSpreadElement(element)) {
        throw new CompileError(`spread is not supported in ${label}`);
      }
      this.branchExpression(unwrap(element), label);
    }
    const joinPc = this.pc();
    const forkInstruction = this.instructions[fork];
    if (!forkInstruction || forkInstruction.op !== "Fork") {
      throw new CompileError("internal: fork missing");
    }
    forkInstruction.join_pc = joinPc;
    this.emit({ op: method === "all" ? "JoinAll" : "JoinAny" }, expression);
  }

  branchExpression(expression: ts.Expression, label = "Promise.all"): void {
    let callTarget = expression;
    if (ts.isAwaitExpression(callTarget)) {
      callTarget = unwrap(callTarget.expression);
    }
    if (!ts.isCallExpression(callTarget)) {
      throw new CompileError(`${label} branches must be durable calls`);
    }
    if (promiseMethod(callTarget)) {
      throw new CompileError(`${label} branches must be effect, waitForEvent, sleep, or invoke`);
    }
    this.emitDurableCall(callTarget, expression);
  }

  emitDurableCall(call: ts.CallExpression, node: ts.Node): void {
    const durable = resolveDurable(call.expression, this.checker);
    if (durable === "effect") {
      if (call.arguments.length !== 2) {
        throw new CompileError("`effect` takes a key and a callback");
      }
      const key = stringLiteral(call.arguments[0]!, "effect key");
      this.emit({ op: "LoadConst", value: { t: "string", v: key } }, node);
      this.emit({ op: "Effect" }, node);
      return;
    }
    if (durable === "waitForEvent") {
      if (call.arguments.length !== 1) {
        throw new CompileError("`waitForEvent` takes an event name");
      }
      const name = stringLiteral(call.arguments[0]!, "event name");
      this.emit({ op: "LoadConst", value: { t: "string", v: name } }, node);
      this.emit({ op: "WaitForEvent" }, node);
      return;
    }
    if (durable === "sleep") {
      if (call.arguments.length !== 1) {
        throw new CompileError("`sleep` takes a duration in milliseconds");
      }
      this.expression(call.arguments[0]!);
      this.emit({ op: "Sleep" }, node);
      return;
    }
    if (durable === "invoke") {
      if (call.arguments.some((argument) => ts.isSpreadElement(argument))) {
        throw new CompileError("`invoke` does not take spread");
      }
      if (call.arguments.length < 1) {
        throw new CompileError("`invoke` takes a flow name and optional arguments");
      }
      for (const argument of call.arguments.slice(1)) {
        this.expression(argument);
      }
      const name = stringLiteral(call.arguments[0]!, "invoke name");
      this.emit({ op: "LoadConst", value: { t: "string", v: name } }, node);
      const argCount = call.arguments.length - 1;
      this.emit(argCount === 0 ? { op: "Invoke" } : { op: "Invoke", arg_count: argCount }, node);
      return;
    }
    throw new CompileError("call is not a resolved durable operation");
  }

  hasBinding(name: string): boolean {
    return this.scopes.some((scope) => scope.has(name));
  }

  unwatchAll(): void {
    for (let index = this.loops.length - 1; index >= 0; index -= 1) {
      if (this.loops[index]!.unwatch) {
        this.emit({ op: "UnwatchIter" }, this.sourceFile);
      }
    }
  }

  forOfStatement(statement: ts.ForOfStatement): void {
    if (statement.awaitModifier) {
      this.fail(statement, "for await is not supported", WHY_SUBSET);
    }
    if (!ts.isVariableDeclarationList(statement.initializer) || statement.initializer.declarations.length !== 1) {
      this.fail(statement, "for-of binding must be one variable", WHY_SUBSET);
    }
    const declaration = statement.initializer.declarations[0]!;
    if (!ts.isIdentifier(declaration.name)) {
      this.fail(statement, "for-of binding must be an identifier", WHY_SUBSET);
    }
    const kind = (statement.initializer.flags & ts.NodeFlags.Const) !== 0 ? "const" : "let";
    this.declare(declaration.name.text, kind);
    const index = this.alloc();
    const length = this.alloc();
    this.expression(statement.expression);
    this.emit({ op: "WatchIter" }, statement.expression);
    this.emit({ op: "Length" }, statement.expression);
    this.emit({ op: "StoreLocal", local: length }, statement);
    this.emit({ op: "LoadConst", value: { t: "number", v: 0 } }, statement);
    this.emit({ op: "StoreLocal", local: index }, statement);
    const loop: Loop = { breaks: [], continues: [], unwatch: true };
    this.loops.push(loop);
    const cond = this.pc();
    this.emit({ op: "LoadLocal", local: index }, statement);
    this.emit({ op: "LoadLocal", local: length }, statement);
    this.emit({ op: "Lt" }, statement);
    const jumpEnd = this.emit({ op: "JumpIfFalse", target: 0 }, statement);
    this.expression(statement.expression);
    this.emit({ op: "LoadLocal", local: index }, statement);
    this.emit({ op: "GetIndex" }, statement);
    this.initLocal(declaration.name, declaration);
    this.statement(statement.statement);
    const incr = this.pc();
    loop.continueTarget = incr;
    for (const site of loop.continues) {
      this.patch(site, incr);
    }
    this.emit({ op: "LoadLocal", local: index }, statement);
    this.emit({ op: "LoadConst", value: { t: "number", v: 1 } }, statement);
    this.emit({ op: "Add" }, statement);
    this.emit({ op: "StoreLocal", local: index }, statement);
    this.emit({ op: "Jump", target: cond }, statement);
    this.patch(jumpEnd, this.pc());
    this.emit({ op: "UnwatchIter" }, statement);
    const end = this.pc();
    for (const site of loop.breaks) {
      this.patch(site, end);
    }
    this.loops.pop();
  }

  assign(expression: ts.BinaryExpression): void {
    const op = expression.operatorToken.kind;
    const arithmetic: Partial<Record<number, Instruction["op"]>> = {
      [ts.SyntaxKind.PlusEqualsToken]: "Add",
      [ts.SyntaxKind.MinusEqualsToken]: "Sub",
      [ts.SyntaxKind.AsteriskEqualsToken]: "Mul",
      [ts.SyntaxKind.SlashEqualsToken]: "Div",
      [ts.SyntaxKind.PercentEqualsToken]: "Rem",
      [ts.SyntaxKind.AsteriskAsteriskEqualsToken]: "Pow",
    };
    const arith = arithmetic[op];
    if (ts.isIdentifier(expression.left)) {
      const binding = this.lookup(expression.left.text);
      if (binding.kind === "const") {
        throw new CompileError(`cannot assign to const \`${expression.left.text}\``);
      }
      if (arith) {
        this.loadName(expression.left.text, expression.left);
        this.expression(expression.right);
        this.emit({ op: arith } as Instruction, expression);
      } else {
        this.expression(expression.right);
      }
      this.storeName(expression.left.text, expression);
      return;
    }
    const valueSlot = this.alloc();
    if (ts.isPropertyAccessExpression(expression.left) && ts.isIdentifier(expression.left.name)) {
      const key = expression.left.name.text;
      const base = this.alloc();
      this.expression(expression.left.expression);
      this.emit({ op: "StoreLocal", local: base }, expression.left);
      if (arith) {
        this.emit({ op: "LoadLocal", local: base }, expression.left);
        this.emit({ op: "GetProp", key }, expression.left);
        this.expression(expression.right);
        this.emit({ op: arith } as Instruction, expression);
      } else {
        this.expression(expression.right);
      }
      this.emit({ op: "StoreLocal", local: valueSlot }, expression);
      this.emit({ op: "LoadLocal", local: base }, expression);
      this.emit({ op: "LoadLocal", local: valueSlot }, expression);
      this.emit({ op: "SetProp", key }, expression);
      this.emit({ op: "Pop" }, expression);
      this.emit({ op: "LoadLocal", local: valueSlot }, expression);
      return;
    }
    if (ts.isElementAccessExpression(expression.left)) {
      const base = this.alloc();
      const index = this.alloc();
      this.expression(expression.left.expression);
      this.emit({ op: "StoreLocal", local: base }, expression.left);
      this.expression(expression.left.argumentExpression);
      this.emit({ op: "StoreLocal", local: index }, expression.left);
      if (arith) {
        this.emit({ op: "LoadLocal", local: base }, expression.left);
        this.emit({ op: "LoadLocal", local: index }, expression.left);
        this.emit({ op: "GetIndex" }, expression.left);
        this.expression(expression.right);
        this.emit({ op: arith } as Instruction, expression);
      } else {
        this.expression(expression.right);
      }
      this.emit({ op: "StoreLocal", local: valueSlot }, expression);
      this.emit({ op: "LoadLocal", local: base }, expression);
      this.emit({ op: "LoadLocal", local: index }, expression);
      this.emit({ op: "LoadLocal", local: valueSlot }, expression);
      this.emit({ op: "SetIndex" }, expression);
      this.emit({ op: "Pop" }, expression);
      this.emit({ op: "LoadLocal", local: valueSlot }, expression);
      return;
    }
    throw new CompileError("assignment target must be a local, property, or index");
  }

  shortCircuit(expression: ts.BinaryExpression, and: boolean): void {
    const slot = this.alloc();
    this.expression(expression.left);
    this.emit({ op: "StoreLocal", local: slot }, expression.left);
    this.emit({ op: "LoadLocal", local: slot }, expression.left);
    const jump = this.emit(
      { op: and ? "JumpIfFalse" : "JumpIfTrue", target: 0 },
      expression,
    );
    this.expression(expression.right);
    const jumpEnd = this.emit({ op: "Jump", target: 0 }, expression);
    this.patch(jump, this.pc());
    this.emit({ op: "LoadLocal", local: slot }, expression.left);
    this.patch(jumpEnd, this.pc());
  }

  destructureArray(list: ts.VariableDeclarationList, declaration: ts.VariableDeclaration): void {
    if (!declaration.initializer || !ts.isArrayBindingPattern(declaration.name)) {
      throw new CompileError("array destructuring needs an initializer");
    }
    const kind = (list.flags & ts.NodeFlags.Const) !== 0 ? "const" : "let";
    for (const element of declaration.name.elements) {
      if (!ts.isBindingElement(element) || element.dotDotDotToken || element.initializer || !ts.isIdentifier(element.name)) {
        throw new CompileError("array destructuring bindings must be simple identifiers");
      }
      this.declare(element.name.text, kind);
    }
    this.expression(declaration.initializer);
    declaration.name.elements.forEach((element, index) => {
      if (!ts.isBindingElement(element) || !ts.isIdentifier(element.name)) {
        return;
      }
      this.emit({ op: "ArrayIndex", index }, declaration);
      this.initLocal(element.name, declaration);
    });
    this.emit({ op: "Pop" }, declaration);
  }

  destructureObject(list: ts.VariableDeclarationList, declaration: ts.VariableDeclaration): void {
    if (!declaration.initializer || !ts.isObjectBindingPattern(declaration.name)) {
      throw new CompileError("object destructuring needs an initializer");
    }
    const kind = (list.flags & ts.NodeFlags.Const) !== 0 ? "const" : "let";
    const fields: Array<{ key: string; name: ts.Identifier }> = [];
    for (const element of declaration.name.elements) {
      if (!ts.isBindingElement(element) || element.dotDotDotToken || element.initializer || !ts.isIdentifier(element.name)) {
        throw new CompileError("object destructuring bindings must be simple identifiers");
      }
      const key = element.propertyName && ts.isIdentifier(element.propertyName)
        ? element.propertyName.text
        : element.name.text;
      this.declare(element.name.text, kind);
      fields.push({ key, name: element.name });
    }
    const base = this.alloc();
    this.expression(declaration.initializer);
    this.emit({ op: "StoreLocal", local: base }, declaration);
    for (const field of fields) {
      this.emit({ op: "LoadLocal", local: base }, declaration);
      this.emit({ op: "GetProp", key: field.key }, declaration);
      this.initLocal(field.name, declaration);
    }
  }

  installOuter(name: string, kind: "const" | "let", index: number): void {
    const scope = this.scopes[this.scopes.length - 1];
    if (!scope) {
      throw new CompileError("internal: no scope");
    }
    scope.set(name, { slot: 0, kind, cell: false, outerIndex: index });
  }

  boxExisting(name: string, node: ts.Node): void {
    const binding = this.lookup(name);
    const box = this.alloc();
    this.emit({ op: "LoadLocal", local: binding.slot }, node);
    this.emit({ op: "NewCell" }, node);
    this.emit({ op: "NewEnv", count: 1 }, node);
    this.emit({ op: "StoreLocal", local: box }, node);
    binding.slot = box;
    binding.cell = true;
  }

  initLocal(name: ts.Identifier, node: ts.Node): void {
    const binding = this.lookup(name.text);
    if (this.fn.capturedLocals.has(name)) {
      binding.cell = true;
      this.emit({ op: "NewCell" }, node);
      this.emit({ op: "NewEnv", count: 1 }, node);
    }
    this.emit({ op: "StoreLocal", local: binding.slot }, node);
  }

  loadName(name: string, node: ts.Node): void {
    const binding = this.lookup(name);
    if (binding.outerIndex !== undefined) {
      this.emit({ op: "LoadLocal", local: 0 }, node);
      this.emit({ op: "EnvGet", index: binding.outerIndex }, node);
      return;
    }
    this.emit({ op: "LoadLocal", local: binding.slot }, node);
    if (binding.cell) {
      this.emit({ op: "EnvGet", index: 0 }, node);
    }
  }

  storeName(name: string, node: ts.Node): void {
    const binding = this.lookup(name);
    if (binding.kind === "const") {
      throw new CompileError(`cannot assign to const \`${name}\``);
    }
    if (binding.outerIndex === undefined && !binding.cell) {
      this.emit({ op: "StoreLocal", local: binding.slot }, node);
      this.emit({ op: "LoadLocal", local: binding.slot }, node);
      return;
    }
    const temp = this.alloc();
    this.emit({ op: "StoreLocal", local: temp }, node);
    if (binding.outerIndex !== undefined) {
      this.emit({ op: "LoadLocal", local: 0 }, node);
      this.emit({ op: "LoadLocal", local: temp }, node);
      this.emit({ op: "EnvSet", index: binding.outerIndex }, node);
    } else {
      this.emit({ op: "LoadLocal", local: binding.slot }, node);
      this.emit({ op: "LoadLocal", local: temp }, node);
      this.emit({ op: "EnvSet", index: 0 }, node);
    }
    this.emit({ op: "LoadLocal", local: temp }, node);
  }

  emitCellRef(name: string, node: ts.Node): void {
    const binding = this.lookup(name);
    if (binding.outerIndex !== undefined) {
      this.emit({ op: "LoadLocal", local: 0 }, node);
      this.emit({ op: "EnvSlot", index: binding.outerIndex }, node);
      return;
    }
    this.emit({ op: "LoadLocal", local: binding.slot }, node);
    this.emit({ op: "EnvSlot", index: 0 }, node);
  }

  arrowExpression(expression: ts.ArrowFunction): void {
    if (ts.getModifiers(expression)?.some((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword)) {
      this.fail(expression, "async arrows are not supported", WHY_SUBSET, "keep durable operations in the program entry");
    }
    for (const param of expression.parameters) {
      if (param.initializer || param.questionToken || param.dotDotDotToken || !ts.isIdentifier(param.name)) {
        this.fail(expression, "arrow parameters must be plain identifiers", WHY_SUBSET);
      }
    }
    const rec = this.recs.get(expression);
    if (!rec) {
      this.fail(expression, "unsupported arrow", WHY_SUBSET);
    }
    for (const captured of rec.captured) {
      this.emitCellRef(captured.name, expression);
    }
    this.emit({ op: "NewEnv", count: rec.captured.length }, expression);
    this.emit({ op: "NewClosure", func: rec.id }, expression);
  }

  callValue(expression: ts.CallExpression): void {
    if (expression.arguments.some((argument) => ts.isSpreadElement(argument))) {
      throw new CompileError("spread is not supported");
    }
    this.expression(unwrap(expression.expression));
    for (const argument of expression.arguments) {
      this.expression(argument);
    }
    this.emit({ op: "CallClosure", argc: expression.arguments.length }, expression);
  }

  lowerArrayMethod(expression: ts.CallExpression): boolean {
    const target = unwrap(expression.expression);
    if (!ts.isPropertyAccessExpression(target) || !ts.isIdentifier(target.name)) {
      return false;
    }
    const name = target.name.text;
    if (name !== "map" && name !== "filter" && name !== "reduce") {
      return false;
    }
    if (expression.arguments.some((argument) => ts.isSpreadElement(argument))) {
      throw new CompileError("spread is not supported");
    }
    const callbackCount = name === "reduce" ? [1, 2] : [1];
    if (!callbackCount.includes(expression.arguments.length)) {
      this.fail(
        expression,
        `\`${name}\` does not take a thisArg`,
        WHY_SUBSET,
        "pass the callback only",
      );
    }
    const array = this.alloc();
    const index = this.alloc();
    const length = this.alloc();
    const callback = this.alloc();
    this.expression(target.expression);
    this.emit({ op: "StoreLocal", local: array }, expression);
    this.expression(expression.arguments[0]!);
    this.emit({ op: "StoreLocal", local: callback }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "WatchIter" }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "Length" }, expression);
    this.emit({ op: "StoreLocal", local: length }, expression);
    const loop: Loop = { breaks: [], continues: [], unwatch: true };
    this.loops.push(loop);
    if (name === "reduce") {
      this.lowerReduce(expression, array, index, length, callback, loop);
    } else {
      this.lowerMapOrFilter(expression, name, array, index, length, callback, loop);
    }
    this.loops.pop();
    return true;
  }

  lowerMapOrFilter(
    expression: ts.CallExpression,
    name: "map" | "filter",
    array: number,
    index: number,
    length: number,
    callback: number,
    loop: Loop,
  ): void {
    const result = this.alloc();
    const element = this.alloc();
    this.emit({ op: "NewArray" }, expression);
    this.emit({ op: "StoreLocal", local: result }, expression);
    this.emit({ op: "LoadConst", value: { t: "number", v: 0 } }, expression);
    this.emit({ op: "StoreLocal", local: index }, expression);
    const cond = this.pc();
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadLocal", local: length }, expression);
    this.emit({ op: "Lt" }, expression);
    const jumpEnd = this.emit({ op: "JumpIfFalse", target: 0 }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "GetIndex" }, expression);
    this.emit({ op: "StoreLocal", local: element }, expression);
    this.emit({ op: "LoadLocal", local: callback }, expression);
    this.emit({ op: "LoadLocal", local: element }, expression);
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "CallClosure", argc: 3 }, expression);
    if (name === "filter") {
      const skip = this.emit({ op: "JumpIfFalse", target: 0 }, expression);
      this.emit({ op: "LoadLocal", local: result }, expression);
      this.emit({ op: "LoadLocal", local: element }, expression);
      this.emit({ op: "ArrayPush" }, expression);
      this.emit({ op: "Pop" }, expression);
      this.patch(skip, this.pc());
    } else {
      const mapped = this.alloc();
      this.emit({ op: "StoreLocal", local: mapped }, expression);
      this.emit({ op: "LoadLocal", local: result }, expression);
      this.emit({ op: "LoadLocal", local: mapped }, expression);
      this.emit({ op: "ArrayPush" }, expression);
      this.emit({ op: "Pop" }, expression);
    }
    const incr = this.pc();
    loop.continueTarget = incr;
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadConst", value: { t: "number", v: 1 } }, expression);
    this.emit({ op: "Add" }, expression);
    this.emit({ op: "StoreLocal", local: index }, expression);
    this.emit({ op: "Jump", target: cond }, expression);
    const end = this.pc();
    this.patch(jumpEnd, end);
    this.emit({ op: "UnwatchIter" }, expression);
    this.emit({ op: "LoadLocal", local: result }, expression);
  }

  lowerReduce(
    expression: ts.CallExpression,
    array: number,
    index: number,
    length: number,
    callback: number,
    loop: Loop,
  ): void {
    const accumulator = this.alloc();
    const hasInit = expression.arguments.length === 2;
    if (!hasInit) {
      this.emit({ op: "LoadLocal", local: length }, expression);
      this.emit({ op: "LoadConst", value: { t: "number", v: 0 } }, expression);
      this.emit({ op: "StrictEq" }, expression);
      const nonempty = this.emit({ op: "JumpIfFalse", target: 0 }, expression);
      this.emit({ op: "UnwatchIter" }, expression);
      this.emit({ op: "LoadConst", value: { t: "string", v: "TypeError" } }, expression);
      this.emit({ op: "Throw" }, expression);
      this.patch(nonempty, this.pc());
      this.emit({ op: "LoadLocal", local: array }, expression);
      this.emit({ op: "LoadConst", value: { t: "number", v: 0 } }, expression);
      this.emit({ op: "GetIndex" }, expression);
      this.emit({ op: "StoreLocal", local: accumulator }, expression);
      this.emit({ op: "LoadConst", value: { t: "number", v: 1 } }, expression);
      this.emit({ op: "StoreLocal", local: index }, expression);
    } else {
      this.expression(expression.arguments[1]!);
      this.emit({ op: "StoreLocal", local: accumulator }, expression);
      this.emit({ op: "LoadConst", value: { t: "number", v: 0 } }, expression);
      this.emit({ op: "StoreLocal", local: index }, expression);
    }
    const cond = this.pc();
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadLocal", local: length }, expression);
    this.emit({ op: "Lt" }, expression);
    const jumpEnd = this.emit({ op: "JumpIfFalse", target: 0 }, expression);
    this.emit({ op: "LoadLocal", local: callback }, expression);
    this.emit({ op: "LoadLocal", local: accumulator }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "GetIndex" }, expression);
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadLocal", local: array }, expression);
    this.emit({ op: "CallClosure", argc: 4 }, expression);
    this.emit({ op: "StoreLocal", local: accumulator }, expression);
    const incr = this.pc();
    loop.continueTarget = incr;
    this.emit({ op: "LoadLocal", local: index }, expression);
    this.emit({ op: "LoadConst", value: { t: "number", v: 1 } }, expression);
    this.emit({ op: "Add" }, expression);
    this.emit({ op: "StoreLocal", local: index }, expression);
    this.emit({ op: "Jump", target: cond }, expression);
    const end = this.pc();
    this.patch(jumpEnd, end);
    this.emit({ op: "UnwatchIter" }, expression);
    this.emit({ op: "LoadLocal", local: accumulator }, expression);
  }

  lowerHelperOrMethod(expression: ts.CallExpression): boolean {
    const target = unwrap(expression.expression);
    if (ts.isIdentifier(target)) {
      const info = this.functions.get(target.text);
      if (!info) {
        return false;
      }
      if (expression.arguments.length > info.paramCount) {
        throw new CompileError(`\`${target.text}\` expects at most ${info.paramCount} arguments`);
      }
      if (info.closure) {
        this.emit({ op: "LoadFunc", func: info.id }, target);
      }
      for (const argument of expression.arguments) {
        if (ts.isSpreadElement(argument)) {
          throw new CompileError("spread is not supported");
        }
        this.expression(argument);
      }
      if (info.closure) {
        this.emit({ op: "CallClosure", argc: expression.arguments.length }, expression);
      } else {
        this.emit({ op: "Call", func: info.id, argc: expression.arguments.length }, expression);
      }
      return true;
    }
    if (
      ts.isPropertyAccessExpression(target) &&
      ts.isIdentifier(target.name) &&
      target.name.text === "push" &&
      expression.arguments.length === 1 &&
      !ts.isSpreadElement(expression.arguments[0]!)
    ) {
      this.expression(target.expression);
      this.expression(expression.arguments[0]!);
      this.emit({ op: "ArrayPush" }, expression);
      return true;
    }
    return false;
  }

  needsImplicitReturn(): boolean {
    const end = this.pc();
    const last = this.instructions[end - 1];
    if (!last || last.op !== "Return") {
      return true;
    }
    return this.instructions.some((instruction) => {
      if ("target" in instruction && instruction.target === end) {
        return true;
      }
      if (instruction.op === "PushTry" && (instruction.catch === end || instruction.finally === end)) {
        return true;
      }
      if (instruction.op === "Fork" && instruction.join_pc === end) {
        return true;
      }
      return false;
    });
  }

  seal(): void {
    const len = this.instructions.length;
    const needsNop = this.instructions.some((instruction) => {
      if ("target" in instruction && instruction.target === len) {
        return true;
      }
      if (instruction.op === "PushTry") {
        return instruction.catch === len || instruction.finally === len;
      }
      return false;
    });
    if (needsNop) {
      this.instructions.push({ op: "Nop" });
      this.spans.push(null);
    }
  }
}

function unwrap(expression: ts.Expression): ts.Expression {
  while (ts.isParenthesizedExpression(expression) || ts.isAsExpression(expression)) {
    expression = expression.expression;
  }
  return expression;
}

function constValue(expression: ts.Expression): ConstValue | undefined {
  switch (expression.kind) {
    case ts.SyntaxKind.TrueKeyword:
      return { t: "bool", v: true };
    case ts.SyntaxKind.FalseKeyword:
      return { t: "bool", v: false };
    case ts.SyntaxKind.NullKeyword:
      return { t: "null" };
    default:
      break;
  }
  if (ts.isNumericLiteral(expression)) {
    return { t: "number", v: Number(expression.text) };
  }
  if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
    return { t: "string", v: expression.text };
  }
  return undefined;
}

function requiredFrom(instructions: Instruction[]): {
  engine: EngineFeature[];
  host: HostCapability[];
} {
  const engine: EngineFeature[] = ["ts.control_flow"];
  const host: HostCapability[] = ["host.persist_checkpoint"];
  const addEngine = (feature: EngineFeature) => {
    if (!engine.includes(feature)) {
      engine.push(feature);
    }
  };
  const addHost = (capability: HostCapability) => {
    if (!host.includes(capability)) {
      host.push(capability);
    }
  };
  for (const instruction of instructions) {
    switch (instruction.op) {
      case "Effect":
        addEngine("durable.effect");
        addHost("host.effect");
        break;
      case "WaitForEvent":
        addEngine("durable.wait_for_event");
        addHost("host.event");
        break;
      case "Sleep":
        addEngine("durable.sleep");
        addHost("host.timer");
        break;
      case "Invoke":
        addEngine("durable.invoke");
        addHost("host.child");
        break;
      case "Fork":
      case "JoinAll":
      case "JoinAny":
        addEngine("durable.concurrent_group");
        break;
      case "Throw":
      case "PushTry":
      case "PopTry":
        addEngine("ts.exceptions");
        break;
      case "Add":
      case "Sub":
      case "Mul":
      case "Div":
      case "Rem":
      case "Neg":
      case "GetIndex":
      case "SetIndex":
      case "Length":
      case "WatchIter":
      case "UnwatchIter":
      case "Same":
      case "Call":
      case "Pow":
      case "NewCell":
      case "NewEnv":
      case "NewClosure":
      case "EnvGet":
      case "EnvSet":
      case "EnvSlot":
      case "CallClosure":
      case "LoadFunc":
        addEngine("lang.compute");
        break;
      default:
        break;
    }
  }
  return { engine, host };
}

function promiseMethod(call: ts.CallExpression): string | undefined {
  const expression = unwrap(call.expression);
  if (!ts.isPropertyAccessExpression(expression) || !ts.isIdentifier(expression.name)) {
    return undefined;
  }
  const base = unwrap(expression.expression);
  if (!ts.isIdentifier(base) || base.text !== "Promise") {
    return undefined;
  }
  return expression.name.text;
}

function isAwaitPromiseAll(expression: ts.Expression): boolean {
  const inner = unwrap(expression);
  if (!ts.isAwaitExpression(inner) || !ts.isCallExpression(unwrap(inner.expression))) {
    return false;
  }
  return promiseMethod(unwrap(inner.expression) as ts.CallExpression) === "all";
}

function resolveDurable(
  expression: ts.Expression,
  checker: ts.TypeChecker,
): DurableName | undefined {
  const symbol = checker.getSymbolAtLocation(expression);
  if (!symbol) {
    return undefined;
  }
  const resolved = symbol.flags & ts.SymbolFlags.Alias ? checker.getAliasedSymbol(symbol) : symbol;
  const name = resolved.getName();
  if (name !== "effect" && name !== "waitForEvent" && name !== "sleep" && name !== "invoke") {
    return undefined;
  }
  const declaration = resolved.getDeclarations()?.[0];
  if (!declaration) {
    return undefined;
  }
  const file = declaration.getSourceFile().fileName.replace(/\\/g, "/");
  if (!isDurableDeclarationFile(file)) {
    return undefined;
  }
  return name;
}

function isDurableDeclarationFile(file: string): boolean {
  return (
    file === SDK_PATH ||
    file === PRIMITIVES_PATH ||
    file.endsWith("@trigora/sdk/index.d.ts") ||
    file.endsWith("@tcc-engine/primitives/index.d.ts")
  );
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

function fail(
  filename: string,
  sourceFile: ts.SourceFile,
  node: ts.Node,
  message: string,
  why?: string,
  alternative?: string,
): never {
  throw new CompileError(message, filename, spanOf(sourceFile, filename, node), {
    ...(why === undefined ? {} : { why }),
    ...(alternative === undefined ? {} : { alternative }),
  });
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

function diagnosticInfo(
  diagnostic: ts.Diagnostic,
  filename: string,
): { message: string; span: DiagnosticSpan | null } {
  const message = `${filename}: ${ts.flattenDiagnosticMessageText(diagnostic.messageText, "\n")}`;
  const source = diagnostic.file;
  if (!source || diagnostic.start === undefined) {
    return { message, span: { file: filename, start_line: 1, start_column: 1 } };
  }
  const start = source.getLineAndCharacterOfPosition(diagnostic.start);
  const end = source.getLineAndCharacterOfPosition(diagnostic.start + (diagnostic.length ?? 0));
  return {
    message,
    span: {
      file: filename,
      start_line: start.line + 1,
      start_column: start.character + 1,
      end_line: end.line + 1,
      end_column: end.character + 1,
    },
  };
}
