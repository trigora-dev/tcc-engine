use super::*;

impl Compiler {
    pub(super) fn binding(
        &mut self,
        pat: &Pat,
    ) -> Result<(String, bool, Option<Ty>), CompileError> {
        match pat {
            Pat::Ident(ident) => {
                if ident.by_ref.is_some() || ident.subpat.is_some() {
                    return Err(CompileError::at(
                        "references are outside this subset",
                        ident.span(),
                    ));
                }
                Ok((ident.ident.to_string(), ident.mutability.is_some(), None))
            }
            Pat::Type(typed) => {
                let (name, mutable, _) = self.binding(&typed.pat)?;
                Ok((name, mutable, Some(self.ty(&typed.ty)?)))
            }
            _ => Err(CompileError::at(
                "let patterns must be an identifier",
                pat.span(),
            )),
        }
    }

    pub(super) fn undefined(&mut self) {
        self.emit(Instruction::LoadConst {
            value: ConstValue::Undefined,
        });
    }

    pub(super) fn require_entry(&self, span: proc_macro2::Span) -> Result<(), CompileError> {
        if self.entry {
            Ok(())
        } else {
            Err(CompileError::at(
                "durable operations are only allowed in the program entry",
                span,
            ))
        }
    }

    pub(super) fn capture_value(
        &mut self,
        name: &str,
        consume: bool,
        span: proc_macro2::Span,
    ) -> Result<Option<Ty>, CompileError> {
        let Some((index, ty)) = self.captures.get(name).copied() else {
            return Ok(None);
        };
        if self.moved.contains_key(name) {
            return Err(CompileError::at(
                format!("use of moved value `{name}`"),
                span,
            ));
        }
        self.load(0);
        self.emit(Instruction::EnvGet { index });
        if consume && self.moves(ty) {
            let tmp = self.fresh();
            self.store(tmp);
            self.load(0);
            self.undefined();
            self.emit(Instruction::EnvSet { index });
            self.moved.insert(name.to_string(), ());
            self.load(tmp);
        }
        Ok(Some(ty))
    }

    pub(super) fn unit_variant(
        &mut self,
        enum_name: &str,
        variant: &str,
        span: proc_macro2::Span,
    ) -> Result<Ty, CompileError> {
        let Some(ty) = self.enums.get(enum_name).copied() else {
            return Err(CompileError::at("expected the enum", span));
        };
        let Some(fields) = self.variant(ty, variant) else {
            return Err(CompileError::at(
                format!("unknown variant `{variant}`"),
                span,
            ));
        };
        if !matches!(fields, VariantFields::Unit) {
            return Err(CompileError::at(
                "construct a field-carrying variant with its fields",
                span,
            ));
        }
        self.emit(Instruction::NewObject);
        self.string(variant);
        self.emit(Instruction::SetProp {
            key: TAG.to_string(),
        });
        Ok(ty)
    }

    pub(super) fn none_value(&mut self, span: proc_macro2::Span) -> Result<Ty, CompileError> {
        let ty = self
            .expected
            .filter(|ty| self.option_inner(*ty).is_some())
            .or_else(|| self.option_inner(self.ret).map(|_| self.ret));
        let Some(ty) = ty else {
            return Err(CompileError::at("None needs a type ascription", span));
        };
        self.emit(Instruction::NewObject);
        self.string("None");
        self.emit(Instruction::SetProp {
            key: TAG.to_string(),
        });
        Ok(ty)
    }

    pub(super) fn short_circuit(&mut self, binary: &syn::ExprBinary) -> Result<Ty, CompileError> {
        let and = matches!(binary.op, BinOp::And(_));
        let left = self.expr(&binary.left)?;
        if left != self.ty_bool {
            return Err(CompileError::at("&& and || require bool", binary.span()));
        }
        let skip = if and {
            self.emit(Instruction::JumpIfFalse { target: Pc(0) })
        } else {
            self.emit(Instruction::JumpIfTrue { target: Pc(0) })
        };
        let right = self.expr(&binary.right)?;
        if right != self.ty_bool {
            return Err(CompileError::at("&& and || require bool", binary.span()));
        }
        let to_end = self.emit(Instruction::Jump { target: Pc(0) });
        self.patch(skip);
        self.emit(Instruction::LoadConst {
            value: ConstValue::Bool(!and),
        });
        self.patch(to_end);
        Ok(self.ty_bool)
    }

