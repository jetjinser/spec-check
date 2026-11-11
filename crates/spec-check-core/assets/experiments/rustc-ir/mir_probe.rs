#![feature(rustc_private)]
extern crate rustc_borrowck;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

use rustc_driver::{Callbacks, Compilation, run_compiler};
use rustc_middle::{
    mir::TerminatorKind,
    ty::{self, TyCtxt},
};

// Structural path rule from the pinned Verus rust_to_vir_base converter.
fn interface_path(tcx: TyCtxt<'_>, def: rustc_hir::def_id::DefId) -> Option<String> {
    use rustc_hir::definitions::DefPathData;
    let path = tcx.def_path(def);
    let mut parts = vec![tcx.crate_name(path.krate).to_string()];
    for d in &path.data {
        parts.push(match &d.data {
            DefPathData::ValueNs(s) | DefPathData::TypeNs(s) => s.to_string(),
            DefPathData::Impl => format!("impl&%{}", d.disambiguator),
            DefPathData::Ctor => "ctor%".into(),
            DefPathData::Closure => format!("closure%{}", d.disambiguator),
            DefPathData::OpaqueTy => format!("opaque{}", d.disambiguator),
            DefPathData::ForeignMod => continue,
            _ => return None,
        });
    }
    Some(parts.join("::"))
}

fn interface_type(tcx: TyCtxt<'_>, typ: ty::Ty<'_>) -> Option<String> {
    match typ.kind() {
        ty::Bool => Some("bool".into()),
        ty::Never => Some("!".into()),
        ty::Uint(kind) => Some(format!("{kind:?}").to_ascii_lowercase()),
        ty::Int(kind) => Some(format!("{kind:?}").to_ascii_lowercase()),
        ty::Ref(_, inner, mutable) => Some(format!(
            "{}{}",
            if mutable.is_mut() { "&mut " } else { "&" },
            interface_type(tcx, *inner)?
        )),
        ty::Slice(inner) => Some(format!("slice<{}>", interface_type(tcx, *inner)?)),
        ty::Str => Some("str".into()),
        ty::Tuple(items) => Some(format!(
            "tuple[{}]",
            items
                .iter()
                .map(|t| interface_type(tcx, t))
                .collect::<Option<Vec<_>>>()?
                .join(",")
        )),
        ty::Adt(def, args) => {
            let mut types = Vec::new();
            for arg in args.iter() {
                match arg.kind() {
                    ty::GenericArgKind::Type(t) => types.push(interface_type(tcx, t)?),
                    ty::GenericArgKind::Lifetime(_) => {}
                    _ => return None,
                }
            }
            Some(format!(
                "{}<{}>",
                interface_path(tcx, def.did())?,
                types.join(",")
            ))
        }
        ty::Param(p) => Some(format!("param:{}", p.name)),
        _ => None,
    }
}

fn scalar_prefix<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: rustc_hir::def_id::LocalDefId,
    body: &rustc_middle::mir::Body<'tcx>,
) {
    use rustc_middle::mir::{Operand, START_BLOCK, StatementKind};
    if std::env::var_os("CLIENT_PROJECT_PREFIX").is_none()
        || !matches!(
            tcx.def_kind(owner),
            rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
        )
    {
        return;
    }
    let annotated=tcx.hir_attrs(tcx.local_def_id_to_hir_id(owner)).iter().any(|a|matches!(a,rustc_hir::Attribute::Unparsed(item) if item.path.segments.first().is_some_and(|p|p.as_str()=="verus" || p.as_str()=="verifier")));
    if body
        .basic_blocks
        .iter()
        .any(|b| b.terminator().successors().any(|to| to == START_BLOCK))
    {
        return;
    }
    let first = &body.basic_blocks[START_BLOCK];
    if !tcx
        .hir_body_owned_by(owner)
        .value
        .span
        .source_callsite()
        .contains(first.terminator().source_info.span.source_callsite())
    {
        return;
    }

    if !first.statements.iter().all(|s| {
        matches!(
            s.kind,
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop
        )
    }) {
        return;
    }
    let TerminatorKind::Call { func, args, .. } = &first.terminator().kind else {
        return;
    };
    let ty::FnDef(def, generics) = func.ty(&body.local_decls, tcx).kind() else {
        return;
    };
    if !def.is_local() || !generics.is_empty() {
        return;
    }
    let Some(path) = interface_path(tcx, *def) else {
        return;
    };
    if !path
        .split("::")
        .all(|p| !p.is_empty() && p.chars().all(|c| c == '_' || c.is_alphanumeric()))
    {
        return;
    }
    let sig = tcx.fn_sig(*def).instantiate(tcx, generics).skip_binder();
    if !format!("{sig:?}").starts_with("fn(") || sig.inputs().len() != args.len() {
        return;
    }
    let primitive = |t: ty::Ty<'tcx>| matches!(t.kind(), ty::Bool | ty::Int(_) | ty::Uint(_));
    let ret = if primitive(sig.output()) {
        interface_type(tcx, sig.output()).unwrap()
    } else if matches!(sig.output().kind(),ty::Tuple(xs) if xs.is_empty()) {
        "()".into()
    } else {
        return;
    };
    let mut inputs = std::collections::BTreeMap::new();
    let mut operands = Vec::new();
    for arg in args {
        let place = match &arg.node {
            Operand::Copy(p) | Operand::Move(p) => p,
            _ => return,
        };
        let index = place.local.as_usize();
        let typ = body.local_decls[place.local].ty;
        if !place.projection.is_empty() || index == 0 || index > body.arg_count || !primitive(typ) {
            return;
        }
        inputs.insert(index, interface_type(tcx, typ).unwrap());
        operands.push(index.to_string());
    }
    let inputs = inputs
        .into_iter()
        .map(|(n, t)| format!("{n}:{t}"))
        .collect::<Vec<_>>()
        .join("|");
    let trait_entry = tcx
        .opt_associated_item(owner.to_def_id())
        .and_then(|a| a.trait_item_def_id())
        .map(|id| interface_path(tcx, id).unwrap_or("UNRESOLVED_TRAIT".into()))
        .unwrap_or("-".into());
    eprintln!(
        "CALLER_PREFIX\t{owner:?}\t{:?}\t{path}\t{inputs}\t{}\t{ret}\t{trait_entry}\t{annotated}",
        first.terminator().source_info.span,
        operands.join(",")
    );
}

