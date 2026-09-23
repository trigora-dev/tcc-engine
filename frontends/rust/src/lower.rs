//! Owned Rust subset. Compounds move, `f64` and `bool` copy, and references are rejected.
//!
//! `cargo check` against the prelude owns move checking. This walk emits the
//! existing instruction set and does not call rustc.

use std::collections::{HashMap, HashSet};

use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    BinOp, Block, Expr, Fields, File, FnArg, Item, Lit, Pat, ReturnType, Stmt, Type, TypePath,
};
use tcc_ir::{
    Artifact, ConstValue, EngineFeature, Envelope, FuncId, Function, HostCapability, Instruction,
    LocalId, Pc, Program, ENGINE_FORMAT_VERSION, FRONTEND_RUST, LANGUAGE_SEMANTICS_RUST,
};

const TAG: &str = "$tag";
const PAYLOAD: &str = "$0";

#[derive(Debug)]
pub struct CompileError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl CompileError {
    fn at(message: impl Into<String>, span: proc_macro2::Span) -> Self {
        let start = span.start();
        Self {
            message: message.into(),
            line: start.line as u32,
            column: start.column as u32 + 1,
        }
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    F64,
    Bool,
    String,
    Unit,
    Struct,
    Enum,
    OptionF64,
    ResultF64,
    ResultBool,
    ResultString,
    ResultUnit,
    ResultStruct,
    ResultVec,
    VecF64,
    Closure,
    /// A durable result whose payload type comes from a type ascription.
    Host,
    Never,
}

impl Ty {
    fn moves(self) -> bool {
        !matches!(self, Ty::F64 | Ty::Bool | Ty::Unit | Ty::Host | Ty::Never)
    }
}

fn result_inner(ty: Ty) -> Option<Ty> {
    match ty {
        Ty::ResultF64 => Some(Ty::F64),
        Ty::ResultBool => Some(Ty::Bool),
        Ty::ResultString => Some(Ty::String),
        Ty::ResultUnit => Some(Ty::Unit),
        Ty::ResultStruct => Some(Ty::Struct),
        Ty::ResultVec => Some(Ty::VecF64),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct Slot {
    id: u32,
    ty: Ty,
    mutable: bool,
}

struct Variant {
    fields: Vec<String>,
}

#[derive(Clone)]
struct HelperSig {
    id: u32,
    params: Vec<Ty>,
    ret: Ty,
}

#[derive(Clone)]
struct ClosureSig {
    params: Vec<Ty>,
    ret: Ty,
}

struct LoopFrame {
    /// `Some` when `continue` can jump now. `None` for a `for` increment that is patched later.
    continue_pc: Option<u32>,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

struct Compiler {
    names: HashMap<String, Slot>,
    moved: HashMap<String, ()>,
    instructions: Vec<Instruction>,
    next_local: u32,
    param_count: u32,
    ret: Ty,
    struct_name: Option<String>,
    struct_fields: Vec<String>,
    enum_name: Option<String>,
    variants: HashMap<String, Variant>,
    loop_binding: Option<String>,
    helpers: HashMap<String, HelperSig>,
    closure_sigs: HashMap<String, ClosureSig>,
    pending_closure: Option<ClosureSig>,
    captures: HashMap<String, (u32, Ty)>,
    loops: Vec<LoopFrame>,
    extras: Vec<Function>,
    next_func: u32,
    entry: bool,
    expected: Option<Ty>,
}

struct FnState {
    names: HashMap<String, Slot>,
    moved: HashMap<String, ()>,
    instructions: Vec<Instruction>,
    next_local: u32,
    param_count: u32,
    ret: Ty,
    loop_binding: Option<String>,
    captures: HashMap<String, (u32, Ty)>,
    loops: Vec<LoopFrame>,
    entry: bool,
    expected: Option<Ty>,
    pending_closure: Option<ClosureSig>,
}

pub fn lower(source: &str) -> Result<Artifact, CompileError> {
    let file: File = syn::parse_file(source).map_err(|error| CompileError {
        message: error.to_string(),
        line: 1,
        column: 1,
    })?;
    let mut reject = Reject { error: None };
    reject.visit_file(&file);
    if let Some(error) = reject.error {
        return Err(error);
    }
    let mut compiler = Compiler {
        names: HashMap::new(),
        moved: HashMap::new(),
        instructions: Vec::new(),
        next_local: 0,
        param_count: 0,
        ret: Ty::F64,
        struct_name: None,
        struct_fields: Vec::new(),
        enum_name: None,
        variants: HashMap::new(),
        loop_binding: None,
        helpers: HashMap::new(),
        closure_sigs: HashMap::new(),
        pending_closure: None,
        captures: HashMap::new(),
        loops: Vec::new(),
        extras: Vec::new(),
        next_func: 1,
        entry: false,
        expected: None,
    };
    compiler.collect(&file)?;
    compiler.compile_run(&file)?;
    compiler.finish()
}

struct Reject {
    error: Option<CompileError>,
}

impl<'ast> Visit<'ast> for Reject {
    fn visit_item(&mut self, item: &'ast Item) {
        match item {
            Item::Use(_) | Item::Struct(_) | Item::Enum(_) | Item::Fn(_) => {}
            Item::Macro(mac) => {
                self.note("macros are outside this subset", mac.span());
                return;
            }
            other => {
                self.note("outside this subset", other.span());
                return;
            }
        }
        syn::visit::visit_item(self, item);
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        match expr {
            Expr::Reference(_) => {
                self.note("references are outside this subset", expr.span());
                return;
            }
            Expr::Macro(_) => {
                self.note("macros are outside this subset", expr.span());
                return;
            }
            _ => {}
        }
        syn::visit::visit_expr(self, expr);
    }

    fn visit_type(&mut self, ty: &'ast Type) {
        if let Type::Reference(_) = ty {
            self.note("references are outside this subset", ty.span());
            return;
        }
        syn::visit::visit_type(self, ty);
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        if matches!(pat, Pat::Reference(_)) {
            self.note("references are outside this subset", pat.span());
            return;
        }
        syn::visit::visit_pat(self, pat);
    }
}

impl Reject {
    fn note(&mut self, message: &str, span: proc_macro2::Span) {
        if self.error.is_none() {
            self.error = Some(CompileError::at(message, span));
        }
    }
}

impl Compiler {
    fn collect(&mut self, file: &File) -> Result<(), CompileError> {
        for item in &file.items {
            match item {
                Item::Struct(item) => {
                    if self.struct_name.is_some() {
                        return Err(CompileError::at(
                            "this subset accepts one struct",
                            item.span(),
                        ));
                    }
                    self.reject_attrs(&item.attrs)?;
                    if !item.generics.params.is_empty() {
                        return Err(CompileError::at(
                            "generics are outside this subset",
                            item.span(),
                        ));
                    }
                    let Fields::Named(fields) = &item.fields else {
                        return Err(CompileError::at(
                            "struct fields must be named f64 fields",
                            item.span(),
                        ));
                    };
                    if fields.named.is_empty() {
                        return Err(CompileError::at("struct needs an f64 field", item.span()));
                    }
                    for field in &fields.named {
                        self.reject_attrs(&field.attrs)?;
                        let name = field.ident.as_ref().unwrap();
                        if self.ty(&field.ty)? != Ty::F64 {
                            return Err(CompileError::at(
                                "struct fields must be f64",
                                field.span(),
                            ));
                        }
                        self.struct_fields.push(name.to_string());
                    }
                    self.struct_name = Some(item.ident.to_string());
                }
                Item::Enum(item) => {
                    if self.enum_name.is_some() {
                        return Err(CompileError::at(
                            "this subset accepts one enum",
                            item.span(),
                        ));
                    }
                    self.reject_attrs(&item.attrs)?;
                    if !item.generics.params.is_empty() {
                        return Err(CompileError::at(
                            "generics are outside this subset",
                            item.span(),
                        ));
                    }
                    for variant in &item.variants {
                        self.reject_attrs(&variant.attrs)?;
                        if variant.discriminant.is_some() {
                            return Err(CompileError::at(
                                "enum discriminants are outside this subset",
                                variant.span(),
                            ));
                        }
                        let fields = match &variant.fields {
                            Fields::Unit => Vec::new(),
                            Fields::Named(named_fields) => {
                                let mut names = Vec::new();
                                for field in &named_fields.named {
                                    let name = field.ident.as_ref().unwrap();
                                    if self.ty(&field.ty)? != Ty::F64 {
                                        return Err(CompileError::at(
                                            "enum fields must be f64",
                                            field.span(),
                                        ));
                                    }
                                    names.push(name.to_string());
                                }
                                if names.is_empty() {
                                    return Err(CompileError::at(
                                        "the field-carrying variant needs an f64 field",
                                        variant.span(),
                                    ));
                                }
                                names
                            }
                            Fields::Unnamed(_) => {
                                return Err(CompileError::at(
                                    "tuple variants are outside this subset",
                                    variant.span(),
                                ));
                            }
                        };
                        self.variants
                            .insert(variant.ident.to_string(), Variant { fields });
                    }
                    self.enum_name = Some(item.ident.to_string());
                }
                Item::Use(_) | Item::Fn(_) => {}
                _ => unreachable!("rejected before collect"),
            }
        }
        Ok(())
    }

    fn compile_run(&mut self, file: &File) -> Result<(), CompileError> {
        let mut helpers = Vec::new();
        let mut run = None;
        for item in &file.items {
            let Item::Fn(func) = item else {
                continue;
            };
            if func.sig.ident == "run" {
                if run.is_some() {
                    return Err(CompileError::at("a file has one run function", func.span()));
                }
                run = Some(func);
            } else {
                helpers.push(func);
            }
        }
        let Some(run) = run else {
            return Err(CompileError {
                message: "the file needs async fn run".into(),
                line: 1,
                column: 1,
            });
        };
        let mut next_id = 1u32;
        for func in &helpers {
            if func.sig.asyncness.is_some() {
                return Err(CompileError::at("helpers cannot be async", func.span()));
            }
            let (params, ret) = self.signature(func)?;
            let name = func.sig.ident.to_string();
            if self.helpers.contains_key(&name) {
                return Err(CompileError::at("duplicate helper", func.span()));
            }
            self.helpers.insert(
                name,
                HelperSig {
                    id: next_id,
                    params,
                    ret,
                },
            );
            next_id += 1;
        }
        self.next_func = next_id;
        for func in &helpers {
            let name = func.sig.ident.to_string();
            let helper = self.helpers.get(&name).cloned().unwrap();
            self.compile_detached(helper.id, &name, |compiler| {
                compiler.compile_fn_body(func, helper.ret, false)
            })?;
        }
        self.entry = true;
        self.compile_fn_body(run, self.signature(run)?.1, true)
    }

    fn signature(&self, func: &syn::ItemFn) -> Result<(Vec<Ty>, Ty), CompileError> {
        self.reject_attrs(&func.attrs)?;
        if !func.sig.generics.params.is_empty() || func.sig.generics.where_clause.is_some() {
            return Err(CompileError::at(
                "generics are outside this subset",
                func.span(),
            ));
        }
        let ReturnType::Type(_, ret_ty) = &func.sig.output else {
            return Err(CompileError::at(
                "functions must declare a return type",
                func.span(),
            ));
        };
        let ret = self.ty(ret_ty)?;
        if matches!(ret, Ty::Host | Ty::Never | Ty::Closure) {
            return Err(CompileError::at("unsupported return type", ret_ty.span()));
        }
        let mut params = Vec::new();
        for input in &func.sig.inputs {
            let FnArg::Typed(pat) = input else {
                return Err(CompileError::at(
                    "functions have no self parameter",
                    input.span(),
                ));
            };
            params.push(self.ty(&pat.ty)?);
        }
        Ok((params, ret))
    }

    fn compile_fn_body(
        &mut self,
        func: &syn::ItemFn,
        ret: Ty,
        entry: bool,
    ) -> Result<(), CompileError> {
        if entry && func.sig.asyncness.is_none() {
            return Err(CompileError::at("run must be async fn", func.span()));
        }
        self.ret = ret;
        self.entry = entry;
        for input in &func.sig.inputs {
            let FnArg::Typed(pat) = input else {
                return Err(CompileError::at(
                    "functions have no self parameter",
                    input.span(),
                ));
            };
            let Pat::Ident(ident) = &*pat.pat else {
                return Err(CompileError::at(
                    "parameters must be plain identifiers",
                    pat.span(),
                ));
            };
            if ident.by_ref.is_some() || ident.subpat.is_some() {
                return Err(CompileError::at(
                    "references are outside this subset",
                    ident.span(),
                ));
            }
            let ty = self.ty(&pat.ty)?;
            self.bind(ident.ident.to_string(), ty, ident.mutability.is_some());
        }
        self.param_count = self.next_local;
        let produced = self.block(&func.block, true)?;
        if self.terminated() {
            return Ok(());
        }
        if produced != Some(self.ret) {
            return Err(CompileError::at(
                "the function body does not match its return type",
                func.span(),
            ));
        }
        self.emit(Instruction::Return);
        Ok(())
    }

    fn terminated(&self) -> bool {
        matches!(self.instructions.last(), Some(Instruction::Return))
    }

    fn compile_detached<T>(
        &mut self,
        id: u32,
        name: &str,
        build: impl FnOnce(&mut Self) -> Result<T, CompileError>,
    ) -> Result<T, CompileError> {
        let saved = self.take_fn();
        self.clear_fn();
        let built = build(self);
        if built.is_ok() {
            let function = self.seal_function(id, name);
            self.extras.push(function);
        }
        self.put_fn(saved);
        built
    }

    fn take_fn(&mut self) -> FnState {
        FnState {
            names: std::mem::take(&mut self.names),
            moved: std::mem::take(&mut self.moved),
            instructions: std::mem::take(&mut self.instructions),
            next_local: self.next_local,
            param_count: self.param_count,
            ret: self.ret,
            loop_binding: self.loop_binding.take(),
            captures: std::mem::take(&mut self.captures),
            loops: std::mem::take(&mut self.loops),
            entry: self.entry,
            expected: self.expected.take(),
            pending_closure: self.pending_closure.take(),
        }
    }

    fn put_fn(&mut self, state: FnState) {
        self.names = state.names;
        self.moved = state.moved;
        self.instructions = state.instructions;
        self.next_local = state.next_local;
        self.param_count = state.param_count;
        self.ret = state.ret;
        self.loop_binding = state.loop_binding;
        self.captures = state.captures;
        self.loops = state.loops;
        self.entry = state.entry;
        self.expected = state.expected;
        self.pending_closure = state.pending_closure;
    }

    fn clear_fn(&mut self) {
        self.names.clear();
        self.moved.clear();
        self.instructions.clear();
        self.next_local = 0;
        self.param_count = 0;
        self.ret = Ty::Unit;
        self.loop_binding = None;
        self.captures.clear();
        self.loops.clear();
        self.entry = false;
        self.expected = None;
        self.pending_closure = None;
    }

    fn seal_function(&mut self, id: u32, name: &str) -> Function {
        let instructions = std::mem::take(&mut self.instructions);
        Function {
            id: FuncId(id),
            name: name.into(),
            param_count: self.param_count,
            local_count: self.next_local,
            param_defaults: Vec::new(),
            spans: vec![None; instructions.len()],
            instructions,
        }
    }

    fn finish(self) -> Result<Artifact, CompileError> {
        let mut functions = vec![Function {
            id: FuncId(0),
            name: "run".into(),
            param_count: self.param_count,
            local_count: self.next_local,
            param_defaults: Vec::new(),
            spans: vec![None; self.instructions.len()],
            instructions: self.instructions,
        }];
        functions.extend(self.extras);
        let (features, caps) = required_features(&functions);
        Ok(Artifact {
            envelope: Envelope {
                artifact_hash: String::new(),
                frontend_id: FRONTEND_RUST.into(),
                frontend_version: crate::PACKAGE_VERSION.into(),
                language_semantics_version: LANGUAGE_SEMANTICS_RUST.into(),
                engine_format_version: ENGINE_FORMAT_VERSION,
                required_engine_features: features,
                required_host_capabilities: caps,
                runtime_modules: Vec::new(),
            },
            program: Program {
                entry: FuncId(0),
                functions,
            },
        })
    }

    fn bind(&mut self, name: String, ty: Ty, mutable: bool) -> u32 {
        let id = self.next_local;
        self.next_local += 1;
        self.names.insert(name, Slot { id, ty, mutable });
        id
    }

    fn fresh(&mut self) -> u32 {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    fn emit(&mut self, instruction: Instruction) -> usize {
        self.instructions.push(instruction);
        self.instructions.len() - 1
    }

    fn patch(&mut self, index: usize) {
        let target = Pc(self.instructions.len() as u32);
        match &mut self.instructions[index] {
            Instruction::Jump { target: slot }
            | Instruction::JumpIfTrue { target: slot }
            | Instruction::JumpIfFalse { target: slot } => *slot = target,
            _ => panic!("patch target is not a jump"),
        }
    }

    fn load(&mut self, local: u32) {
        self.emit(Instruction::LoadLocal {
            local: LocalId(local),
        });
    }

    fn store(&mut self, local: u32) {
        self.emit(Instruction::StoreLocal {
            local: LocalId(local),
        });
    }

    fn clear(&mut self, local: u32) {
        self.emit(Instruction::LoadConst {
            value: ConstValue::Undefined,
        });
        self.store(local);
    }

    fn number(&mut self, number: f64) {
        self.emit(Instruction::LoadConst {
            value: ConstValue::Number(number),
        });
    }

    fn string(&mut self, text: &str) {
        self.emit(Instruction::LoadConst {
            value: ConstValue::String(text.to_string()),
        });
    }

    fn block(&mut self, block: &Block, tail: bool) -> Result<Option<Ty>, CompileError> {
        if block.stmts.is_empty() {
            return Ok(None);
        }
        for (index, stmt) in block.stmts.iter().enumerate() {
            let last = index + 1 == block.stmts.len();
            if last && tail {
                if let Stmt::Expr(expr, None) = stmt {
                    return Ok(Some(self.expr_move(expr, true)?));
                }
            }
            self.stmt(stmt)?;
        }
        Ok(None)
    }

    fn stmt(&mut self, stmt: &Stmt) -> Result<(), CompileError> {
        match stmt {
            Stmt::Local(local) => {
                let init = local
                    .init
                    .as_ref()
                    .ok_or_else(|| CompileError::at("let needs an initializer", local.span()))?;
                match &local.pat {
                    Pat::Ident(_) | Pat::Type(_) => {
                        let (name, mutable, ascribed) = self.binding(&local.pat)?;
                        let previous = self.expected.take();
                        if let Some(ty) = ascribed {
                            self.expected = Some(ty);
                        }
                        let (mut ty, moved_from) = if let Expr::Path(path) = &*init.expr {
                            self.load_local(path)?
                        } else {
                            (self.expr(&init.expr)?, None)
                        };
                        self.expected = previous;
                        if ty == Ty::Host {
                            ty = ascribed.ok_or_else(|| {
                                CompileError::at("this value needs a type ascription", local.span())
                            })?;
                        }
                        if let Some(want) = ascribed {
                            if ty != want {
                                return Err(CompileError::at(
                                    "initializer does not match its type",
                                    local.span(),
                                ));
                            }
                        }
                        if ty == Ty::Closure {
                            let sig = self.pending_closure.take().ok_or_else(|| {
                                CompileError::at(
                                    "internal: closure signature missing",
                                    local.span(),
                                )
                            })?;
                            self.closure_sigs.insert(name.clone(), sig);
                        }
                        let id = self.bind(name, ty, mutable);
                        self.store(id);
                        if let Some((source_name, source)) = moved_from {
                            self.clear(source);
                            self.moved.insert(source_name, ());
                        }
                    }
                    Pat::Wild(_) => {
                        self.expr(&init.expr)?;
                        self.emit(Instruction::Pop);
                    }
                    _ => {
                        return Err(CompileError::at(
                            "let patterns must be an identifier",
                            local.span(),
                        ))
                    }
                }
                Ok(())
            }
            Stmt::Expr(expr, _) => {
                if matches!(expr, Expr::Return(_) | Expr::Break(_) | Expr::Continue(_)) {
                    self.expr(expr)?;
                    return Ok(());
                }
                let ty = self.expr(expr)?;
                if ty == Ty::Closure {
                    return Err(CompileError::at(
                        "bind a closure before calling it",
                        expr.span(),
                    ));
                }
                if ty != Ty::Never {
                    self.emit(Instruction::Pop);
                }
                Ok(())
            }
            Stmt::Macro(mac) => Err(CompileError::at(
                "macros are outside this subset",
                mac.span(),
            )),
            Stmt::Item(item) => Err(CompileError::at(
                "helpers are outside this subset",
                item.span(),
            )),
        }
    }

    fn expr(&mut self, expr: &Expr) -> Result<Ty, CompileError> {
        self.expr_move(expr, false)
    }

    /// `consume` moves a path whose type moves. Copies stay in the source local.
    fn expr_move(&mut self, expr: &Expr, consume: bool) -> Result<Ty, CompileError> {
        match expr {
            Expr::Lit(lit) => self.lit(&lit.lit),
            Expr::Path(path) => self.path_value(path, consume),
            Expr::Field(field) => self.field(field),
            Expr::Binary(binary) => self.binary(binary),
            Expr::Assign(assign) => self.assign(&assign.left, &assign.right, None),
            Expr::Cast(cast) => self.cast(cast),
            Expr::Call(call) => self.call(call),
            Expr::MethodCall(call) => self.method(call),
            Expr::If(expr) => self.if_expr(expr),
            Expr::ForLoop(expr) => self.for_loop(expr),
            Expr::Match(expr) => self.match_expr(expr),
            Expr::Block(block) => self
                .block(&block.block, true)?
                .ok_or_else(|| CompileError::at("block must produce a value", block.span())),
            Expr::Try(try_expr) => self.question(try_expr),
            Expr::Paren(paren) => self.expr_move(&paren.expr, consume),
            Expr::Unary(unary) => self.unary(unary),
            Expr::Return(ret) => self.return_expr(ret),
            Expr::Closure(closure) => self.closure(closure),
            Expr::Index(index) => self.index(index),
            Expr::While(expr) => self.while_loop(expr),
            Expr::Loop(expr) => self.loop_expr(expr),
            Expr::Break(expr) => self.break_expr(expr),
            Expr::Continue(expr) => self.continue_expr(expr),
            Expr::Reference(_) => Err(CompileError::at(
                "references are outside this subset",
                expr.span(),
            )),
            Expr::Await(_) => Err(CompileError::at(
                "await a durable operation with ?",
                expr.span(),
            )),
            Expr::Macro(_) => Err(CompileError::at(
                "macros are outside this subset",
                expr.span(),
            )),
            _ => Err(CompileError::at("outside this subset", expr.span())),
        }
    }

    fn lit(&mut self, lit: &Lit) -> Result<Ty, CompileError> {
        match lit {
            Lit::Float(lit) => {
                let number = lit
                    .base10_parse::<f64>()
                    .map_err(|_| CompileError::at("expected an f64 literal", lit.span()))?;
                if !number.is_finite() {
                    return Err(CompileError::at(
                        "integers and floats must be finite f64 values",
                        lit.span(),
                    ));
                }
                self.number(number);
                Ok(Ty::F64)
            }
            Lit::Int(lit) => {
                let number = lit.base10_parse::<i64>().map_err(|_| {
                    CompileError::at("expected a small integer literal", lit.span())
                })?;
                let float = number as f64;
                if float as i64 != number {
                    return Err(CompileError::at(
                        "integers must be exact f64 values",
                        lit.span(),
                    ));
                }
                self.number(float);
                Ok(Ty::F64)
            }
            Lit::Bool(lit) => {
                self.emit(Instruction::LoadConst {
                    value: ConstValue::Bool(lit.value),
                });
                Ok(Ty::Bool)
            }
            Lit::Str(lit) => {
                self.string(&lit.value());
                Ok(Ty::String)
            }
            _ => Err(CompileError::at("outside this subset", lit.span())),
        }
    }

    /// Load a local. A moving local is returned so the caller can clear it after storing.
    fn load_local(
        &mut self,
        path: &syn::ExprPath,
    ) -> Result<(Ty, Option<(String, u32)>), CompileError> {
        let Some(name) = single_ident(&path.path) else {
            return Err(CompileError::at("let can move a local", path.span()));
        };
        if self.moved.contains_key(&name) {
            return Err(CompileError::at(
                format!("use of moved value `{name}`"),
                path.span(),
            ));
        }
        let Some(slot) = self.names.get(&name).cloned() else {
            if let Some(ty) = self.capture_value(&name, true, path.span())? {
                return Ok((ty, None));
            }
            return Err(CompileError::at(
                format!("unknown name `{name}`"),
                path.span(),
            ));
        };
        self.load(slot.id);
        if slot.ty.moves() {
            Ok((slot.ty, Some((name, slot.id))))
        } else {
            Ok((slot.ty, None))
        }
    }

    fn path_value(&mut self, path: &syn::ExprPath, consume: bool) -> Result<Ty, CompileError> {
        if let Some((enum_name, variant)) = two_idents(&path.path) {
            return self.unit_variant(&enum_name, &variant, path.span());
        }
        let Some(name) = single_ident(&path.path) else {
            return Err(CompileError::at(
                "paths must be locals, Ok, Err, Some, or None",
                path.span(),
            ));
        };
        if name == "None" {
            return self.none_value();
        }
        if self.moved.contains_key(&name) {
            return Err(CompileError::at(
                format!("use of moved value `{name}`"),
                path.span(),
            ));
        }
        let Some(slot) = self.names.get(&name).cloned() else {
            if let Some(ty) = self.capture_value(&name, consume, path.span())? {
                return Ok(ty);
            }
            return Err(CompileError::at(
                format!("unknown name `{name}`"),
                path.span(),
            ));
        };
        self.load(slot.id);
        if consume && slot.ty.moves() {
            let tmp = self.fresh();
            self.store(tmp);
            self.clear(slot.id);
            self.moved.insert(name, ());
            self.load(tmp);
        }
        Ok(slot.ty)
    }

    fn field(&mut self, field: &syn::ExprField) -> Result<Ty, CompileError> {
        let name = match &field.member {
            syn::Member::Named(ident) => ident.to_string(),
            syn::Member::Unnamed(_) => {
                return Err(CompileError::at(
                    "tuple fields are outside this subset",
                    field.span(),
                ))
            }
        };
        if name.starts_with('$') || name == "tag" {
            return Err(CompileError::at(
                "enum representation is not a user field",
                field.span(),
            ));
        }
        let Expr::Path(base) = &*field.base else {
            return Err(CompileError::at(
                "field access must start from a local",
                field.span(),
            ));
        };
        let ty = self.path_value(base, false)?;
        if ty != Ty::Struct {
            return Err(CompileError::at(
                "field access is only supported on the struct",
                field.span(),
            ));
        }
        if !self.struct_fields.iter().any(|field| field == &name) {
            return Err(CompileError::at(
                format!("unknown field `{name}`"),
                field.span(),
            ));
        }
        self.emit(Instruction::GetProp { key: name });
        Ok(Ty::F64)
    }

    fn binary(&mut self, binary: &syn::ExprBinary) -> Result<Ty, CompileError> {
        let arith = match binary.op {
            BinOp::Add(_) => Some(Instruction::Add),
            BinOp::Sub(_) => Some(Instruction::Sub),
            BinOp::Mul(_) => Some(Instruction::Mul),
            BinOp::Div(_) => Some(Instruction::Div),
            BinOp::Rem(_) => Some(Instruction::Rem),
            BinOp::AddAssign(_) => {
                return self.assign(&binary.left, &binary.right, Some(Instruction::Add))
            }
            BinOp::SubAssign(_) => {
                return self.assign(&binary.left, &binary.right, Some(Instruction::Sub))
            }
            BinOp::MulAssign(_) => {
                return self.assign(&binary.left, &binary.right, Some(Instruction::Mul))
            }
            BinOp::DivAssign(_) => {
                return self.assign(&binary.left, &binary.right, Some(Instruction::Div))
            }
            BinOp::RemAssign(_) => {
                return self.assign(&binary.left, &binary.right, Some(Instruction::Rem))
            }
            BinOp::And(_) | BinOp::Or(_) => return self.short_circuit(binary),
            BinOp::Eq(_)
            | BinOp::Ne(_)
            | BinOp::Lt(_)
            | BinOp::Le(_)
            | BinOp::Gt(_)
            | BinOp::Ge(_) => None,
            _ => {
                return Err(CompileError::at(
                    "that operator is outside this subset",
                    binary.span(),
                ))
            }
        };
        let left = self.expr(&binary.left)?;
        let right = self.expr(&binary.right)?;
        if let Some(op) = arith {
            if left != Ty::F64 || right != Ty::F64 {
                return Err(CompileError::at("arithmetic requires f64", binary.span()));
            }
            self.emit(op);
            return Ok(Ty::F64);
        }
        let op = match binary.op {
            BinOp::Eq(_) => Instruction::StrictEq,
            BinOp::Ne(_) => Instruction::StrictNeq,
            BinOp::Lt(_) => Instruction::Lt,
            BinOp::Le(_) => Instruction::Le,
            BinOp::Gt(_) => Instruction::Gt,
            BinOp::Ge(_) => Instruction::Ge,
            _ => unreachable!("comparison operator"),
        };
        let comparable = matches!(
            (left, right),
            (Ty::F64, Ty::F64) | (Ty::Bool, Ty::Bool) | (Ty::String, Ty::String)
        );
        if !comparable {
            return Err(CompileError::at(
                "compare f64, bool, or String; match structs and enums",
                binary.span(),
            ));
        }
        if !matches!(binary.op, BinOp::Eq(_) | BinOp::Ne(_))
            && (left != Ty::F64 || right != Ty::F64)
        {
            return Err(CompileError::at(
                "ordered comparison requires f64",
                binary.span(),
            ));
        }
        self.emit(op);
        Ok(Ty::Bool)
    }

    fn assign(
        &mut self,
        left: &Expr,
        right: &Expr,
        arith: Option<Instruction>,
    ) -> Result<Ty, CompileError> {
        match left {
            Expr::Path(path) => {
                let Some(name) = single_ident(&path.path) else {
                    return Err(CompileError::at(
                        "assignment target must be a local",
                        left.span(),
                    ));
                };
                let Some(slot) = self.names.get(&name).cloned() else {
                    return Err(CompileError::at(
                        format!("unknown name `{name}`"),
                        left.span(),
                    ));
                };
                if !slot.mutable {
                    return Err(CompileError::at(
                        format!("cannot assign to immutable `{name}`"),
                        left.span(),
                    ));
                }
                if slot.ty != Ty::F64 {
                    return Err(CompileError::at(
                        "assignment of a compound value is a move, written as let",
                        left.span(),
                    ));
                }
                if arith.is_some() {
                    self.load(slot.id);
                }
                let ty = self.expr(right)?;
                if ty != Ty::F64 {
                    return Err(CompileError::at("assignment requires f64", right.span()));
                }
                if let Some(op) = arith {
                    self.emit(op);
                }
                self.store(slot.id);
                self.emit(Instruction::LoadConst {
                    value: ConstValue::Undefined,
                });
                Ok(Ty::F64)
            }
            Expr::Field(field) => {
                let key = match &field.member {
                    syn::Member::Named(ident) => ident.to_string(),
                    syn::Member::Unnamed(_) => {
                        return Err(CompileError::at(
                            "tuple fields are outside this subset",
                            field.span(),
                        ))
                    }
                };
                let Expr::Path(base) = &*field.base else {
                    return Err(CompileError::at(
                        "field assignment must start from a local",
                        field.span(),
                    ));
                };
                let Some(name) = single_ident(&base.path) else {
                    return Err(CompileError::at(
                        "field assignment must start from a local",
                        field.span(),
                    ));
                };
                let Some(slot) = self.names.get(&name).cloned() else {
                    return Err(CompileError::at(
                        format!("unknown name `{name}`"),
                        field.span(),
                    ));
                };
                if slot.ty != Ty::Struct || !slot.mutable {
                    return Err(CompileError::at(
                        "field assignment requires a mutable struct local",
                        field.span(),
                    ));
                }
                if !self.struct_fields.iter().any(|field| field == &key) {
                    return Err(CompileError::at(
                        format!("unknown field `{key}`"),
                        field.span(),
                    ));
                }
                if arith.is_some() {
                    self.load(slot.id);
                    self.emit(Instruction::GetProp { key: key.clone() });
                }
                let ty = self.expr(right)?;
                if ty != Ty::F64 {
                    return Err(CompileError::at(
                        "field assignment requires f64",
                        right.span(),
                    ));
                }
                if let Some(op) = arith {
                    self.emit(op);
                }
                let value = self.fresh();
                self.store(value);
                self.load(slot.id);
                self.load(value);
                self.emit(Instruction::SetProp { key });
                Ok(Ty::Struct)
            }
            _ => Err(CompileError::at(
                "assignment target must be a local or a struct field",
                left.span(),
            )),
        }
    }

    fn cast(&mut self, cast: &syn::ExprCast) -> Result<Ty, CompileError> {
        if self.ty(&cast.ty)? != Ty::F64 {
            return Err(CompileError::at("only `as f64` is supported", cast.span()));
        }
        let Expr::Path(path) = &*cast.expr else {
            return Err(CompileError::at(
                "as f64 is only supported on the range loop binding",
                cast.span(),
            ));
        };
        let Some(name) = single_ident(&path.path) else {
            return Err(CompileError::at(
                "as f64 is only supported on the range loop binding",
                cast.span(),
            ));
        };
        if self.loop_binding.as_deref() != Some(name.as_str()) {
            return Err(CompileError::at(
                "as f64 is only supported on the range loop binding",
                cast.span(),
            ));
        }
        self.path_value(path, false)
    }

    fn call(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        if let Some(text) = string_from_call(call) {
            self.string(&text);
            return Ok(Ty::String);
        }
        let Expr::Path(path) = &*call.func else {
            return Err(CompileError::at("unsupported call", call.span()));
        };
        if let Some(name) = single_ident(&path.path) {
            if let Some(slot) = self.names.get(&name).cloned() {
                if slot.ty == Ty::Closure {
                    return self.call_closure(call, &name, slot.id);
                }
            }
            if self.helpers.contains_key(&name) {
                return self.call_helper(call, &name);
            }
            if matches!(
                name.as_str(),
                "wait_for_event" | "effect" | "sleep" | "invoke" | "join" | "race"
            ) {
                return Err(CompileError::at(
                    "await a durable operation with ?",
                    call.span(),
                ));
            }
        }
        if path_string(&path.path) == "Vec::new" {
            self.emit(Instruction::NewArray);
            return Ok(Ty::VecF64);
        }
        let Some(name) = single_ident(&path.path) else {
            return Err(CompileError::at("unsupported call", call.span()));
        };
        if name == "Some" {
            if call.args.len() != 1 {
                return Err(CompileError::at("Some takes one argument", call.span()));
            }
            let payload = self.expr_move(&call.args[0], true)?;
            if payload != Ty::F64 {
                return Err(CompileError::at("only Some(f64) is supported", call.span()));
            }
            self.wrap("Some");
            return Ok(Ty::OptionF64);
        }
        if call.args.len() != 1 {
            return Err(CompileError::at(
                "Ok and Err take one argument",
                call.span(),
            ));
        }
        let payload = self.expr_move(&call.args[0], true)?;
        let inner = result_inner(self.ret);
        match (name.as_str(), inner, payload) {
            ("Ok", Some(expected), got) if expected == got => {
                self.wrap("Ok");
                Ok(self.ret)
            }
            ("Err", Some(_), Ty::String) => {
                self.wrap("Err");
                Ok(self.ret)
            }
            ("Ok" | "Err", _, _) => Err(CompileError::at(
                "Ok and Err do not match the return type",
                call.span(),
            )),
            _ => Err(CompileError::at("unsupported call", call.span())),
        }
    }

    fn method(&mut self, call: &syn::ExprMethodCall) -> Result<Ty, CompileError> {
        if call.method == "into" && call.args.is_empty() && call.turbofish.is_none() {
            if let Expr::Lit(syn::ExprLit {
                lit: Lit::Str(text),
                ..
            }) = &*call.receiver
            {
                self.string(&text.value());
                return Ok(Ty::String);
            }
        }
        if call.method == "push_str" {
            return Err(CompileError::at(
                "string mutation is outside this subset",
                call.span(),
            ));
        }
        if call.method == "len" && call.args.is_empty() && call.turbofish.is_none() {
            let ty = self.expr(&call.receiver)?;
            if ty != Ty::VecF64 {
                return Err(CompileError::at(
                    "len is supported on Vec<f64>",
                    call.span(),
                ));
            }
            self.emit(Instruction::Length);
            return Ok(Ty::F64);
        }
        if call.method == "push" && call.args.len() == 1 && call.turbofish.is_none() {
            let ty = self.expr(&call.receiver)?;
            if ty != Ty::VecF64 {
                return Err(CompileError::at(
                    "push is supported on Vec<f64>",
                    call.span(),
                ));
            }
            let value = self.expr_move(&call.args[0], true)?;
            if value != Ty::F64 {
                return Err(CompileError::at("Vec<f64> push takes f64", call.span()));
            }
            self.emit(Instruction::ArrayPush);
            self.emit(Instruction::Pop);
            self.emit(Instruction::LoadConst {
                value: ConstValue::Undefined,
            });
            return Ok(Ty::Unit);
        }
        Err(CompileError::at(
            "methods are outside this subset",
            call.span(),
        ))
    }

    fn question(&mut self, try_expr: &syn::ExprTry) -> Result<Ty, CompileError> {
        let inner = if let Expr::Await(await_expr) = &*try_expr.expr {
            let payload = self.durable(&await_expr.base)?;
            self.wrap("Ok");
            payload
        } else {
            let ty = self.expr(&try_expr.expr)?;
            result_inner(ty)
                .ok_or_else(|| CompileError::at("? requires a Result", try_expr.span()))?
        };
        self.unwrap_result(inner)
    }

    fn if_expr(&mut self, expr: &syn::ExprIf) -> Result<Ty, CompileError> {
        let cond = self.expr(&expr.cond)?;
        if cond != Ty::Bool {
            return Err(CompileError::at("if requires a bool", expr.cond.span()));
        }
        let to_else = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
        let then_ty = self
            .block(&expr.then_branch, true)?
            .ok_or_else(|| CompileError::at("if branch must produce a value", expr.span()))?;
        let to_end = self.emit(Instruction::Jump { target: Pc(0) });
        self.patch(to_else);
        let Some((_, else_branch)) = &expr.else_branch else {
            return Err(CompileError::at("if must have an else", expr.span()));
        };
        let else_ty = self.expr(else_branch)?;
        if then_ty != else_ty {
            return Err(CompileError::at(
                "if branches have different types",
                expr.span(),
            ));
        }
        self.patch(to_end);
        Ok(then_ty)
    }

    fn for_loop(&mut self, expr: &syn::ExprForLoop) -> Result<Ty, CompileError> {
        if expr.label.is_some() {
            return Err(CompileError::at(
                "loop labels are outside this subset",
                expr.span(),
            ));
        }
        let Pat::Ident(pat) = &*expr.pat else {
            return Err(CompileError::at(
                "for binds one identifier",
                expr.pat.span(),
            ));
        };
        if pat.by_ref.is_some() || pat.mutability.is_some() {
            return Err(CompileError::at(
                "the range binding is not a reference",
                pat.span(),
            ));
        }
        let name = pat.ident.to_string();
        match &*expr.expr {
            Expr::Range(range) => self.for_range(&name, range, &expr.body),
            other => self.for_vec(&name, other, &expr.body),
        }
    }

    fn match_expr(&mut self, expr: &syn::ExprMatch) -> Result<Ty, CompileError> {
        let scrutinee = self.scrutinee(&expr.expr)?;
        let mut result = None;
        let mut ends = Vec::new();
        for arm in &expr.arms {
            if arm.guard.is_some() {
                return Err(CompileError::at(
                    "match guards are outside this subset",
                    arm.span(),
                ));
            }
            let (tag, binding) = self.arm_pattern(&arm.pat, scrutinee.ty)?;
            self.load(scrutinee.id);
            self.emit(Instruction::GetProp {
                key: TAG.to_string(),
            });
            self.string(&tag);
            self.emit(Instruction::StrictEq);
            let to_next = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
            if let Some((name, key)) = binding {
                self.load(scrutinee.id);
                self.emit(Instruction::GetProp { key });
                let id = self.bind(name.clone(), Ty::F64, false);
                self.store(id);
            }
            let ty = self.expr(&arm.body)?;
            if let Some(expected) = result {
                if expected != ty {
                    return Err(CompileError::at(
                        "match arms have different types",
                        arm.span(),
                    ));
                }
            } else {
                result = Some(ty);
            }
            ends.push(self.emit(Instruction::Jump { target: Pc(0) }));
            self.patch(to_next);
        }
        self.string("non-exhaustive match");
        self.emit(Instruction::Throw);
        for end in ends {
            self.patch(end);
        }
        result.ok_or_else(|| CompileError::at("match needs an arm", expr.span()))
    }

    fn scrutinee(&mut self, expr: &Expr) -> Result<Slot, CompileError> {
        let Expr::Path(path) = expr else {
            return Err(CompileError::at(
                "match subject must be a local",
                expr.span(),
            ));
        };
        let Some(name) = single_ident(&path.path) else {
            return Err(CompileError::at(
                "match subject must be a local",
                expr.span(),
            ));
        };
        let Some(slot) = self.names.get(&name).cloned() else {
            return Err(CompileError::at(
                format!("unknown name `{name}`"),
                expr.span(),
            ));
        };
        if !matches!(slot.ty, Ty::Enum | Ty::OptionF64) {
            return Err(CompileError::at(
                "match subject must be the enum or Option<f64>",
                expr.span(),
            ));
        }
        let dest = self.fresh();
        self.load(slot.id);
        self.store(dest);
        self.clear(slot.id);
        self.moved.insert(name, ());
        Ok(Slot {
            id: dest,
            ty: slot.ty,
            mutable: false,
        })
    }

    fn arm_pattern(
        &mut self,
        pat: &Pat,
        subject: Ty,
    ) -> Result<(String, Option<(String, String)>), CompileError> {
        match pat {
            Pat::TupleStruct(pat) => {
                if subject != Ty::OptionF64 || path_string(&pat.path) != "Some" {
                    return Err(CompileError::at("expected Some(n)", pat.span()));
                }
                if pat.elems.len() != 1 {
                    return Err(CompileError::at("Some binds one identifier", pat.span()));
                }
                let Pat::Ident(ident) = &pat.elems[0] else {
                    return Err(CompileError::at("Some binds one identifier", pat.span()));
                };
                Ok((
                    "Some".into(),
                    Some((ident.ident.to_string(), PAYLOAD.into())),
                ))
            }
            Pat::Struct(pat) => {
                if subject != Ty::Enum {
                    return Err(CompileError::at("expected the enum", pat.span()));
                }
                let (enum_name, variant) = two_idents(&pat.path)
                    .ok_or_else(|| CompileError::at("expected Enum::Variant", pat.span()))?;
                if self.enum_name.as_deref() != Some(enum_name.as_str()) {
                    return Err(CompileError::at("expected the enum", pat.span()));
                }
                let info = self.variants.get(&variant).ok_or_else(|| {
                    CompileError::at(format!("unknown variant `{variant}`"), pat.span())
                })?;
                if info.fields.len() != pat.fields.len() || pat.rest.is_some() {
                    return Err(CompileError::at(
                        "destructure every field of the variant",
                        pat.span(),
                    ));
                }
                if info.fields.len() != 1 {
                    return Err(CompileError::at(
                        "this subset destructures one field",
                        pat.span(),
                    ));
                }
                let field = &pat.fields[0];
                let syn::Member::Named(member) = &field.member else {
                    return Err(CompileError::at("expected a named field", field.span()));
                };
                if member != &info.fields[0] {
                    return Err(CompileError::at("unknown enum field", field.span()));
                }
                let Pat::Ident(ident) = &*field.pat else {
                    return Err(CompileError::at(
                        "enum field binds an identifier",
                        field.span(),
                    ));
                };
                Ok((
                    variant,
                    Some((ident.ident.to_string(), info.fields[0].clone())),
                ))
            }
            Pat::Path(path) => {
                let text = path_string(&path.path);
                if subject == Ty::OptionF64 && text == "None" {
                    return Ok(("None".into(), None));
                }
                if subject == Ty::Enum {
                    if let Some((enum_name, variant)) = two_idents(&path.path) {
                        if self.enum_name.as_deref() == Some(enum_name.as_str())
                            && self
                                .variants
                                .get(&variant)
                                .is_some_and(|info| info.fields.is_empty())
                        {
                            return Ok((variant, None));
                        }
                    }
                }
                Err(CompileError::at("unsupported match pattern", pat.span()))
            }
            Pat::Ident(ident) if subject == Ty::OptionF64 && ident.ident == "None" => {
                Ok(("None".into(), None))
            }
            _ => Err(CompileError::at("unsupported match pattern", pat.span())),
        }
    }

    fn wrap(&mut self, tag: &str) {
        let payload = self.fresh();
        self.store(payload);
        self.emit(Instruction::NewObject);
        self.string(tag);
        self.emit(Instruction::SetProp {
            key: TAG.to_string(),
        });
        self.load(payload);
        self.emit(Instruction::SetProp {
            key: PAYLOAD.to_string(),
        });
    }

    fn ty(&self, ty: &Type) -> Result<Ty, CompileError> {
        let Type::Path(TypePath { qself: None, path }) = ty else {
            if let Type::Tuple(tuple) = ty {
                if tuple.elems.is_empty() {
                    return Ok(Ty::Unit);
                }
            }
            return Err(CompileError::at("unsupported type", ty.span()));
        };
        if path.segments.len() != 1 {
            return Err(CompileError::at("unsupported type", ty.span()));
        }
        let segment = &path.segments[0];
        let name = segment.ident.to_string();
        match name.as_str() {
            "f64" => Ok(Ty::F64),
            "bool" => Ok(Ty::Bool),
            "String" => Ok(Ty::String),
            "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64"
            | "u128" | "usize" => Err(CompileError::at(
                "integer types are outside this subset",
                ty.span(),
            )),
            "Vec" => {
                let inner = generic_args(segment, 1)?;
                if self.ty(&inner[0])? != Ty::F64 {
                    return Err(CompileError::at("only Vec<f64> is supported", ty.span()));
                }
                Ok(Ty::VecF64)
            }
            "Option" => {
                let inner = generic_args(segment, 1)?;
                if self.ty(&inner[0])? != Ty::F64 {
                    return Err(CompileError::at("only Option<f64> is supported", ty.span()));
                }
                Ok(Ty::OptionF64)
            }
            "Result" => {
                let args = generic_args(segment, 2)?;
                let ok = self.ty(&args[0])?;
                let err = self.ty(&args[1])?;
                if err != Ty::String {
                    return Err(CompileError::at(
                        "Result's error type must be String",
                        ty.span(),
                    ));
                }
                match ok {
                    Ty::F64 => Ok(Ty::ResultF64),
                    Ty::Bool => Ok(Ty::ResultBool),
                    Ty::String => Ok(Ty::ResultString),
                    Ty::Unit => Ok(Ty::ResultUnit),
                    Ty::Struct => Ok(Ty::ResultStruct),
                    Ty::VecF64 => Ok(Ty::ResultVec),
                    _ => Err(CompileError::at("unsupported Result ok type", ty.span())),
                }
            }
            other if self.struct_name.as_deref() == Some(other) => Ok(Ty::Struct),
            other if self.enum_name.as_deref() == Some(other) => Ok(Ty::Enum),
            _ => Err(CompileError::at("unsupported type", ty.span())),
        }
    }

    fn reject_attrs(&self, attrs: &[syn::Attribute]) -> Result<(), CompileError> {
        let Some(attr) = attrs.first() else {
            return Ok(());
        };
        if attr.path().is_ident("derive") {
            let text = match &attr.meta {
                syn::Meta::List(list) => list.tokens.to_string(),
                _ => String::new(),
            };
            if text.contains("Copy") || text.contains("Clone") {
                return Err(CompileError::at(
                    "user-defined Copy and Clone are outside this subset",
                    attr.span(),
                ));
            }
        }
        Err(CompileError::at(
            "attributes are outside this subset",
            attr.span(),
        ))
    }
}

#[path = "surface_impl.rs"]
mod surface;

fn generic_args(segment: &syn::PathSegment, count: usize) -> Result<Vec<Type>, CompileError> {
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return Err(CompileError::at(
            "missing generic arguments",
            segment.span(),
        ));
    };
    let mut types = Vec::new();
    for arg in &args.args {
        let syn::GenericArgument::Type(ty) = arg else {
            return Err(CompileError::at(
                "unsupported generic argument",
                segment.span(),
            ));
        };
        types.push(ty.clone());
    }
    if types.len() != count {
        return Err(CompileError::at(
            "unexpected generic arguments",
            segment.span(),
        ));
    }
    Ok(types)
}

fn single_ident(path: &syn::Path) -> Option<String> {
    if path.segments.len() == 1 && path.segments[0].arguments.is_empty() {
        Some(path.segments[0].ident.to_string())
    } else {
        None
    }
}

fn two_idents(path: &syn::Path) -> Option<(String, String)> {
    if path.segments.len() == 2
        && path
            .segments
            .iter()
            .all(|segment| segment.arguments.is_empty())
    {
        Some((
            path.segments[0].ident.to_string(),
            path.segments[1].ident.to_string(),
        ))
    } else {
        None
    }
}

fn path_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn string_from_call(call: &syn::ExprCall) -> Option<String> {
    let Expr::Path(path) = &*call.func else {
        return None;
    };
    if path_string(&path.path) != "String::from" || call.args.len() != 1 {
        return None;
    }
    string_lit(&call.args[0])
}

fn string_lit(expr: &Expr) -> Option<String> {
    if let Expr::Lit(syn::ExprLit {
        lit: Lit::Str(text),
        ..
    }) = expr
    {
        Some(text.value())
    } else {
        None
    }
}

fn durable_name(expr: &Expr) -> Result<String, CompileError> {
    if let Some(text) = string_lit(expr) {
        return Ok(text);
    }
    if let Expr::Call(call) = expr {
        if let Some(text) = string_from_call(call) {
            return Ok(text);
        }
    }
    Err(CompileError::at(
        "durable names are string literals",
        expr.span(),
    ))
}

fn required_features(functions: &[Function]) -> (Vec<EngineFeature>, Vec<HostCapability>) {
    let mut effect = false;
    let mut sleep = false;
    let mut wait = false;
    let mut invoke = false;
    let mut join = false;
    for function in functions {
        for instruction in &function.instructions {
            match instruction {
                Instruction::Effect => effect = true,
                Instruction::Sleep => sleep = true,
                Instruction::WaitForEvent => wait = true,
                Instruction::Invoke { .. } => invoke = true,
                Instruction::Fork { .. } | Instruction::JoinAll | Instruction::JoinAny => {
                    join = true
                }
                _ => {}
            }
        }
    }
    let mut features = vec![
        EngineFeature(EngineFeature::TS_CONTROL_FLOW.into()),
        EngineFeature(EngineFeature::LANG_COMPUTE.into()),
    ];
    let mut caps = vec![HostCapability(HostCapability::PERSIST_CHECKPOINT.into())];
    if effect {
        features.push(EngineFeature(EngineFeature::DURABLE_EFFECT.into()));
        caps.push(HostCapability(HostCapability::EFFECT.into()));
    }
    if sleep {
        features.push(EngineFeature(EngineFeature::DURABLE_SLEEP.into()));
        caps.push(HostCapability(HostCapability::TIMER.into()));
    }
    if wait {
        features.push(EngineFeature(EngineFeature::DURABLE_WAIT_FOR_EVENT.into()));
        caps.push(HostCapability(HostCapability::EVENT.into()));
    }
    if invoke {
        features.push(EngineFeature(EngineFeature::DURABLE_INVOKE.into()));
        caps.push(HostCapability(HostCapability::CHILD.into()));
    }
    if join {
        features.push(EngineFeature(
            EngineFeature::DURABLE_CONCURRENT_GROUP.into(),
        ));
    }
    (features, caps)
}

struct CaptureWalk<'a> {
    names: &'a HashMap<String, Slot>,
    params: &'a HashSet<String>,
    seen: HashSet<String>,
    found: Vec<(String, Ty)>,
}

impl<'ast> Visit<'ast> for CaptureWalk<'ast> {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        if let Expr::Path(path) = expr {
            if let Some(name) = single_ident(&path.path) {
                if !self.params.contains(&name) && self.seen.insert(name.clone()) {
                    if let Some(slot) = self.names.get(&name) {
                        self.found.push((name, slot.ty));
                    }
                }
            }
        }
        syn::visit::visit_expr(self, expr);
    }
}
