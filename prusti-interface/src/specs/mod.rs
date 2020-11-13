use prusti_specs::specifications::{json::Assertion as JsonAssertion, SpecType};
use rustc_ast::ast;
use rustc_hir::{intravisit, ItemKind};
use rustc_middle::hir::map::Map;
use rustc_middle::ty::TyCtxt;
use rustc_span::Span;
use rustc_span::symbol::Symbol;
use rustc_hir::def_id::{DefId, LocalDefId};
use std::collections::HashMap;
use std::convert::TryInto;
use crate::environment::Environment;
use crate::utils::{
    has_spec_only_attr, has_extern_spec_attr, read_prusti_attr, read_prusti_attrs, has_prusti_attr
};
use log::debug;

pub mod external;
pub mod typed;

use typed::StructuralToTyped;
use typed::SpecIdRef;
use std::fmt;
use crate::specs::external::ExternSpecResolver;
use prusti_specs::specifications::common::SpecificationId;

struct SpecItem {
    spec_id: typed::SpecificationId,
    spec_type: SpecType,
    specification: JsonAssertion,
}

impl fmt::Debug for SpecItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpecItem")
         .field("spec_id", &self.spec_id)
         .finish()
    }
}

struct Item<'tcx> {
    name: Symbol,
    attrs: &'tcx [ast::Attribute],
}

pub struct SpecCollector<'tcx> {
    tcx: TyCtxt<'tcx>,
    spec_items: Vec<SpecItem>,
    def_spec: typed::DefSpecificationMap<'tcx>,
    typed_expressions: HashMap<String, LocalDefId>,
    extern_resolver: ExternSpecResolver<'tcx>,
}

impl<'tcx> SpecCollector<'tcx> {
    pub fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx: tcx,
            spec_items: Vec::new(),
            def_spec: HashMap::new(),
            typed_expressions: HashMap::new(),
            extern_resolver: ExternSpecResolver::new(tcx),
        }
    }

    pub fn determine_typed_procedure_specs(self) -> typed::SpecificationMap<'tcx> {
        let typed_expressions = self.typed_expressions;
        let tcx = self.tcx;
        self.spec_items
            .into_iter()
            .map(|spec_item| {
                let assertion = reconstruct_typed_assertion(
                    spec_item.specification,
                    &typed_expressions,
                    tcx
                );
                (spec_item.spec_id, assertion)
            })
            .collect()
    }

    pub fn determine_def_specs(&self, env: &Environment<'tcx>) -> typed::DefSpecificationMap<'tcx> {
        let mut def_spec = self.def_spec.clone();
        self.extern_resolver.check_duplicates(env);
        // TODO: do something with the traits
        for (real_id, (_, spec_id)) in self.extern_resolver.get_extern_fn_map().iter() {
            if def_spec.contains_key(real_id) {
                panic!("duplicate spec"); // TODO: proper error
            }
            println!("real: {:#?} specs: {:#?}", real_id, spec_id);
            if let Some(specs) = def_spec.get(spec_id) {
                println!("> {:#?}", specs);
                def_spec.insert(*real_id, specs.to_vec());
            }
        }
        def_spec
    }
}

fn get_procedure_spec_ids(def_id: DefId, attrs: &[ast::Attribute]) -> Vec<SpecIdRef> {
    let mut spec_id_refs = vec![];

    let parse_spec_id = |spec_id: String| -> SpecificationId {
        spec_id.try_into().expect(
            &format!("cannot parse the spec_id attached to {:?}", def_id)
        )
    };

    spec_id_refs.extend(
        read_prusti_attrs("pre_spec_id_ref", attrs).into_iter().map(
            |raw_spec_id| SpecIdRef::Precondition(parse_spec_id(raw_spec_id))
        )
    );
    spec_id_refs.extend(
        read_prusti_attrs("post_spec_id_ref", attrs).into_iter().map(
            |raw_spec_id| SpecIdRef::Postcondition(parse_spec_id(raw_spec_id))
        )
    );
    spec_id_refs.extend(
        read_prusti_attrs("pledge_spec_id_ref", attrs).into_iter().map(
            |value| {
                let mut value = value.splitn(2, ":");
                let raw_lhs_spec_id = value.next().unwrap();
                let raw_rhs_spec_id = value.next().unwrap();
                let lhs_spec_id = if !raw_lhs_spec_id.is_empty() {
                    Some(parse_spec_id(raw_lhs_spec_id.to_string()))
                } else {
                    None
                };
                let rhs_spec_id = parse_spec_id(raw_rhs_spec_id.to_string());
                SpecIdRef::Pledge{ lhs: lhs_spec_id, rhs: rhs_spec_id }
            }
        )
    );
    debug!("Function {:?} has specification ids {:?}", def_id, spec_id_refs);
    spec_id_refs
}