    pub(super) fn unary(&mut self, expr: &syn::ExprUnary) -> Result<Ty, CompileError> {
        match expr.op {
            syn::UnOp::Not(_) => {
                let ty = self.expr(&expr.expr)?;
                if ty != self.ty_bool {
                    return Err(CompileError::at("! requires bool", expr.span()));
                }
                self.emit(Instruction::Not);
                Ok(self.ty_bool)
            }
            syn::UnOp::Neg(_) => {
                let ty = self.expr(&expr.expr)?;
                if ty != self.ty_f64 {
                    return Err(CompileError::at("unary minus requires f64", expr.span()));
                }
                self.emit(Instruction::Neg);
                Ok(self.ty_f64)
            }
            _ => Err(CompileError::at("outside this subset", expr.span())),
        }
    }

    pub(super) fn return_expr(&mut self, expr: &syn::ExprReturn) -> Result<Ty, CompileError> {
        if let Some(value) = &expr.expr {
            let ty = self.expr_move(value, true)?;
            if ty != self.ret {
                return Err(CompileError::at(
                    "return does not match the function type",
                    expr.span(),
                ));
            }
        } else if self.ret != self.ty_unit {
            return Err(CompileError::at("return a value", expr.span()));
        } else {
            self.undefined();
        }
        self.emit(Instruction::Return);
        Ok(self.ty_never)
    }

    pub(super) fn index(&mut self, expr: &syn::ExprIndex) -> Result<Ty, CompileError> {
        let ty = self.expr(&expr.expr)?;
        let Some(elem) = self.vec_elem(ty) else {
            return Err(CompileError::at("index a Vec", expr.span()));
        };
        if self.moves(elem) {
            return Err(CompileError::at(
                "indexing a Vec would borrow; use for",
                expr.span(),
            ));
        }
        let index = self.expr(&expr.index)?;
        if index != self.ty_f64 {
            return Err(CompileError::at("Vec index is f64", expr.span()));
        }
        self.emit(Instruction::GetIndex);
        Ok(elem)
    }

    pub(super) fn call_helper(
        &mut self,
        call: &syn::ExprCall,
        name: &str,
    ) -> Result<Ty, CompileError> {
        let helper = self.helpers.get(name).cloned().unwrap();
        if call.args.len() != helper.params.len() {
            return Err(CompileError::at(
                "helper call has the wrong number of arguments",
                call.span(),
            ));
        }
        for (arg, expected) in call.args.iter().zip(&helper.params) {
            let ty = self.expr_move(arg, true)?;
            if ty != *expected {
                return Err(CompileError::at(
                    "helper argument has the wrong type",
                    arg.span(),
                ));
            }
        }
        self.emit(Instruction::Call {
            func: FuncId(helper.id),
            argc: helper.params.len() as u32,
        });
        Ok(helper.ret)
    }

    pub(super) fn call_closure(
        &mut self,
        call: &syn::ExprCall,
        name: &str,
        local: u32,
    ) -> Result<Ty, CompileError> {
        let sig =
            self.closure_sigs.get(name).cloned().ok_or_else(|| {
                CompileError::at("internal: closure signature missing", call.span())
            })?;
        if call.args.len() != sig.params.len() {
            return Err(CompileError::at(
                "closure call has the wrong number of arguments",
                call.span(),
            ));
        }
        self.load(local);
        for (arg, expected) in call.args.iter().zip(&sig.params) {
            let ty = self.expr_move(arg, true)?;
            if ty != *expected {
                return Err(CompileError::at(
                    "closure argument has the wrong type",
                    arg.span(),
                ));
            }
        }
        self.emit(Instruction::CallClosure {
            argc: sig.params.len() as u32,
        });
        Ok(sig.ret)
    }

