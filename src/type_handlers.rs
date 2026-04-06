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

pub struct CppMap<'d> {
    // the std::pair
    pub item_type: &'d Type,
    pub cur_node: CppMapNode,
}
impl<'d> CppMap<'d> {
    pub fn opt_read(debug: &'d DebugPool, dump: &CoreDump, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("std::map<") {
            return None;
        }
        let item_type = debug.get_type(&composite_type.sub_types["value_type"]);
        // Get the inner (not type-erased) node type (TODO: Get the data offset from it)
        /*
        let (_, rb_ty) = get_field(debug, ty, &Path::root().field("_M_t"));
        let rb_ty = resolve_alias_chain(debug, debug.get_type(&rb_ty));
        let Type::Struct(ct) = rb_ty else { panic!("RB Tree not a struct/class") };
        let node_type = resolve_alias_chain(debug, debug.get_type(&ct.sub_types["_Link_type"]));
        let Type::Pointer(node_type) = node_type else { panic!("Expected pointer, got {:?}", node_type)};
        let node_type = resolve_alias_chain(debug, debug.get_type(node_type));
        */

        let node_count = dump.read_ptr(addr + 0x28);
        println!("> node_count={node_count}");

        let first_node = if node_count > 0 {
            let mut cur_n = CppMapNode::read(dump, addr + 8);
            cur_n = CppMapNode::read(dump, cur_n.left_addr);
            cur_n
        }else {
            CppMapNode::read(dump, 0)
        };

        Some(Self {
            item_type,
            cur_node: first_node,
        })
    }
}
#[derive(Copy,Clone)]
pub struct CppMapNode {
    addr: u64,
    left_addr: u64,
    parent_addr: u64,
    right_addr: u64,
}
impl CppMapNode {
    fn read(dump: &CoreDump, addr: u64) -> Self {
        if addr == 0 {
            return Self { addr, left_addr: 0, parent_addr: 0, right_addr: 0 };
        }
        Self {
            addr,
            parent_addr: dump.read_ptr(addr + 0x8),
            left_addr: dump.read_ptr(addr + 0x10),
            right_addr: dump.read_ptr(addr + 0x18),
        }
    }
    
    pub fn is_nil(&self) -> bool {
        self.addr == 0
    }
    pub fn data_addr(&self) -> u64 {
        self.addr + 0x20
    }

    pub fn next(&self, dump: &CoreDump) -> Self {
        let mut cur_n = *self;
        // Increment iterator (See `_Rb_tree_increment` implementtion)
        if cur_n.right_addr != 0 {
            // Iterate into the RHS until no more LHS
            cur_n = Self::read(dump, cur_n.right_addr);
            while cur_n.left_addr != 0 {
                cur_n = Self::read(dump, cur_n.left_addr);
            }
        }
        else {
            let mut p = Self::read(dump, cur_n.parent_addr);
            while cur_n.addr == p.right_addr {
                let pa = p.parent_addr;
                cur_n = p;
                p = Self::read(dump, pa);
            }
            if cur_n.right_addr != p.addr {
                cur_n = p;
            }
        }
        cur_n
    }
}