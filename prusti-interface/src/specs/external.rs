use rustc_hir::intravisit::{self, Visitor};
use rustc_hir::def_id::DefId;
use rustc_middle::hir::map::Map;
use rustc_middle::ty::TyCtxt;
use rustc_span::{Span, MultiSpan};

use std::collections::HashMap;
use crate::environment::Environment;
use crate::PrustiError;

/// This struct is used to build a mapping of external functions to their
/// Prusti specifications (see `extern_fn_map`).
pub struct ExternSpecResolver<'tcx> {
    tcx: TyCtxt<'tcx>,

    /// Maps real functions (keyed by their `DefId`) to Prusti-generated fake
    /// functions with specifications. The mapping may also optionally contain
    /// the `DefId` of the implementing type to account for trait
    /// implementations.
    extern_fn_map: HashMap<DefId, (Option<DefId>, DefId)>,

    /// Duplicate specifications detected, keyed by the `DefId` of the function
    /// to be specified.
    spec_duplicates: HashMap<DefId, Vec<(DefId, Span)>>,
}

impl<'tcx> ExternSpecResolver<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx: tcx,
            extern_fn_map: HashMap::new(),
            spec_duplicates: HashMap::new(),
        }
    }

    /// Registers an external function specification. The arguments for this
    /// function are the same as arguments given to a function visit in an
    /// intravisit visitor.
    ///
    /// In case of duplicates, the function is added to `spec_duplicates`, and
    /// will later (in `check_duplicates`) be reported as an error. Otherwise,
    /// the function is added to `extern_fn_map`.
    pub fn add_extern_fn(
        &mut self,
        fn_kind: intravisit::FnKind<'tcx>,
        fn_decl: &'tcx rustc_hir::FnDecl,
        body_id: rustc_hir::BodyId,
        span: Span,
        id: rustc_hir::hir_id::HirId
    ) {
        let mut visitor = ExternSpecVisitor {
            tcx: self.tcx,
            spec_found: None,
        };
        visitor.visit_fn(fn_kind, fn_decl, body_id, span, id);
        let current_def_id = self.tcx.hir().local_def_id(id).to_def_id();
        if let Some((def_id, impl_ty, span)) = visitor.spec_found {
            match self.extern_fn_map.get(&def_id) {
                Some((existing_impl_ty, _)) if existing_impl_ty == &impl_ty => {
                    match self.spec_duplicates.get_mut(&def_id) {
                        Some(dups) => {
                            dups.push((current_def_id, span));
                        }
                        None => {
                            self.spec_duplicates.insert(def_id, vec![(current_def_id, span)]);
                        }
                    }
                }
                _ => {
                    // TODO: what if def_id was present, but impl_ty was different?
                    self.extern_fn_map.insert(def_id, (impl_ty, current_def_id));
                }
            }
        }
    }

    /// Report errors for duplicate specifications found during specification
    /// collection.
    pub fn check_duplicates(&self, env: &Environment<'tcx>) {
        for (&def_id, specs) in self.spec_duplicates.iter() {
            let function_name = env.get_item_name(def_id);
            PrustiError::incorrect(
                format!("duplicate specification for {}", function_name),
                MultiSpan::from_spans(specs.iter()
                    .map(|s| s.1)
                    .collect())
            ).emit(env);
        }
    }

    pub fn get_extern_fn_map(&self) -> HashMap<DefId, (Option<DefId>, DefId)> {
        self.extern_fn_map.clone()
    }
}

/// A visitor that is called on external specification methods, as generated by
/// the external spec rewriter, looking specifically for the call to the
/// external function.
///
/// TODO: is the HIR representation stable enought that this could be
/// accomplished by a nested match rather than a full visitor?
struct ExternSpecVisitor<'tcx> {
    tcx: TyCtxt<'tcx>,
    spec_found: Option<(DefId, Option<DefId>, Span)>,
}

/// Gets the `DefId` from the given path.
fn get_impl_type<'tcx>(qself: &rustc_hir::QPath<'tcx>) -> Option<DefId> {
    if let rustc_hir::QPath::TypeRelative(ty, _) = qself {
        if let rustc_hir::TyKind::Path(qpath) = &ty.kind {
            if let rustc_hir::QPath::Resolved(_, path) = qpath {
                if let rustc_hir::def::Res::Def(_, id) = path.res {
                    return Some(id);
                }
            }
        }
    }
    return None;
}

impl<'tcx> Visitor<'tcx> for ExternSpecVisitor<'tcx> {
    type Map = Map<'tcx>;

    fn nested_visit_map<'this>(&'this mut self) -> intravisit::NestedVisitorMap<Self::Map> {
        let map = self.tcx.hir();
        intravisit::NestedVisitorMap::All(map)
    }

    fn visit_expr(&mut self, ex: &'tcx rustc_hir::Expr<'tcx>) {
        if self.spec_found.is_some() {
            return;
        }
        if let rustc_hir::ExprKind::Call(ref callee_expr, ref arguments) = ex.kind {
            if let rustc_hir::ExprKind::Path(ref qself) = callee_expr.kind {
                let res = self.tcx.typeck(callee_expr.hir_id.owner).qpath_res(qself, callee_expr.hir_id);
                if let rustc_hir::def::Res::Def(_, def_id) = res {
                    self.spec_found = Some((def_id, get_impl_type(qself), ex.span));
                    return;
                }
            }
        }
        intravisit::walk_expr(self, ex);
    }
}
