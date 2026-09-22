import ts from "typescript";

export type Captured = { name: string; kind: "const" | "let"; decl: ts.Identifier };

export type FnRec = {
  id: number;
  name: string;
  node: ts.FunctionDeclaration | ts.ArrowFunction;
  parent?: FnRec;
  paramCount: number;
  closure: boolean;
  declared: Set<ts.Identifier>;
  captured: Captured[];
  capturedLocals: Set<ts.Identifier>;
};

type Site = { decl: ts.Identifier; kind: "const" | "let"; fn: FnRec };

export type CaptureAnalysis = {
  recs: Map<ts.Node, FnRec>;
  taken: Set<string>;
};

export function analyzeCaptures(
  sourceFile: ts.SourceFile,
  entry: ts.FunctionDeclaration,
  helpers: ts.FunctionDeclaration[],
): CaptureAnalysis {
  const helperNames = new Set(helpers.map((helper) => helper.name!.text));
  const recs = new Map<ts.Node, FnRec>();
  const taken = new Set<string>();
  let nextId = 1 + helpers.length;
  const entryRec = recFor(entry, 0, entry.name?.text ?? "default", false);
  recs.set(entry, entryRec);
  helpers.forEach((helper, index) => {
    recs.set(helper, recFor(helper, index + 1, helper.name!.text, false));
  });

  const rootScopes: Scope[] = [];
  visit(sourceFile, undefined, rootScopes);

  for (const rec of recs.values()) {
    if (rec.node !== entry && rec.parent) {
      rec.closure = true;
    }
  }
  for (const name of taken) {
    const helper = helpers.find((item) => item.name!.text === name);
    const rec = helper ? recs.get(helper) : undefined;
    if (rec) {
      rec.closure = true;
    }
  }
  return { recs, taken };

  type Scope = Map<string, Site>;

  function recFor(
    node: ts.FunctionDeclaration | ts.ArrowFunction,
    id: number,
    name: string,
    closure: boolean,
  ): FnRec {
    return {
      id,
      name,
      node,
      paramCount: node.parameters.length,
      closure,
      declared: new Set(),
      captured: [],
      capturedLocals: new Set(),
    };
  }

  function visit(node: ts.Node, fn: FnRec | undefined, scopes: Scope[]): void {
    if (ts.isFunctionDeclaration(node) && node.body && !recs.has(node)) {
      return;
    }
    if (ts.isFunctionDeclaration(node) && node.body && recs.has(node)) {
      const rec = recs.get(node)!;
      if (fn) {
        rec.parent = fn;
      }
      const inner = pushFunction(rec, scopes);
      for (const statement of node.body.statements) {
        visit(statement, rec, inner);
      }
      return;
    }
    if (ts.isArrowFunction(node)) {
      if (hasAsync(node)) {
        return;
      }
      const rec = recFor(node, nextId, `arrow${nextId}`, true);
      nextId += 1;
      if (fn) {
        rec.parent = fn;
      }
      recs.set(node, rec);
      const inner = pushFunction(rec, scopes);
      if (ts.isBlock(node.body)) {
        for (const statement of node.body.statements) {
          visit(statement, rec, inner);
        }
      } else {
        visit(node.body, rec, inner);
      }
      return;
    }
    if (ts.isBlock(node) || ts.isCatchClause(node)) {
      const inner = scopes.concat(new Map());
      ts.forEachChild(node, (child) => visit(child, fn, inner));
      return;
    }
    if (fn && ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      const kind = declarationKind(node);
      bind(scopes, node.name.text, { decl: node.name, kind, fn });
      fn.declared.add(node.name);
    }
    if (fn && ts.isIdentifier(node) && isReference(node)) {
      const found = lookup(scopes, node.text);
      if (!found) {
        if (helperNames.has(node.text) && !isDirectCall(node)) {
          taken.add(node.text);
        }
        return;
      }
      if (found.fn !== fn) {
        noteCapture(fn, found);
      }
      return;
    }
    ts.forEachChild(node, (child) => visit(child, fn, scopes));
  }

  function pushFunction(rec: FnRec, scopes: Scope[]): Scope[] {
    const scope = new Map<string, Site>();
    for (const param of rec.node.parameters) {
      if (ts.isIdentifier(param.name)) {
        const site: Site = { decl: param.name, kind: "let", fn: rec };
        scope.set(param.name.text, site);
        rec.declared.add(param.name);
      }
    }
    return scopes.concat(scope);
  }

  function noteCapture(fn: FnRec, found: Site): void {
    add(fn, found);
    found.fn.capturedLocals.add(found.decl);
    let mid = fn.parent;
    while (mid && mid !== found.fn) {
      add(mid, found);
      mid = mid.parent;
    }
  }

  function add(fn: FnRec, found: Site): void {
    if (fn.captured.some((item) => item.decl === found.decl)) {
      return;
    }
    fn.captured.push({ name: found.decl.text, kind: found.kind, decl: found.decl });
  }
}

function bind(scopes: Array<Map<string, Site>>, name: string, site: Site): void {
  const scope = scopes[scopes.length - 1];
  scope?.set(name, site);
}

function lookup(scopes: Array<Map<string, Site>>, name: string): Site | undefined {
  for (let index = scopes.length - 1; index >= 0; index -= 1) {
    const found = scopes[index]?.get(name);
    if (found) {
      return found;
    }
  }
  return undefined;
}

function declarationKind(node: ts.VariableDeclaration): "const" | "let" {
  const list = node.parent;
  if (ts.isVariableDeclarationList(list) && (list.flags & ts.NodeFlags.Const) !== 0) {
    return "const";
  }
  return "let";
}

function isDirectCall(node: ts.Identifier): boolean {
  const parent = node.parent;
  return ts.isCallExpression(parent) && unwrapCallee(parent.expression) === node;
}

function unwrapCallee(expression: ts.Expression): ts.Expression {
  let current = expression;
  while (ts.isParenthesizedExpression(current) || ts.isAsExpression(current) || ts.isTypeAssertionExpression(current)) {
    current = current.expression;
  }
  return current;
}

function isReference(node: ts.Identifier): boolean {
  if (inTypePosition(node)) {
    return false;
  }
  const parent = node.parent;
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) {
    return false;
  }
  if (ts.isVariableDeclaration(parent) && parent.name === node) {
    return false;
  }
  if (ts.isParameter(parent) && parent.name === node) {
    return false;
  }
  if (ts.isFunctionDeclaration(parent) && parent.name === node) {
    return false;
  }
  if (ts.isBindingElement(parent) && parent.name === node) {
    return false;
  }
  if (ts.isBreakStatement(parent) || ts.isContinueStatement(parent)) {
    return false;
  }
  return true;
}

function inTypePosition(node: ts.Node): boolean {
  let current: ts.Node | undefined = node.parent;
  while (current) {
    if (ts.isTypeNode(current)) {
      return true;
    }
    current = current.parent;
  }
  return false;
}

function hasAsync(node: ts.ArrowFunction): boolean {
  return Boolean(ts.getModifiers(node)?.some((modifier) => modifier.kind === ts.SyntaxKind.AsyncKeyword));
}