    pub(super) fn unwrap_result(&mut self, inner: Ty) -> Result<Ty, CompileError> {
        let result = self.fresh();
        self.store(result);
        self.load(result);
        self.emit(Instruction::GetProp {
            key: TAG.to_string(),
        });
        self.string("Ok");
        self.emit(Instruction::StrictEq);
        let to_err = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
        self.load(result);
        self.emit(Instruction::GetProp {
            key: PAYLOAD.to_string(),
        });
        let to_end = self.emit(Instruction::Jump { target: Pc(0) });
        self.patch(to_err);
        self.load(result);
        self.emit(Instruction::GetProp {
            key: PAYLOAD.to_string(),
        });
        self.wrap("Err");
        self.emit(Instruction::Return);
        self.patch(to_end);
        Ok(inner)
    }

    pub(super) fn durable(&mut self, expr: &Expr) -> Result<Ty, CompileError> {
        self.require_entry(expr.span())?;
        let Expr::Call(call) = expr else {
            return Err(CompileError::at("await a durable call", expr.span()));
        };
        self.durable_call(call)
    }

    pub(super) fn durable_call(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        let Expr::Path(path) = &*call.func else {
            return Err(CompileError::at("await a durable call", call.span()));
        };
        let Some(name) = self.resolve_durable(&path.path)? else {
            return Err(CompileError::at("await a durable call", call.span()));
        };
        match name.as_str() {
            "wait_for_event" => self.op_wait(call),
            "effect" => self.op_effect(call),
            "sleep" => self.op_sleep(call),
            "invoke" => self.op_invoke(call),
            "join" => self.op_join(call, true),
            "race" => self.op_join(call, false),
            _ => Err(CompileError::at("await a durable call", call.span())),
        }
    }

    pub(super) fn op_wait(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        if call.args.len() != 1 {
            return Err(CompileError::at(
                "wait_for_event takes a string literal",
                call.span(),
            ));
        }
        let name = durable_name(&call.args[0])?;
        self.string(&name);
        self.emit(Instruction::WaitForEvent);
        Ok(self.expected.unwrap_or(self.ty_bool))
    }

    pub(super) fn op_effect(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        if call.args.len() != 2 {
            return Err(CompileError::at(
                "effect takes a string literal and a closure",
                call.span(),
            ));
        }
        let key = durable_name(&call.args[0])?;
        let Expr::Closure(closure) = &call.args[1] else {
            return Err(CompileError::at(
                "effect callback must be a closure",
                call.span(),
            ));
        };
        if closure.asyncness.is_some() {
            return Err(CompileError::at(
                "effect callbacks cannot be async",
                closure.span(),
            ));
        }
        if !closure.inputs.is_empty() {
            return Err(CompileError::at(
                "effect callbacks take no parameters",
                closure.span(),
            ));
        }
        let captures = self.find_captures(&closure.body, &HashSet::new());
        if !captures.is_empty() && closure.capture.is_none() {
            return Err(CompileError::at(
                "the move keyword is required when a closure captures",
                closure.span(),
            ));
        }
        let ty = self.type_discarded(&closure.body)?;
        self.emit(Instruction::NewObject);
        for (name, _) in &captures {
            self.capture_input(name, closure.span())?;
        }
        self.string(&key);
        self.emit(Instruction::Effect { has_input: true });
        Ok(ty)
    }