fn reconstruct_typed_assertion<'tcx>(
    assertion: JsonAssertion,
    typed_expressions: &HashMap<String, LocalDefId>,
    tcx: TyCtxt<'tcx>
) -> typed::Assertion<'tcx> {
    assertion.to_typed(typed_expressions, tcx)
}

fn deserialize_spec_from_attrs(attrs: &[ast::Attribute]) -> JsonAssertion {
    let json_string = read_prusti_attr("assertion", attrs)
        .expect("could not find prusti::assertion");
    JsonAssertion::from_json_string(&json_string)
}

impl<'tcx> intravisit::Visitor<'tcx> for SpecCollector<'tcx> {
    type Map = Map<'tcx>;

    fn nested_visit_map(&mut self) -> intravisit::NestedVisitorMap<Self::Map> {
        let map = self.tcx.hir();
        intravisit::NestedVisitorMap::All(map)
    }

    fn visit_fn(
        &mut self,
        fn_kind: intravisit::FnKind<'tcx>,
        fn_decl: &'tcx rustc_hir::FnDecl,
        body_id: rustc_hir::BodyId,
        span: Span,
        id: rustc_hir::hir_id::HirId,
    ) {
        intravisit::walk_fn(self, fn_kind, fn_decl, body_id, span, id);

        let local_id = self.tcx.hir().local_def_id(id);
        let def_id = local_id.to_def_id();
        let attrs = fn_kind.attrs();

        // Collect external function specifications
        if has_extern_spec_attr(attrs) {
            self.extern_resolver.add_extern_fn(fn_kind, fn_decl, body_id, span, id);
        }

        // Collect procedure specifications
        let procedure_spec_ids = get_procedure_spec_ids(def_id, attrs);
        if procedure_spec_ids.len() > 0 {
            println!("{:#?} has {} specs", def_id, procedure_spec_ids.len());
            self.def_spec.insert(def_id, procedure_spec_ids);
        }

        // Collect a typed expression
        if let Some(expr_id) = read_prusti_attr("expr_id", attrs) {
            self.typed_expressions.insert(expr_id, local_id);
        }

        // Collect a specification id and its assertion
        if let Some(raw_spec_id) = read_prusti_attr("spec_id", attrs) {
            let spec_id: SpecificationId = raw_spec_id.try_into()
                .expect("failed conversion to SpecificationId");
            let specification = deserialize_spec_from_attrs(attrs);

            // Detect the kind of specification
            let spec_type = if has_prusti_attr(attrs, "loop_body_invariant_spec") {
                SpecType::Invariant
            } else {
                let fn_name = match fn_kind {
                    intravisit::FnKind::ItemFn(ref ident, ..) |
                    intravisit::FnKind::Method(ref ident, ..) => ident.name.to_ident_string(),
                    intravisit::FnKind::Closure(..) => unreachable!(
                        "a closure is annotated with prusti::spec_id but not with \
                        prusti::loop_body_invariant_spec"
                    ),
                };
                if fn_name.starts_with("prusti_pre_item_") {
                    SpecType::Precondition
                } else if fn_name.starts_with("prusti_post_item_") {
                    SpecType::Postcondition
                } else {
                    unreachable!()
                }
            };

            let spec_item = SpecItem {spec_id, spec_type, specification};
            self.spec_items.push(spec_item);
        }
    }
}
