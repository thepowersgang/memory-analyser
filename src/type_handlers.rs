use super::core_dump::CoreDump;
use super::debug_info::{DebugPool,Type};
use super::visit_helpers::{Path, get_field, resolve_alias_chain};

pub struct CppUniquePtr<'d> {
    pub target_addr: u64,
    pub target_ty: &'d Type,
    //pub alloc_addr: u64,
    //pub alloc_ty: &'d Type,
}
impl<'d> CppUniquePtr<'d> {
    pub fn opt_read(debug: &'d DebugPool, dump: &CoreDump, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if composite_type.name().starts_with("std::unique_ptr<") {
            let (o, ptr_ty) = get_field(debug, ty, &Path::root().field("_M_t").parent(0).field("_M_t").parent(0).parent(1).field("_M_head_impl"));
            let ptr_ty = resolve_alias_chain(debug, debug.get_type(&ptr_ty));
            let Type::Pointer(inner_ty) = ptr_ty else { panic!("Expected pointer") };
            let inner_ty = resolve_alias_chain(debug, debug.get_type(&inner_ty));

            Some(CppUniquePtr {
                target_addr: dump.read_ptr(addr + o),
                target_ty: inner_ty
            })
        }
        else {
            None
        }
    }
}

pub struct CppVector<'d> {
    pub begin: u64,
    pub end: u64,
    pub alloc_end: u64,
    pub item_ty: &'d Type,
}
impl<'d> CppVector<'d> {
    pub fn opt_read(debug: &'d DebugPool, dump: &CoreDump, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("std::vector<") {
            return None;
        }
        let inner_ty = {
            let (_, ty) = get_field(debug, ty, &Path::root().parent(0).field("_M_impl").parent(1).field("_M_start"));
            let ty = debug.get_type(&ty);
            let ty = resolve_alias_chain(debug, ty);
            let Type::Pointer(ty) = ty else { panic!("Expected pointer, got {:?}", ty); };
            *ty
            };
        Some(CppVector {
            begin: dump.read_ptr(addr + 0),
            end: dump.read_ptr(addr + 8),
            alloc_end: dump.read_ptr(addr + 16),
            item_ty: debug.get_type(&inner_ty),
        })
    }
}