    pub(super) fn capture_input(
        &mut self,
        name: &str,
        span: proc_macro2::Span,
    ) -> Result<(), CompileError> {
        if self.moved.contains_key(name) {
            return Err(CompileError::at(
                format!("use of moved value `{name}`"),
                span,
            ));
        }
        let Some(slot) = self.names.get(name).cloned() else {
            return Err(CompileError::at(format!("unknown name `{name}`"), span));
        };
        self.load(slot.id);
        if self.moves(slot.ty) {
            self.clear(slot.id);
            self.moved.insert(name.to_string(), ());
        }
        self.emit(Instruction::SetProp {
            key: name.to_string(),
        });
        Ok(())
    }

    pub(super) fn op_sleep(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        if call.args.len() != 1 {
            return Err(CompileError::at("sleep takes an f64 duration", call.span()));
        }
        let ty = self.expr(&call.args[0])?;
        if ty != self.ty_f64 {
            return Err(CompileError::at("sleep takes an f64 duration", call.span()));
        }
        self.emit(Instruction::Sleep);
        Ok(self.ty_unit)
    }

    pub(super) fn op_invoke(&mut self, call: &syn::ExprCall) -> Result<Ty, CompileError> {
        if call.args.len() != 2 {
            return Err(CompileError::at(
                "invoke takes a program name and one argument value",
                call.span(),
            ));
        }
        let name = durable_name(&call.args[0])?;
        let argc = self.flatten_invoke(&call.args[1])?;
        self.string(&name);
        self.emit(Instruction::Invoke { arg_count: argc });
        self.expected
            .ok_or_else(|| CompileError::at("invoke needs a type ascription", call.span()))
    }

    pub(super) fn flatten_invoke(&mut self, expr: &Expr) -> Result<u32, CompileError> {
        if let Expr::Tuple(tuple) = expr {
            for element in &tuple.elems {
                self.expr_move(element, true)?;
            }
            return Ok(tuple.elems.len() as u32);
        }
        self.expr_move(expr, true)?;
        Ok(1)
    }

    pub(super) fn op_join(&mut self, call: &syn::ExprCall, all: bool) -> Result<Ty, CompileError> {
        if call.args.len() != 2 {
            return Err(CompileError::at(
                if all {
                    "join takes two branches"
                } else {
                    "race takes two branches"
                },
                call.span(),
            ));
        }
        let fork = self.emit(Instruction::Fork {
            count: 2,
            join_pc: Pc(0),
        });
        let left = self.branch(&call.args[0])?;
        let right = self.branch(&call.args[1])?;
        if left != right {
            return Err(CompileError::at(
                "join and race branches must share a type",
                call.span(),
            ));
        }
        let join_pc = Pc(self.instructions.len() as u32);
        if let Instruction::Fork { join_pc: slot, .. } = &mut self.instructions[fork] {
            *slot = join_pc;
        }
        self.emit(if all {
            Instruction::JoinAll
        } else {
            Instruction::JoinAny
        });
        if all {
            Ok(self.vec_ty(left))
        } else {
            Ok(left)
        }
    }

    pub(super) fn branch(&mut self, expr: &Expr) -> Result<Ty, CompileError> {
        let call = match expr {
            Expr::Call(call) => call,
            Expr::Await(await_expr) => match &*await_expr.base {
                Expr::Call(call) => call,
                _ => {
                    return Err(CompileError::at(
                        "join branches must be durable calls",
                        expr.span(),
                    ))
                }
            },
            _ => {
                return Err(CompileError::at(
                    "join branches must be durable calls",
                    expr.span(),
                ))
            }
        };
        let Expr::Path(path) = &*call.func else {
            return Err(CompileError::at(
                "join branches must be durable calls",
                expr.span(),
            ));
        };
        let Some(name) = self.resolve_durable(&path.path)? else {
            return Err(CompileError::at(
                "join branches must be durable calls",
                expr.span(),
            ));
        };
        if matches!(name.as_str(), "join" | "race") {
            return Err(CompileError::at(
                "nested join and race are outside this subset",
                expr.span(),
            ));
        }
        self.durable_call(call)
    }