// Bootstrap records for source whose explicit ghost ABI still needs connection.
// This mode deliberately stops before type checking and carries no MIR claims.
fn project_source(tcx: TyCtxt<'_>) {
    use rustc_hir::{
        intravisit::{self, Visitor},
        ExprKind, StmtKind,
    };
    struct Source<'tcx> {
        tcx: TyCtxt<'tcx>,
    }
    impl<'hir> Visitor<'hir> for Source<'_> {
        fn visit_stmt(&mut self, stmt: &'hir rustc_hir::Stmt<'hir>) {
            if let StmtKind::Let(local) = stmt.kind {
                eprintln!(
                    "SOURCE_BINDING\t{:?}\t{:?}\t{:?}\t{:?}",
                    stmt.span,
                    local.pat.span,
                    local.ty.map(|t| t.span),
                    local.init.map(|e| e.span)
                );
                if let rustc_hir::PatKind::Tuple(parts, _) = local.pat.kind {
                    for (index, part) in parts.iter().enumerate() {
                        let kind = match part.kind {
                            rustc_hir::PatKind::Wild => "wildcard",
                            rustc_hir::PatKind::Binding(..) => "binding",
                            _ => "unsupported",
                        };
                        eprintln!(
                            "SOURCE_COMPONENT\t{:?}\t{index}\t{kind}\t{:?}",
                            local.pat.span, part.span
                        );
                    }
                }
            }
            intravisit::walk_stmt(self, stmt);
        }
        fn visit_expr(&mut self, expr: &'hir rustc_hir::Expr<'hir>) {
            if !expr.span.from_expansion() {
                match expr.kind {
                    ExprKind::Call(callee, args) => {
                        if let ExprKind::Path(rustc_hir::QPath::Resolved(Some(ty), path)) =
                            callee.kind
                        {
                            eprintln!("SOURCE_SELF\t{:?}\t{:?}", expr.span, ty.span);
                            let traits = path
                                .segments
                                .iter()
                                .filter_map(|segment| match segment.res {
                                    rustc_hir::def::Res::Def(
                                        rustc_hir::def::DefKind::Trait,
                                        def,
                                    ) => interface_path(self.tcx, def),
                                    _ => None,
                                })
                                .collect::<Vec<_>>();
                            let trait_path = if let [path] = traits.as_slice() {
                                path.as_str()
                            } else {
                                "UNRESOLVED_TRAIT"
                            };
                            if let Some(method) = path.segments.last() {
                                eprintln!(
                                    "SOURCE_QUALIFIED\t{:?}\t{trait_path}\t{}",
                                    expr.span, method.ident.name
                                );
                            }
                        }
                        let target = if let ExprKind::Path(rustc_hir::QPath::Resolved(_, path)) =
                            callee.kind
                        {
                            match path.res {
                                rustc_hir::def::Res::Def(kind, def) => {
                                    eprintln!(
                                        "SOURCE_TARGET\t{:?}\t{kind:?}\t{def:?}\t{:?}",
                                        expr.span,
                                        self.tcx.def_span(def)
                                    );
                                    interface_path(self.tcx, def)
                                        .unwrap_or("UNSUPPORTED_PATH".into())
                                }
                                _ => "UNRESOLVED_CALL".into(),
                            }
                        } else {
                            "UNRESOLVED_ASSOCIATED_CALL".into()
                        };
                        eprintln!(
                            "SOURCE_CALL\t{:?}\t{:?}\t{target}\t{}",
                            expr.span,
                            callee.span,
                            args.len()
                        );
                        for (i, arg) in args.iter().enumerate() {
                            eprintln!("SOURCE_ARGUMENT\t{:?}\t{i}\t{:?}", expr.span, arg.span);
                        }
                    }
                    ExprKind::MethodCall(segment, receiver, args, _) => {
                        eprintln!(
                            "SOURCE_METHOD\t{:?}\t{}\t{:?}\t{}",
                            expr.span,
                            segment.ident.name,
                            receiver.span,
                            args.len()
                        );
                        for (i, arg) in args.iter().enumerate() {
                            eprintln!("SOURCE_ARGUMENT\t{:?}\t{i}\t{:?}", expr.span, arg.span);
                        }
                    }
                    _ => {}
                }
            }
            intravisit::walk_expr(self, expr);
        }
    }
    let mut selected = 0;
    for owner in tcx.hir_body_owners() {
        let path = tcx.def_path_str(owner.to_def_id());
        if selected_project_borrow(&path)
            || (std::env::var_os("CLIENT_PROJECT_BORROW_PATHS").is_none()
                && matches!(tcx.def_kind(owner), rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn))
        {
            selected += 1;
            eprintln!("SOURCE_OWNER\t{path}\t{:?}", tcx.def_span(owner));
            eprintln!("SOURCE_BODY\t{path}\t{:?}", tcx.hir_body_owned_by(owner).value.span.source_callsite());
            Source { tcx }.visit_expr(tcx.hir_body_owned_by(owner).value);
            eprintln!("SOURCE_END\t{path}");
        }
    }
    if selected == 0 {
        tcx.sess
            .dcx()
            .err("source-only extraction selected no HIR bodies");
    }
}

