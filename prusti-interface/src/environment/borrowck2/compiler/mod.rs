use std::{collections::HashMap, rc::Rc};
use rustc_middle::{mir, ty, ty::TyCtxt};
use rustc_hir::{def_id::DefId};
use rustc_mir::borrow_check::facts::AllFacts;
use rustc_mir::borrow_check::location::LocationTable;
use rustc_mir::borrow_check::universal_regions::UniversalRegions;
use rustc_mir::borrow_check::location::LocationIndex;

mod extract;
mod derive;

pub(super) use self::extract::enrich_mir_body;

/// A wrapper around MIR body that hides unnecessary details.
pub struct MirBody<'tcx> {
    def_id: DefId,
    // Information obtained from the borrow checker.
    inputs_and_output: Vec<ty::Ty<'tcx>>,
    body: mir::Body<'tcx>,
    tcx: TyCtxt<'tcx>,
    universal_regions: Rc<UniversalRegions<'tcx>>,
    universal_regions_outlives: Vec<(ty::RegionVid, ty::RegionVid)>,
    polonius_facts: AllFacts,
    location_table: LocationTable,
    // Derived information.
    /// The names of local variables.
    local_names: HashMap<mir::Local, String>,
    /// Outlives relations at the given statement.
    outlives: HashMap<LocationIndex, Vec<(ty::RegionVid, ty::RegionVid)>>,
}

pub struct Variable<'body, 'tcx> {
    id: mir::Local,
    decl: &'body mir::LocalDecl<'tcx>,
    body: &'body MirBody<'tcx>,
}

pub struct BasicBlock<'body, 'tcx> {
    index: mir::BasicBlock,
    data: &'body mir::BasicBlockData<'tcx>,
    body: &'body MirBody<'tcx>,
}

pub struct Statement<'body, 'tcx> {
    location: mir::Location,
    statement: &'body mir::Statement<'tcx>,
    body: &'body MirBody<'tcx>,
}

pub struct Terminator<'body, 'tcx> {
    location: mir::Location,
    terminator: &'body mir::Terminator<'tcx>,
    body: &'body MirBody<'tcx>,
}

impl<'tcx> MirBody<'tcx> {
    pub fn iter_locals<'a>(&'a self) -> impl Iterator<Item=Variable<'a, 'tcx>> {
        self.body.local_decls.iter_enumerated().map(move |(id, decl)| {
            Variable {
                id,
                decl,
                body: self,
            }
        })
    }
    pub fn basic_block_indices(&self) -> impl Iterator<Item=mir::BasicBlock> {
        self.body.basic_blocks().indices()
    }
    pub fn get_block<'a>(&'a self, index: mir::BasicBlock) -> BasicBlock<'a, 'tcx> {
        BasicBlock {
            index,
            data: &self.body[index],
            body: self,
        }
    }
    pub fn iter_inputs_and_output_types<'a>(&'a self) -> impl Iterator<Item=&'a ty::Ty<'tcx>> {
        self.inputs_and_output.iter()
    }
    pub fn get_universal_regions(&self) -> impl Iterator<Item=ty::RegionVid> {
        self.universal_regions.universal_regions()
    }
    pub fn get_universal_region_names<'a>(&'a self) -> impl Iterator<Item=(ty::Region<'tcx>, ty::RegionVid)> + 'a {
        self.universal_regions.named_universal_regions()
    }
    /// The region that corresponds to `'static`.
    pub fn get_static_region(&self) -> ty::RegionVid {
        self.universal_regions.fr_static
    }
    /// The region that corresponds to the lifetime of the function itself.
    pub fn get_function_region(&self) -> ty::RegionVid {
        self.universal_regions.fr_fn_body
    }
    pub fn get_universal_region_outlives(&self) -> &[(ty::RegionVid, ty::RegionVid)] {
        &self.universal_regions_outlives
    }
    pub fn get_outlives_at_start(&self, location: mir::Location) -> Option<&Vec<(ty::RegionVid, ty::RegionVid)>> {
        let index = self.location_table.start_index(location);
        self.outlives.get(&index)
    }
    pub fn get_outlives_at_mid(&self, location: mir::Location) -> Option<&Vec<(ty::RegionVid, ty::RegionVid)>> {
        let index = self.location_table.mid_index(location);
        self.outlives.get(&index)
    }
}

impl<'body, 'tcx> Variable<'body, 'tcx> {
    /// Return the user-friendly name of the variable.
    pub fn name(&self) -> Option<&str> {
        self.body.local_names.get(&self.id).map(|s| s.as_ref())
    }
    /// Return the identifier of the variable.
    pub fn id(&self) -> mir::Local {
        self.id
    }
    /// Return the type of the variable.
    pub fn ty(&self) -> ty::Ty<'tcx> {
        self.decl.ty
    }
}

impl<'body, 'tcx> BasicBlock<'body, 'tcx> {
    pub fn iter_statements<'a>(&'a self) -> impl Iterator<Item=Statement<'a, 'tcx>> {
        self.data.statements.iter().enumerate().map(
            move |(index, statement)| {
                Statement {
                    location: mir::Location {
                        block: self.index,
                        statement_index: index,
                    },
                    statement,
                    body: self.body
                }
            }
        )
    }
    pub fn terminator<'a>(&'a self) -> Option<Terminator<'a, 'tcx>> {
        self.data.terminator.as_ref().map(|terminator| {
            Terminator {
                location: mir::Location {
                    block: self.index,
                    statement_index: self.data.statements.len(),
                },
                terminator,
                body: self.body,
            }
        })
    }
}

impl<'body, 'tcx> Statement<'body, 'tcx> {
    pub fn index(&self) -> usize {
        self.location.statement_index
    }
    pub fn kind(&self) -> &mir::StatementKind<'tcx> {
        &self.statement.kind
    }
    pub fn location(&self) -> mir::Location {
        self.location
    }
}

impl<'body, 'tcx> Terminator<'body, 'tcx> {
    pub fn basic_block(&self) -> mir::BasicBlock {
        self.location.block
    }
    pub fn kind(&self) -> &mir::TerminatorKind<'tcx> {
        &self.terminator.kind
    }
}