    pub(super) fn type_discarded(&mut self, expr: &Expr) -> Result<Ty, CompileError> {
        let instructions = self.instructions.len();
        let next_local = self.next_local;
        let names = self.names.clone();
        let moved = self.moved.clone();
        let moved_fields = self.moved_fields.clone();
        let extras = self.extras.len();
        let next_func = self.next_func;
        let pending = self.pending_closure.clone();
        let ty = self.expr(expr);
        self.instructions.truncate(instructions);
        self.next_local = next_local;
        self.names = names;
        self.moved = moved;
        self.moved_fields = moved_fields;
        self.extras.truncate(extras);
        self.next_func = next_func;
        self.pending_closure = pending;
        ty
    }

    pub(super) fn for_range(
        &mut self,
        name: &str,
        range: &syn::ExprRange,
        body: &Block,
    ) -> Result<Ty, CompileError> {
        if !matches!(range.limits, syn::RangeLimits::HalfOpen(_)) {
            return Err(CompileError::at(
                "for only supports an exclusive range",
                range.span(),
            ));
        }
        let Some(start) = &range.start else {
            return Err(CompileError::at("for supports 0..n", range.span()));
        };
        let Some(end) = &range.end else {
            return Err(CompileError::at("for supports 0..n", range.span()));
        };
        let start_ty = self.expr(start)?;
        if start_ty != self.ty_f64 {
            return Err(CompileError::at("range bounds are f64", range.span()));
        }
        let index = self.bind(name.to_string(), self.ty_f64, true, range.span())?;
        self.store(index);
        let end_ty = self.expr(end)?;
        if end_ty != self.ty_f64 {
            return Err(CompileError::at("range bounds are f64", range.span()));
        }
        let end_local = self.fresh();
        self.store(end_local);
        let loop_start = self.instructions.len() as u32;
        self.load(index);
        self.load(end_local);
        self.emit(Instruction::Lt);
        let to_end = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
        let previous = self.loop_binding.replace(name.to_string());
        self.loops.push(LoopFrame {
            continue_pc: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.block(body, false)?;
        self.loop_binding = previous;
        let increment = self.instructions.len() as u32;
        self.patch_continues(increment);
        self.load(index);
        self.number(1.0);
        self.emit(Instruction::Add);
        self.store(index);
        self.emit(Instruction::Jump {
            target: Pc(loop_start),
        });
        self.patch(to_end);
        self.patch_breaks();
        self.loops.pop();
        self.undefined();
        Ok(self.ty_unit)
    }

    pub(super) fn for_vec(
        &mut self,
        name: &str,
        collection: &Expr,
        body: &Block,
    ) -> Result<Ty, CompileError> {
        let ty = match collection {
            Expr::Path(path) => {
                let (ty, moved_from) = self.load_local(path)?;
                if let Some((source, id)) = moved_from {
                    if self.has_moved_field(&source) {
                        return Err(CompileError::at(
                            format!("use of moved value `{source}`"),
                            collection.span(),
                        ));
                    }
                    self.clear(id);
                    self.moved.insert(source, ());
                }
                ty
            }
            other => self.expr(other)?,
        };
        let Some(elem) = self.vec_elem(ty) else {
            return Err(CompileError::at(
                "for supports Vec<T> or 0..n",
                collection.span(),
            ));
        };
        let vec = self.fresh();
        self.store(vec);
        let index = self.fresh();
        self.number(0.0);
        self.store(index);
        let end = self.fresh();
        self.load(vec);
        self.emit(Instruction::Length);
        self.store(end);
        let loop_start = self.instructions.len() as u32;
        self.load(index);
        self.load(end);
        self.emit(Instruction::Lt);
        let to_end = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
        self.load(vec);
        self.load(index);
        self.emit(Instruction::GetIndex);
        let binding = self.bind(name.to_string(), elem, false, collection.span())?;
        self.store(binding);
        self.moved.remove(name);
        if self.moves(elem) {
            self.load(vec);
            self.load(index);
            self.undefined();
            self.emit(Instruction::SetIndex);
        }
        self.loops.push(LoopFrame {
            continue_pc: None,
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.block(body, false)?;
        let increment = self.instructions.len() as u32;
        self.patch_continues(increment);
        self.load(index);
        self.number(1.0);
        self.emit(Instruction::Add);
        self.store(index);
        self.emit(Instruction::Jump {
            target: Pc(loop_start),
        });
        self.patch(to_end);
        self.patch_breaks();
        self.loops.pop();
        self.undefined();
        Ok(self.ty_unit)
    }

    pub(super) fn while_loop(&mut self, expr: &syn::ExprWhile) -> Result<Ty, CompileError> {
        if expr.label.is_some() {
            return Err(CompileError::at(
                "loop labels are outside this subset",
                expr.span(),
            ));
        }
        let start = self.instructions.len() as u32;
        let cond = self.expr(&expr.cond)?;
        if cond != self.ty_bool {
            return Err(CompileError::at("while requires a bool", expr.span()));
        }
        let to_end = self.emit(Instruction::JumpIfFalse { target: Pc(0) });
        self.loops.push(LoopFrame {
            continue_pc: Some(start),
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.block(&expr.body, false)?;
        self.emit(Instruction::Jump { target: Pc(start) });
        self.patch(to_end);
        self.patch_breaks();
        self.loops.pop();
        self.undefined();
        Ok(self.ty_unit)
    }

    pub(super) fn loop_expr(&mut self, expr: &syn::ExprLoop) -> Result<Ty, CompileError> {
        if expr.label.is_some() {
            return Err(CompileError::at(
                "loop labels are outside this subset",
                expr.span(),
            ));
        }
        let start = self.instructions.len() as u32;
        self.loops.push(LoopFrame {
            continue_pc: Some(start),
            breaks: Vec::new(),
            continues: Vec::new(),
        });
        self.block(&expr.body, false)?;
        self.emit(Instruction::Jump { target: Pc(start) });
        self.patch_breaks();
        self.loops.pop();
        self.undefined();
        Ok(self.ty_unit)
    }

    pub(super) fn break_expr(&mut self, expr: &syn::ExprBreak) -> Result<Ty, CompileError> {
        if expr.label.is_some() || expr.expr.is_some() {
            return Err(CompileError::at(
                "break has no label and no value",
                expr.span(),
            ));
        }
        if self.loops.is_empty() {
            return Err(CompileError::at("break is outside a loop", expr.span()));
        }
        let index = self.emit(Instruction::Jump { target: Pc(0) });
        self.loops.last_mut().unwrap().breaks.push(index);
        Ok(self.ty_never)
    }

    pub(super) fn continue_expr(&mut self, expr: &syn::ExprContinue) -> Result<Ty, CompileError> {
        if expr.label.is_some() {
            return Err(CompileError::at(
                "loop labels are outside this subset",
                expr.span(),
            ));
        }
        let continue_pc = self
            .loops
            .last()
            .ok_or_else(|| CompileError::at("continue is outside a loop", expr.span()))?
            .continue_pc;
        if let Some(pc) = continue_pc {
            self.emit(Instruction::Jump { target: Pc(pc) });
        } else {
            let index = self.emit(Instruction::Jump { target: Pc(0) });
            self.loops.last_mut().unwrap().continues.push(index);
        }
        Ok(self.ty_never)
    }

    pub(super) fn patch_breaks(&mut self) {
        let breaks = self
            .loops
            .last()
            .map(|frame| frame.breaks.clone())
            .unwrap_or_default();
        for index in breaks {
            self.patch(index);
        }
    }

    pub(super) fn patch_continues(&mut self, target: u32) {
        let continues = self
            .loops
            .last()
            .map(|frame| frame.continues.clone())
            .unwrap_or_default();
        for index in continues {
            if let Instruction::Jump { target: slot } = &mut self.instructions[index] {
                *slot = Pc(target);
            }
        }
    }

    pub(super) fn closure(&mut self, expr: &syn::ExprClosure) -> Result<Ty, CompileError> {
        if expr.asyncness.is_some()
            || expr.constness.is_some()
            || expr.lifetimes.is_some()
            || expr.movability.is_some()
        {
            return Err(CompileError::at(
                "this closure is outside this subset",
                expr.span(),
            ));
        }
        let mut params = Vec::new();
        let mut param_names = HashSet::new();
        for pat in &expr.inputs {
            let (name, ty) = self.closure_param(pat)?;
            param_names.insert(name.clone());
            params.push((name, ty));
        }
        let captures = self.find_captures(&expr.body, &param_names);
        if !captures.is_empty() && expr.capture.is_none() {
            return Err(CompileError::at(
                "the move keyword is required when a closure captures",
                expr.span(),
            ));
        }
        let annotated = match &expr.output {
            ReturnType::Type(_, ty) => Some(self.ty(ty)?),
            ReturnType::Default => None,
        };
        let id = self.next_func;
        self.next_func += 1;
        let body_ty = self.compile_detached(id, "closure", |compiler| {
            compiler.next_local = 1;
            compiler.param_count = params.len() as u32;
            for (index, (name, ty)) in captures.iter().enumerate() {
                compiler.captures.insert(name.clone(), (index as u32, *ty));
            }
            for (name, ty) in &params {
                compiler.bind(name.clone(), *ty, false, expr.span())?;
            }
            if let Some(ret) = annotated {
                compiler.ret = ret;
            }
            let body_ty = compiler.expr_move(&expr.body, true)?;
            if let Some(ret) = annotated {
                if body_ty != ret {
                    return Err(CompileError::at(
                        "closure body does not match its return type",
                        expr.span(),
                    ));
                }
            }
            compiler.emit(Instruction::Return);
            Ok(body_ty)
        })?;
        for (name, _) in &captures {
            self.capture_cell(name, expr.span())?;
        }
        self.emit(Instruction::NewEnv {
            count: captures.len() as u32,
        });
        self.emit(Instruction::NewClosure { func: FuncId(id) });
        self.pending_closure = Some(ClosureSig {
            params: params.into_iter().map(|(_, ty)| ty).collect(),
            ret: body_ty,
        });
        Ok(self.ty_closure)
    }

    pub(super) fn closure_param(&mut self, pat: &Pat) -> Result<(String, Ty), CompileError> {
        let Pat::Type(typed) = pat else {
            return Err(CompileError::at(
                "closure parameters need a type",
                pat.span(),
            ));
        };
        let Pat::Ident(ident) = &*typed.pat else {
            return Err(CompileError::at(
                "closure parameters are identifiers",
                pat.span(),
            ));
        };
        if ident.by_ref.is_some() {
            return Err(CompileError::at(
                "references are outside this subset",
                ident.span(),
            ));
        }
        Ok((ident.ident.to_string(), self.ty(&typed.ty)?))
    }

    pub(super) fn capture_cell(
        &mut self,
        name: &str,
        span: proc_macro2::Span,
    ) -> Result<(), CompileError> {
        if self.moved.contains_key(name) {
            return Err(CompileError::at(
                format!("use of moved value `{name}`"),
                span,
            ));
        }
        let Some(slot) = self.names.get(name).cloned() else {
            return Err(CompileError::at(format!("unknown name `{name}`"), span));
        };
        self.load(slot.id);
        if self.moves(slot.ty) {
            self.clear(slot.id);
            self.moved.insert(name.to_string(), ());
        }
        self.emit(Instruction::NewCell);
        Ok(())
    }

    pub(super) fn find_captures(&self, expr: &Expr, params: &HashSet<String>) -> Vec<(String, Ty)> {
        let mut walk = CaptureWalk {
            names: &self.names,
            params,
            seen: HashSet::new(),
            found: Vec::new(),
        };
        walk.visit_expr(expr);
        walk.found
    }
}