fn project_inventory(tcx: TyCtxt<'_>) {
    use rustc_hir::intravisit::Visitor;
    struct Expressions<'tcx> {
        tcx: TyCtxt<'tcx>,
        types: &'tcx ty::TypeckResults<'tcx>,
        owner: String,
        assertions: std::collections::BTreeSet<String>,
    }
    impl<'hir> Visitor<'hir> for Expressions<'_> {
        fn visit_expr(&mut self, expr: &'hir rustc_hir::Expr<'hir>) {
            use rustc_hir::ExprKind;
            let span = expr.span.source_callsite();
            let call = match expr.kind {
                ExprKind::Call(callee, _) => Some((
                    "call",
                    match self.types.expr_ty(callee).kind() {
                        ty::FnDef(def, _) => self.tcx.def_path_str(*def),
                        _ => "UNRESOLVED_INDIRECT".to_owned(),
                    },
                )),
                ExprKind::MethodCall(..) => Some((
                    "method",
                    self.types
                        .type_dependent_def_id(expr.hir_id)
                        .map(|def| self.tcx.def_path_str(def))
                        .unwrap_or("UNRESOLVED_METHOD".into()),
                )),
                ExprKind::Binary(..)
                | ExprKind::Unary(..)
                | ExprKind::AssignOp(..)
                | ExprKind::Index(..) => self
                    .types
                    .type_dependent_def_id(expr.hir_id)
                    .map(|def| ("overloaded_operation", self.tcx.def_path_str(def))),
                _ => None,
            };
            if let Some((kind, target)) = call {
                eprintln!(
                    "PROJECT_CALL\t{}\t{:?}\t{span:?}\t{kind}\t{target}",
                    self.owner, expr.hir_id
                );
            }
            if matches!(
                expr.kind,
                ExprKind::If(..) | ExprKind::Match(..) | ExprKind::Loop(..)
            ) {
                eprintln!(
                    "PROJECT_CONTROL\t{}\t{:?}\t{span:?}",
                    self.owner,
                    std::mem::discriminant(&expr.kind)
                );
            }
            if let Some(def) = expr.span.ctxt().outer_expn_data().macro_def_id {
                if self.tcx.crate_name(def.krate).as_str() == "core"
                    && [
                        "assert",
                        "assert_eq",
                        "assert_ne",
                        "debug_assert",
                        "debug_assert_eq",
                        "debug_assert_ne",
                    ]
                    .contains(&self.tcx.item_name(def).as_str())
                {
                    let target = self.tcx.def_path_str(def);
                    let key = format!("{span:?}:{target}");
                    if self.assertions.insert(key) {
                        eprintln!("PROJECT_ASSERT\t{}\t{span:?}\t{target}", self.owner);
                    }
                }
            }
            rustc_hir::intravisit::walk_expr(self, expr);
        }
    }
    for file in tcx.sess.source_map().files().iter() {
        eprintln!("PROJECT_FILE\t{:?}", file.name);
    }
    for owner in tcx.hir_body_owners() {
        let path = tcx.def_path_str(owner.to_def_id());
        let span = tcx.hir_span(tcx.local_def_id_to_hir_id(owner));
        let kind = tcx.def_kind(owner);
        let identity = format!("{owner:?}");
        eprintln!(
            "PROJECT_OWNER\t{identity}\t{path}\t{kind:?}\t{span:?}\t{:?}",
            tcx.hir_attrs(tcx.local_def_id_to_hir_id(owner))
        );
        if std::env::var_os("CLIENT_PROJECT_LINK").is_some() {
            if let Some(native_path) = interface_path(tcx, owner.to_def_id()) {
                eprintln!("LINK_OWNER\t{identity}\t{native_path}\t{span:?}");
                let mut module = owner.to_def_id();
                while tcx.def_kind(module) != rustc_hir::def::DefKind::Mod { module = tcx.parent(module); }
                if let Some(module) = interface_path(tcx, module) {
                    eprintln!("LINK_MODULE\t{identity}\t{module}");
                }
            }
        }
        for attr in tcx.hir_attrs(tcx.local_def_id_to_hir_id(owner)) {
            if let rustc_hir::Attribute::Parsed(rustc_hir::attrs::AttributeKind::RustcTestMarker(
                name,
            )) = attr
            {
                eprintln!("PROJECT_TEST\t{identity}\t{name}\t{span:?}");
            }
        }
        if matches!(
            kind,
            rustc_hir::def::DefKind::Fn
                | rustc_hir::def::DefKind::AssocFn
                | rustc_hir::def::DefKind::Closure
        ) {
            let root = tcx.typeck_root_def_id(owner.to_def_id()).expect_local();
            Expressions {
                tcx,
                types: tcx.typeck(root),
                owner: identity.clone(),
                assertions: Default::default(),
            }
            .visit_expr(tcx.hir_body_owned_by(owner).value);
            if tcx.is_mir_available(owner.to_def_id()) {
                let body = tcx.optimized_mir(owner.to_def_id());
                scalar_prefix(tcx, owner, body);
                eprintln!("PROJECT_MIR_BODY\t{identity}\t{}", body.basic_blocks.len());
                for (local, decl) in body.local_decls.iter_enumerated() {
                    let paths = decl.ty.walk().into_iter().filter_map(|arg| arg.as_type())
                        .filter_map(|t| if let ty::Adt(def, _) = t.kind() { interface_path(tcx, def.did()) } else { None })
                        .collect::<std::collections::BTreeSet<_>>();
                    eprintln!("PROJECT_MIR_LOCAL\t{identity}\t{local:?}\t{:?}\t{}\t{}",
                        decl.ty, interface_type(tcx, decl.ty).unwrap_or("UNSUPPORTED_TYPE".into()),
                        paths.into_iter().collect::<Vec<_>>().join("|"));
                }
                for (block, data) in body.basic_blocks.iter_enumerated() {
                    for (index, statement) in data.statements.iter().enumerate() {
                        eprintln!(
                            "PROJECT_MIR_STMT\t{identity}\t{block:?}\t{index}\t{:?}\t{:?}",
                            statement.source_info.span,
                            statement.kind
                        );
                    }
                    let term = data.terminator();
                    let span = term.source_info.span;
                    eprintln!(
                        "PROJECT_MIR_TERM\t{identity}\t{block:?}\t{span:?}\t{}\t{:?}",
                        data.statements.len(),
                        term.kind
                    );
                    match &term.kind {
                        TerminatorKind::Call {
                            func,
                            args,
                            destination,
                            ..
                        } => {
                            let (target, generics, target_id, target_span) =
                                match func.ty(&body.local_decls, tcx).kind() {
                                    ty::FnDef(def, generics) => (
                                        tcx.def_path_str(*def),
                                        format!("{generics:?}"),
                                        format!("{def:?}"),
                                        format!("{:?}", tcx.def_span(*def)),
                                    ),
                                    ty => (
                                        "UNRESOLVED_INDIRECT".to_owned(),
                                        format!("{ty:?}"),
                                        "unresolved".into(),
                                        "no-location".into(),
                                    ),
                                };
                            if std::env::var_os("CLIENT_PROJECT_LINK").is_some() {
                                if let ty::FnDef(def, params) =
                                    func.ty(&body.local_decls, tcx).kind()
                                {
                                    let declared = interface_path(tcx, *def)
                                        .unwrap_or("UNSUPPORTED_PATH".into());
                                    let env = ty::TypingEnv::post_analysis(tcx, owner.to_def_id());
                                    match ty::Instance::try_resolve(tcx, env, *def, params) {
                                        Ok(Some(instance)) => {
                                            let id = instance.def_id();
                                            let resolved = interface_path(tcx, id)
                                                .unwrap_or("UNSUPPORTED_PATH".into());
                                            let signature = if matches!(
                                                tcx.def_kind(id),
                                                rustc_hir::def::DefKind::Fn
                                                    | rustc_hir::def::DefKind::AssocFn
                                            ) {
                                                format!(
                                                    "{:?}",
                                                    tcx.fn_sig(id)
                                                        .instantiate(tcx, instance.args)
                                                        .skip_binder()
                                                )
                                            } else {
                                                "compiler shim or constructor".into()
                                            };
                                            if matches!(
                                                tcx.def_kind(id),
                                                rustc_hir::def::DefKind::Fn
                                                    | rustc_hir::def::DefKind::AssocFn
                                            ) {
                                                let sig = tcx
                                                    .fn_sig(id)
                                                    .instantiate(tcx, instance.args)
                                                    .skip_binder();
                                                match (
                                                    sig.inputs()
                                                        .iter()
                                                        .map(|t| interface_type(tcx, *t))
                                                        .collect::<Option<Vec<_>>>(),
                                                    interface_type(tcx, sig.output()),
                                                ) {
                                                    (Some(inputs), Some(output)) => eprintln!(
                                                        "LINK_TYPES\t{identity}\t{block:?}\t{}\t{output}",
                                                        inputs.join("|")
                                                    ),
                                                    _ => eprintln!(
                                                        "LINK_TYPES_UNSUPPORTED\t{identity}\t{block:?}\tunsupported signature type"
                                                    ),
                                                }
                                            }
                                            eprintln!(
                                                "LINK_CALL\t{identity}\t{block:?}\t{def:?}\t{declared}\t{id:?}\t{resolved}\t{:?}\t{:?}\t{signature}\t{}",
                                                tcx.def_span(id),
                                                instance.def,
                                                std::iter::once(format!(
                                                    "{:?}",
                                                    tcx.def_span(id).source_callsite()
                                                ))
                                                .chain(
                                                    tcx.def_span(id)
                                                        .macro_backtrace()
                                                        .map(|e| format!("{:?}", e.call_site))
                                                )
                                                .collect::<Vec<_>>()
                                                .join("|")
                                            );
                                        }
                                        other => eprintln!(
                                            "LINK_UNRESOLVED\t{identity}\t{block:?}\t{def:?}\t{declared}\t{other:?}"
                                        ),
                                    }
                                } else {
                                    eprintln!(
                                        "LINK_UNRESOLVED\t{identity}\t{block:?}\tunresolved\tunresolved\tindirect callee"
                                    );
                                }
                            }
                            eprintln!(
                                "PROJECT_MIR_CALL\t{identity}\t{block:?}\t{span:?}\t{target}\t{target_id}\t{target_span}\t{generics}\t{args:?}\t{destination:?}"
                            );
                        }
                        TerminatorKind::Drop { place, .. } => {
                            let ty = place.ty(&body.local_decls, tcx).ty;
                            eprintln!(
                                "PROJECT_MIR_DROP\t{identity}\t{block:?}\t{span:?}\t{place:?}\t{ty:?}"
                            );
                        }
                        TerminatorKind::InlineAsm { .. }
                        | TerminatorKind::TailCall { .. }
                        | TerminatorKind::CoroutineDrop => {
                            eprintln!(
                                "PROJECT_MIR_UNSUPPORTED\t{identity}\t{block:?}\t{span:?}\topaque assembly, tail-call or coroutine destruction; full terminator retained"
                            );
                        }
                        _ => {}
                    }
                }
            } else {
                eprintln!("PROJECT_MIR_UNAVAILABLE\t{identity}\tcompiler reports MIR unavailable");
            }
        } else {
            eprintln!(
                "PROJECT_EXCLUDED\t{identity}\towner kind {kind:?} retained, expressions not visited"
            );
        }
    }
    eprintln!("PROJECT_COMPLETE\t{}", tcx.hir_body_owners().count());
}

struct Probe;
fn equality_body<'tcx>(tcx: TyCtxt<'tcx>, instance: ty::Instance<'tcx>, depth: usize) {
    let name = tcx.def_path_str(instance.def_id());
    if depth > 3
        || ![
            "<std::option::Option<T> as std::cmp::PartialEq>::eq",
            "<std::ops::Range<Idx> as std::cmp::PartialEq>::eq",
            "std::cmp::impls::<impl std::cmp::PartialEq for usize>::eq",
        ]
        .contains(&name.as_str())
    {
        return;
    }
    let env = ty::TypingEnv::fully_monomorphized();
    let body = tcx.instance_mir(instance.def);
    let locals = body
        .local_decls
        .iter()
        .map(|local| local.ty)
        .collect::<Vec<_>>();
    let blocks = body.basic_blocks.iter().collect::<Vec<_>>();
    eprintln!(
        "CLIENT_EQ_BODY\t{name}\t{:?}\t{:?}\tlocals={locals:?}; blocks={blocks:?}",
        tcx.def_span(instance.def_id()),
        instance.args
    );
    for block in body.basic_blocks.iter() {
        if let TerminatorKind::Call { func, .. } = &block.terminator().kind {
            if let ty::FnDef(def, args) = func.ty(&body.local_decls, tcx).kind() {
                let args = instance.instantiate_mir_and_normalize_erasing_regions(
                    tcx,
                    env,
                    ty::EarlyBinder::bind(tcx, *args),
                );
                if let Ok(Some(next)) = ty::Instance::try_resolve(tcx, env, *def, args) {
                    equality_body(tcx, next, depth + 1);
                }
            }
        }
    }
}
fn local_datatypes<'tcx>(
    tcx: TyCtxt<'tcx>,
    typ: ty::Ty<'tcx>,
    env: ty::TypingEnv<'tcx>,
    seen: &mut std::collections::HashSet<rustc_hir::def_id::DefId>,
) {
    match typ.kind() {
        ty::Ref(_, inner, _) | ty::RawPtr(inner, _) | ty::Slice(inner) | ty::Array(inner, _) => {
            local_datatypes(tcx, *inner, env, seen);
        }
        ty::Tuple(items) => {
            for item in items.iter() {
                local_datatypes(tcx, item, env, seen);
            }
        }
        ty::Adt(def, args) => {
            for arg in args.types() {
                local_datatypes(tcx, arg, env, seen);
            }
            if let Some(local) = def.did().as_local() {
                if !seen.insert(def.did()) {
                    return;
                }
                let hir = tcx.local_def_id_to_hir_id(local);
                let verified = tcx.hir_attrs(hir).iter().any(|a| {
                    matches!(a, rustc_hir::Attribute::Unparsed(item)
                        if item.path.segments.first().is_some_and(|p|
                            p.as_str() == "verus" || p.as_str() == "verifier"))
                });
                let span = tcx.hir_span(hir).source_callsite();
                eprintln!(
                    "CLIENT_DATATYPE\t{}\t{span:?}\t{verified}",
                    tcx.def_path_str(def.did())
                );
                for field in def.all_fields() {
                    let instantiated = field.ty(tcx, args);
                    eprintln!(
                        "CLIENT_DATATYPE_FIELD\t{}\t{}\t{instantiated:?}\t{env:?}",
                        tcx.def_path_str(def.did()),
                        field.name
                    );
                    match tcx.try_normalize_erasing_regions(env, instantiated) {
                        Ok(normalized) => local_datatypes(tcx, normalized, env, seen),
                        Err(_) => {
                            let _ = tcx.dcx().err(format!(
                                "client field type normalization unsupported: {instantiated:?}"
                            ));
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract(tcx: TyCtxt<'_>, owner: rustc_hir::def_id::LocalDefId) {
    let root = owner;
    let owner = match std::env::var("CLIENT_NESTED_FUNCTION") {
        Ok(selector) if !selector.is_empty() => tcx
            .hir_body_owners()
            .find(|&candidate| {
                tcx.typeck_root_def_id(candidate.to_def_id()) == root.to_def_id()
                    && tcx.def_path_str(candidate.to_def_id()) == selector
            })
            .expect("selected nested body in the requested function"),
        _ => owner,
    };
    let mut bodies = rustc_borrowck::consumers::get_bodies_with_borrowck_facts(
        tcx,
        root,
        rustc_borrowck::consumers::ConsumerOptions::PoloniusInputFacts,
    );
    let grouped = std::env::var_os("CLIENT_PROJECT_BORROW_PATHS").is_some();
    let mut selected = if grouped {
        bodies.keys().copied().collect::<Vec<_>>()
    } else {
        vec![owner]
    };
    selected.sort_by_key(|id| tcx.def_path_str(id.to_def_id()));
    for owner in selected {
        let path = tcx.def_path_str(owner.to_def_id());
        if grouped {
            eprintln!("CALLER_BEGIN\t{path}");
        }

        let expected_panic = tcx
            .hir_attrs(tcx.local_def_id_to_hir_id(owner))
            .iter()
            .any(|attr| {
                matches!(
                    attr,
                    rustc_hir::Attribute::Parsed(
                        rustc_hir::attrs::AttributeKind::ShouldPanic { .. }
                    )
                )
            });
        eprintln!("CLIENT_TEST_KIND\t{expected_panic}");
        let span = tcx.hir_span(tcx.local_def_id_to_hir_id(owner));
        let snippet = tcx.sess.source_map().span_to_snippet(span).ok();
        let body_span = tcx.hir_body_owned_by(owner).value.span.source_callsite();
        let body_snippet = tcx.sess.source_map().span_to_snippet(body_span).ok();
        eprintln!("CLIENT_BODY\t{body_span:?}\t{body_snippet:?}");
        eprintln!("CLIENT_DECL\t{span:?}\t{snippet:?}");
        if tcx.def_kind(owner) == rustc_hir::def::DefKind::Closure {
            if let ty::Closure(_, args) = tcx.type_of(owner).instantiate_identity().skip_norm_wip().kind() {
                eprintln!("CLIENT_CLOSURE_KIND\t{:?}", args.as_closure().kind());
            }
            for (index, capture) in tcx.closure_captures(owner).iter().enumerate() {
                let span = capture.get_path_span(tcx).source_callsite();
                let binding_id = match capture.place.base {
                    rustc_middle::hir::place::PlaceBase::Upvar(upvar) => format!("{:?}", upvar.var_path.hir_id),
                    _ => "unavailable".to_owned(),
                };
                eprintln!("CLIENT_CAPTURE\t{index}\t{}\t{binding_id}\t{:?}\t{:?}\t{:?}\t{}\t{span:?}",
                    capture.var_ident, capture.info.capture_kind, capture.place.ty(), capture.place,
                    capture.place.projections.len());
            }
        }
        let parent = tcx.parent(owner.to_def_id());
        if matches!(tcx.def_kind(parent), rustc_hir::def::DefKind::Impl { .. } | rustc_hir::def::DefKind::Trait) {
            let is_trait = tcx.def_kind(parent) == rustc_hir::def::DefKind::Trait;
            let container = if is_trait {
                tcx.hir_expect_item(parent.expect_local()).span.source_callsite()
            } else { tcx.def_span(parent).source_callsite() };
            let source = tcx.sess.source_map().span_to_snippet(container).ok();
            let kind = if is_trait { "trait" } else if matches!(tcx.def_kind(parent), rustc_hir::def::DefKind::Impl { of_trait: true }) { "impl" } else { "inherent_impl" };
            eprintln!("CLIENT_CONTAINER\t{kind}\t{container:?}\t{source:?}");
        }
        struct SourceExpressions<'tcx> {
            tcx: TyCtxt<'tcx>,
            typeck: &'tcx ty::TypeckResults<'tcx>,
            typing_env: ty::TypingEnv<'tcx>,
            captured: Vec<rustc_hir::HirId>,
        }
        struct AssertDomain<'tcx> {
            tcx: TyCtxt<'tcx>,
            typeck: &'tcx ty::TypeckResults<'tcx>,
            typing_env: ty::TypingEnv<'tcx>,
            drop_free: bool,
            local_control: bool,
        }
        impl<'hir> rustc_hir::intravisit::Visitor<'hir> for AssertDomain<'_> {
            fn visit_expr(&mut self, expr: &'hir rustc_hir::Expr<'hir>) {
                use rustc_hir::{BlockCheckMode, ExprKind};
                let supported = match expr.kind {
                    ExprKind::Ret(..)
                    | ExprKind::Break(..)
                    | ExprKind::Continue(..)
                    | ExprKind::Yield(..)
                    | ExprKind::Become(..)
                    | ExprKind::Closure(..)
                    | ExprKind::InlineAsm(..) => false,
                    ExprKind::Match(_, _, rustc_hir::MatchSource::TryDesugar(_)) => false,
                    ExprKind::Block(block, _) => {
                        !matches!(block.rules, BlockCheckMode::UnsafeBlock(_))
                    }
                    _ => true,
                };
                let ty = self.typeck.expr_ty(expr);
                let drop_free = !ty.needs_drop(self.tcx, self.typing_env);
                self.drop_free &= drop_free;
                self.local_control &= supported;
                rustc_hir::intravisit::walk_expr(self, expr);
            }
        }
        impl<'hir> rustc_hir::intravisit::Visitor<'hir> for SourceExpressions<'_> {
            fn visit_expr(&mut self, expr: &'hir rustc_hir::Expr<'hir>) {
                use rustc_hir::ExprKind;
                if let ExprKind::Path(qpath) = expr.kind {
                    if let rustc_hir::def::Res::Local(binding) = self.typeck.qpath_res(&qpath, expr.hir_id) {
                        if self.captured.contains(&binding) {
                            let span = expr.span.source_callsite();
                            eprintln!("CLIENT_CAPTURE_USE\t{binding:?}\t{span:?}");
                        }
                    }
                }
                let kind = match expr.kind {
                    ExprKind::Call(..) => Some("call"),
                    ExprKind::MethodCall(..) => Some("method_call"),
                    ExprKind::Index(..) => Some("index"),
                    ExprKind::If(..) => Some("if"),
                    ExprKind::Match(..) => Some("match"),
                    ExprKind::Loop(..) => Some("loop"),
                    _ => None,
                };
                if let Some(kind) = kind {
                    let span = expr.span.source_callsite();
                    let snippet = self.tcx.sess.source_map().span_to_snippet(span).ok();
                    eprintln!("CLIENT_SOURCE_EXPR\t{kind}\t{span:?}\t{snippet:?}");
                }
                let arguments = match expr.kind {
                    ExprKind::Call(_, args) => args.iter().collect::<Vec<_>>(),
                    ExprKind::MethodCall(_, receiver, args, _) => {
                        std::iter::once(receiver).chain(args.iter()).collect()
                    }
                    ExprKind::Index(receiver, index, _) => vec![receiver, index],
                    _ => Vec::new(),
                };
                for (index, arg) in arguments.into_iter().enumerate() {
                    let call_span = expr.span.source_callsite();
                    let span = arg.span.source_callsite();
                    let snippet = self.tcx.sess.source_map().span_to_snippet(span).ok();
                    let ty = self.typeck.expr_ty(arg);
                    eprintln!(
                        "CLIENT_SOURCE_ARGUMENT\t{call_span:?}\t{index}\t{span:?}\t{ty:?}\t{snippet:?}"
                    );
                }
                if let ExprKind::MethodCall(..) = expr.kind {
                    if let Some(def) = self.typeck.type_dependent_def_id(expr.hir_id) {
                        let span = expr.span.source_callsite();
                        eprintln!("CLIENT_SOURCE_METHOD\t{span:?}\t{}\t{def:?}", self.tcx.def_path_str(def));
                    }
                }
                if let ExprKind::Index(..) = expr.kind {
                    if let Some(def) = self.typeck.type_dependent_def_id(expr.hir_id) {
                        let span = expr.span.source_callsite();
                        eprintln!("CLIENT_SOURCE_INDEX\t{span:?}\t{}\t{def:?}", self.tcx.def_path_str(def));
                    }
                }
                if let ExprKind::Closure(closure) = expr.kind {
                    let body = self.tcx.hir_body(closure.body);
                    if let [parameter] = body.params {
                        if let rustc_hir::PatKind::Binding(mode, binding, _, None) = parameter.pat.kind {
                            if let ExprKind::Path(rustc_hir::QPath::Resolved(None, path)) = body.value.kind {
                                if path.res == rustc_hir::def::Res::Local(binding) {
                                    let span = expr.span.source_callsite();
                                    eprintln!("CLIENT_IDENTITY_CLOSURE\t{span:?}\t{mode:?}\t{:?}", self.typeck.expr_ty(expr));
                                }
                            }
                        }
                    }
                }
                if let ExprKind::Match(scrutinee, ..) = expr.kind {
                    if let Some(def) = expr.span.ctxt().outer_expn_data().macro_def_id {
                        let target = self.tcx.def_path_str(def);
                        if self.tcx.crate_name(def.krate).as_str() == "core"
                            && self.tcx.item_name(def).as_str() == "assert_eq"
                        {
                            if let ExprKind::Tup(operands) = scrutinee.kind {
                                for (index, operand) in operands.iter().enumerate() {
                                    if let ExprKind::AddrOf(_, _, value) = operand.kind {
                                        let assertion = expr.span.source_callsite();
                                        let span = value.span.source_callsite();
                                        let snippet =
                                            self.tcx.sess.source_map().span_to_snippet(span).ok();
                                        let operand_ty = self.typeck.expr_ty(value);
                                        eprintln!(
                                            "CLIENT_ASSERT_OPERAND\t{target}\t{assertion:?}\t{index}\t{span:?}\t{operand_ty:?}\t{snippet:?}"
                                        );
                                        let mut domain = AssertDomain {
                                            tcx: self.tcx,
                                            typeck: self.typeck,
                                            typing_env: self.typing_env,
                                            drop_free: true,
                                            local_control: operand_ty == self.tcx.types.usize,
                                        };
                                        domain.visit_expr(value);
                                        eprintln!(
                                            "CLIENT_ASSERT_DOMAIN\t{assertion:?}\t{index}\t{}\t{}\t{}\t{}",
                                            domain.drop_free,
                                            domain.local_control,
                                            !expr
                                                .span
                                                .ctxt()
                                                .outer_expn_data()
                                                .call_site
                                                .from_expansion(),
                                            self.tcx.sess.target.pointer_width
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                if let ExprKind::If(condition, ..) = expr.kind {
                    if let Some(def) = expr.span.ctxt().outer_expn_data().macro_def_id {
                        if self.tcx.crate_name(def.krate).as_str() == "core"
                            && self.tcx.item_name(def).as_str() == "assert"
                        {
                            if let ExprKind::Unary(rustc_hir::UnOp::Not, value) = condition.kind {
                                let target = self.tcx.def_path_str(def);
                                let assertion = expr.span.source_callsite();
                                let span = value.span.source_callsite();
                                let snippet = self.tcx.sess.source_map().span_to_snippet(span).ok();
                                let ty = self.typeck.expr_ty(value);
                                eprintln!(
                                    "CLIENT_ASSERT_OPERAND\t{target}\t{assertion:?}\t0\t{span:?}\t{ty:?}\t{snippet:?}"
                                );
                            }
                        }
                    }
                }
                rustc_hir::intravisit::walk_expr(self, expr);
            }
        }
        use rustc_hir::intravisit::Visitor;
        SourceExpressions {
            tcx,
            typeck: tcx.typeck(root),
            typing_env: ty::TypingEnv::post_analysis(tcx, root.to_def_id()),
            captured: if tcx.def_kind(owner) == rustc_hir::def::DefKind::Closure {
                tcx.closure_captures(owner).iter().filter_map(|c| match c.place.base {
                    rustc_middle::hir::place::PlaceBase::Upvar(upvar) => Some(upvar.var_path.hir_id),
                    _ => None,
                }).collect()
            } else { Vec::new() },
        }
        .visit_expr(tcx.hir_body_owned_by(owner).value);
        // Use the same MIR body and region identifiers as the borrowck facts.
        let facts = bodies.remove(&owner).expect("selected borrowck body");
        let body = &facts.body;
        for info in &body.var_debug_info {
            if let rustc_middle::mir::VarDebugInfoContents::Place(place) = &info.value {
                if place.projection.is_empty() {
                    let span = info.source_info.span.source_callsite();
                    let ty = body.local_decls[place.local].ty;
                    eprintln!("CLIENT_BINDING\t{}\t{place:?}\t{span:?}\t{ty:?}", info.name);
                }
            }
        }
        for (location, loan) in facts.borrow_set.location_map() {
            eprintln!(
                "CLIENT_LOAN\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}\t{:?}",
                location,
                loan.kind(),
                loan.borrowed_place(),
                loan.assigned_place(),
                loan.region(),
                loan.activation_location()
            );
        }
        let out_of_scope = rustc_borrowck::consumers::calculate_borrows_out_of_scope_at_location(
            body,
            &facts.region_inference_context,
            &facts.borrow_set,
        );
        let mut exits: Vec<_> = out_of_scope.iter().collect();
        exits.sort_by_key(|(location, _)| (location.block.as_usize(), location.statement_index));
        for (location, loans) in exits {
            eprintln!("CLIENT_LOAN_OUT_OF_SCOPE\t{location:?}\t{loans:?}");
        }
        if let (Some(input), Some(locations)) = (&facts.input_facts, &facts.location_table) {
            for (region, loan, point) in &input.loan_issued_at {
                eprintln!(
                    "CLIENT_LOAN_ISSUED\t{loan:?}\t{region:?}\t{:?}",
                    locations.to_rich_location(*point)
                );
            }
        }
        eprintln!("CLIENT_FUNCTION\t{path}");
        let mut datatypes = std::collections::HashSet::new();
        if tcx.def_kind(parent) == rustc_hir::def::DefKind::Trait {
            for item in tcx.associated_items(parent).in_definition_order() {
                if tcx.def_kind(item.def_id) == rustc_hir::def::DefKind::AssocFn {
                    let signature = tcx.fn_sig(item.def_id).instantiate_identity().skip_binder();
                    let env = ty::TypingEnv::post_analysis(tcx, item.def_id);
                    eprintln!("CLIENT_TRAIT_SIGNATURE\t{}\t{signature:?}\t{env:?}", tcx.def_path_str(item.def_id));
                    let mut seen = std::collections::HashSet::new();
                    for typ in signature.inputs_and_output.iter() { local_datatypes(tcx, typ, env, &mut seen); }
                }
            }
        }
        eprintln!("CLIENT_PARAMETERS\t{}", body.arg_count);
        for (index, local) in body.args_iter().enumerate() {
            eprintln!("CLIENT_PARAMETER\t{index}\t{local:?}\t{:?}", body.local_decls[local].ty);
        }
        for (local, decl) in body.local_decls.iter_enumerated() {
            local_datatypes(tcx, decl.ty, ty::TypingEnv::post_analysis(tcx, root.to_def_id()), &mut datatypes);
            eprintln!("CLIENT_LOCAL\t{local:?}\t{:?}", decl.ty);
            let printed = format!("{:?}", decl.ty);
            if printed == "std::ops::Range<usize>"
                || printed.starts_with("[std::ops::Range<usize>; ")
            {
                let typing_env = ty::TypingEnv::post_analysis(tcx, root.to_def_id());
                eprintln!(
                    "CLIENT_TYPE_FACT\t{local:?}\t{printed}\t{}",
                    decl.ty.needs_drop(tcx, typing_env)
                );
            }
            if std::env::var_os("CLIENT_RESOURCE_LAYOUT").is_some() {
                if let ty::Adt(def, _) = decl.ty.kind() {
                    if def.did().krate != rustc_hir::def_id::LOCAL_CRATE
                        && tcx.def_path_str(def.did()).ends_with("::Tracked")
                    {
                        let env = ty::TypingEnv::post_analysis(tcx, root.to_def_id());
                        if let Ok(layout) = tcx.layout_of(env.as_query_input(decl.ty)) {
                            eprintln!(
                                "CLIENT_RESOURCE_LAYOUT\t{local:?}\t{}\t{}\t{}\t{}",
                                tcx.def_path_str(def.did()),
                                layout.size.bytes(),
                                layout.align.abi.bytes(),
                                decl.ty.needs_drop(tcx, env)
                            );
                        }
                    }
                }
            }
        }
        for (bb, data) in body.basic_blocks.iter_enumerated() {
            for (index, statement) in data.statements.iter().enumerate() {
                eprintln!(
                    "CLIENT_STMT\t{bb:?}\t{index}\t{:?}\t{:?}",
                    statement.source_info.span, statement.kind
                );
            }
            let term = data.terminator();
            let targets: Vec<_> = term.successors().collect();
            eprintln!("CLIENT_EDGE\t{bb:?}\t{targets:?}");
            eprintln!(
                "CLIENT_TERM\t{bb:?}\t{:?}\t{:?}",
                term.source_info.span, term.kind
            );
            if let TerminatorKind::Call {
                func,
                args,
                destination,
                ..
            } = &term.kind
            {
                let mut shape = term.kind.clone();
                if let TerminatorKind::Call { func, .. } = &mut shape {
                    *func = rustc_middle::mir::Operand::Copy(*destination);
                }
                eprintln!("CLIENT_CALL_SHAPE\t{bb:?}\t{shape:?}");
                if let ty::FnDef(def, generics) = func.ty(&body.local_decls, tcx).kind() {
                    eprintln!(
                        "CLIENT_CALL\t{bb:?}\t{}\t{:?}\t{args:?}\t{destination:?}\t{generics:?}",
                        tcx.def_path_str(*def),
                        tcx.def_span(*def)
                    );
                    if std::env::var_os("CLIENT_RESOLVE_INSTANCES").is_some() {
                        let resolved = ty::Instance::try_resolve(
                            tcx,
                            ty::TypingEnv::post_analysis(tcx, root.to_def_id()),
                            *def,
                            generics,
                        );
                        match resolved {
                            Ok(Some(instance)) => {
                                eprintln!(
                                    "CLIENT_INSTANCE\t{bb:?}\t{}\t{:?}\t{:?}",
                                    tcx.def_path_str(instance.def_id()),
                                    tcx.def_span(instance.def_id()),
                                    instance.args,
                                );
                                if std::env::var_os("CLIENT_EQUALITY_BODIES").is_some()
                                    && tcx.def_path_str(instance.def_id())
                                        == "<std::option::Option<T> as std::cmp::PartialEq>::eq"
                                    && format!("{:?}", instance.args) == "[std::ops::Range<usize>]"
                                {
                                    equality_body(tcx, instance, 0);
                                }
                            }
                            other => eprintln!(
                                "CLIENT_INSTANCE\t{bb:?}\tUNRESOLVED\tno-location (#0)\t{other:?}"
                            ),
                        }
                    }
                }
            }
        }
        if grouped {
            eprintln!("CALLER_END\t{path}");
        }
    }
}
fn selected_project_borrow(path: &str) -> bool {
    static PATHS: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    let Some(file) = std::env::var_os("CLIENT_PROJECT_BORROW_PATHS") else {
        return false;
    };
    PATHS
        .get_or_init(|| {
            std::fs::read_to_string(file)
                .expect("generated borrow selection")
                .lines()
                .map(str::to_owned)
                .collect()
        })
        .contains(path)
}

impl Callbacks for Probe {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.override_queries = Some(|_, providers| {
            providers.queries.mir_borrowck = |tcx, owner| {
                if matches!(
                    tcx.def_kind(owner),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                ) && std::env::var("CLIENT_FUNCTION").ok().as_deref()
                    == Some(tcx.def_path_str(owner.to_def_id()).as_str())
                {
                    extract(tcx, owner);
                } else if matches!(
                    tcx.def_kind(owner),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                ) && selected_project_borrow(&tcx.def_path_str(owner.to_def_id()))
                {
                    extract(tcx, owner);
                }
                let mut original = rustc_middle::query::Providers::default();
                rustc_borrowck::provide(&mut original);
                (original.mir_borrowck)(tcx, owner)
            };
        });
    }

    fn after_expansion<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        if std::env::var_os("CLIENT_PROJECT_SOURCE_ONLY").is_some() {
            project_source(tcx);
            return Compilation::Stop;
        }
        if std::env::var_os("CLIENT_PROJECT_INVENTORY").is_some() {
            project_inventory(tcx);
            if std::env::var_os("CLIENT_PROJECT_BORROW_PATHS").is_none() {
                return Compilation::Stop;
            }
            return Compilation::Continue;
        }
        Compilation::Continue
    }

    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        if std::env::var_os("CLIENT_PROJECT_BORROW_PATHS").is_some() {
            return Compilation::Stop;
        }
        let selector = std::env::var("CLIENT_FUNCTION")
            .expect("set CLIENT_FUNCTION to an exact definition path");
        for owner in tcx.hir_body_owners() {
            let path = tcx.def_path_str(owner.to_def_id());
            if selector == "*"
                && matches!(
                    tcx.def_kind(owner),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                )
            {
                let span = tcx.hir_span(tcx.local_def_id_to_hir_id(owner));
                eprintln!("CLIENT_CANDIDATE\t{path}\t{span:?}");
                continue;
            }
            if path != selector
                || !matches!(
                    tcx.def_kind(owner),
                    rustc_hir::def::DefKind::Fn | rustc_hir::def::DefKind::AssocFn
                )
            {
                continue;
            }
            tcx.ensure_ok().mir_borrowck(owner);
        }
        Compilation::Continue
    }
}
fn main() {
    let mut args: Vec<String> = std::env::args().collect();
    if std::env::var_os("CLIENT_RUSTC_WRAPPER").is_some() {
        args.remove(0);
    }
    run_compiler(&args, &mut Probe);
}
