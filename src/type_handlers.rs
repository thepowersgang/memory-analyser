use super::Input;
use super::core_dump::CoreDump;
use super::debug_info::Type;
use super::visit_helpers::Path;

pub struct CppUniquePtr<'d> {
    pub target_addr: u64,
    pub target_ty: &'d Type,
    //pub alloc_addr: u64,
    //pub alloc_ty: &'d Type,
}
impl<'d> CppUniquePtr<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if composite_type.name().starts_with("std::unique_ptr<") {
            let (o, ptr_ty) = input.get_field(ty, Path::root().field("_M_t").parent(0).field("_M_t").parent(0).parent(1).field("_M_head_impl"));
            let ptr_ty = input.resolve_alias_chain_tr(&ptr_ty);
            let Type::Pointer(inner_ty, _) = ptr_ty else { panic!("Expected pointer") };
            let inner_ty = input.resolve_alias_chain_tr(&inner_ty);

            Some(CppUniquePtr {
                target_addr: input.dump.read_ptr(addr + o),
                target_ty: inner_ty
            })
        }
        else {
            None
        }
    }
}

pub struct CppSharedPtr<'d> {
    pub target_addr: u64,
    pub target_ty: &'d Type,
    pub count_addr: u64,
    pub count_ty: &'d Type,
    //pub alloc_addr: u64,
    //pub alloc_ty: &'d Type,
}
impl<'d> CppSharedPtr<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if composite_type.name().starts_with("std::shared_ptr<") {
            let (ptr_o, ptr_ty) = input.get_field(ty, Path::root().parent(0).field("_M_ptr"));
            let (rc_o , rc_ty ) = input.get_field(ty, Path::root().parent(0).field("_M_refcount").field("_M_pi"));
            let inner_ty = {
                let i = input.resolve_alias_chain_tr(&ptr_ty);
                let Type::Pointer(i, _) = i else { panic!("Expected pointer") };
                input.resolve_alias_chain_tr(i)
            };
            let rc_ty = {
                let i = input.resolve_alias_chain_tr(&rc_ty);
                let Type::Pointer(i, _) = i else { panic!("Expected pointer") };
                input.resolve_alias_chain_tr(i)
            };

            Some(CppSharedPtr {
                target_addr: input.dump.read_ptr(addr + ptr_o),
                target_ty: inner_ty,
                count_addr: input.dump.read_ptr(addr + rc_o),
                count_ty: rc_ty,
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
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("std::vector<") {
            return None;
        }
        let inner_ty = {
            let (_, ty) = input.get_field(ty, Path::root().parent(0).field("_M_impl").parent(1).field("_M_start"));
            let ty = input.resolve_alias_chain_tr(&ty);
            let Type::Pointer(ty, _) = ty else { panic!("Expected pointer, got {:?}", ty); };
            *ty
            };
        Some(CppVector {
            begin: input.dump.read_ptr(addr + 0),
            end: input.dump.read_ptr(addr + 8),
            alloc_end: input.dump.read_ptr(addr + 16),
            item_ty: input.debug.get_type(&inner_ty),
        })
    }
}
pub struct CppString {
    pub ptr: u64,
    pub len: u64,
    /// If zero, the string buffer is stored in the `std::string` itself
    pub capacity: u64,
}
impl CppString {
    pub fn opt_read(input: &Input, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        match composite_type.name() {
        "std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >" => {},
        _ => return None,
        }
        if false {
            print!("CPP STRING: "); crate::dump_type_fields(input.debug, ty, 0); println!("");
        }
        let ptr = input.dump.read_ptr(addr + 0);
        let len = input.dump.read_ptr(addr + 8);
        let capacity = if ptr == addr + 0x10 {
                // Use 0 as a sentinel for inline storage
                0
            }
            else {
                input.dump.read_ptr(addr + 16)
            };
        Some(CppString {
            ptr,
            len,
            capacity,
        })
    }
}

pub struct CppMap<'d> {
    // the std::pair
    pub item_type: &'d Type,
    pub cur_node: CppMapNode,
}
impl<'d> CppMap<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("std::map<") {
            return None;
        }
        let item_type = input.debug.get_type(&composite_type.sub_types["value_type"]);
        // Get the inner (not type-erased) node type (TODO: Get the data offset from it)
        /*
        let (_, rb_ty) = input.get_field(ty, Path::root().field("_M_t"));
        let rb_ty = input.resolve_alias_chain_tr(&rb_ty);
        let Type::Struct(ct) = rb_ty else { panic!("RB Tree not a struct/class") };
        let node_type = input.resolve_alias_chain_tr(&ct.sub_types["_Link_type"]);
        let Type::Pointer(node_type) = node_type else { panic!("Expected pointer, got {:?}", node_type)};
        let node_type = input.resolve_alias_chain_tr(node_type);
        */

        let node_count = input.dump.read_ptr(addr + 0x28);
        println!("> node_count={node_count}");

        let first_node = if node_count > 0 {
            let mut cur_n = CppMapNode::read(input.dump, addr + 8);
            cur_n = CppMapNode::read(input.dump, cur_n.left_addr);
            cur_n
        }else {
            CppMapNode::read(input.dump, 0)
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

pub struct CppUnorderedMap<'d> {
    // the std::pair
    pub item_type: &'d Type,
    pub first_node: CppUnorderedMapNode,
}
impl<'d> CppUnorderedMap<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !composite_type.name().starts_with("std::unordered_map<") {
            return None;
        }
        let item_type = input.resolve_alias_chain_tr(&composite_type.sub_types["value_type"]);
        //print!("UNORDERED MAP: "); crate::dump_type_fields(debug, ty, 0); println!("");
        let node_ptr_ty = {
            let i = input.resolve_alias_chain_tr(&composite_type.fields[0].ty);
            let Type::Struct(i) = i else { panic!("Expected struct, got {}", input.debug.fmt_type(i)) };
            input.resolve_alias_chain_tr(&i.sub_types["__node_ptr"])
        };
        let node_ty = {
            let Type::Pointer(i, _) = node_ptr_ty else { panic!("Expected pointer, got {}", input.debug.fmt_type(node_ptr_ty)) };
            input.resolve_alias_chain_tr(i)
        };
        let (data_ofs, _) = input.get_field(node_ty, Path::root().parent(1).parent(0).field("_M_storage").field("_M_storage").field("__data"));
        //print!("UNORDERED MAP NODE: "); crate::dump_type_fields(debug, node_ty, 0); println!("");

        // _M_buckets: @0x0: *=*=::std::__detail::struct _Hash_node_base,
        // _M_bucket_count: @0x8: prim64,
        // _M_before_begin._M_nxt: @0x10: *::std::__detail::struct _Hash_node_base,
        // _M_element_count: @0x18: prim64,
        // _M_rehash_policy._M_max_load_factor: @0x20: prim32,
        // _M_rehash_policy._M_next_resize: @0x28: prim64,
        // _M_single_bucket: @0x30: *=::std::__detail::struct _Hash_node_base,

        let first_node_addr = input.dump.read_ptr(addr + 0x10);
        Some(CppUnorderedMap {
            item_type,
            first_node: CppUnorderedMapNode::read(input.dump, first_node_addr, data_ofs)
        })
    }
}
pub struct CppUnorderedMapNode {
    node_addr: u64,
    data_ofs: u64,
}
impl CppUnorderedMapNode {
    fn read(dump: &CoreDump, addr: u64, data_ofs: u64) -> Self {
        let _ = dump;
        if addr == 0 {
            return CppUnorderedMapNode { node_addr: 0, data_ofs };
        }
        CppUnorderedMapNode {
            node_addr: addr,
            data_ofs,
        }
    }
    pub fn is_nil(&self) -> bool {
        self.node_addr == 0
    }
    pub fn data_addr(&self) -> u64 {
        self.node_addr + self.data_ofs
    }
    pub fn next(&self, dump: &CoreDump) -> Self {
        let next_addr = dump.read_ptr(self.node_addr);
        Self::read(dump, next_addr, self.data_ofs)
    }
}

/// A mrustc `TAGGED_UNION` structure
pub struct MrustcTaggedUnion<'d> {
    /// Offset to the data, relative to the passed `addr`
    pub data_ofs: u64,
    /// Name and data type of the current variant
    pub variant: Option<(&'d str, &'d Type)>,
    /// Other data fields on the union (`TAGGED_UNION_EX` can add methods and data)
    pub other_fields: &'d [super::debug_info::CompositeField],
}
impl<'d> MrustcTaggedUnion<'d> {
    pub fn opt_read(input: &Input<'d>, ty: &'d Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if !(composite_type.fields.len() >= 2
            && composite_type.fields[0].name == "m_tag"
            && composite_type.fields[1].name == "m_data"
            ) {
            return None;
        }
        let d_ty = input.resolve_alias_chain_tr(&composite_type.fields[1].ty);
        let Type::Union(d_u) = d_ty else { return None };
        let t_o = composite_type.fields[0].offset;
        let d_o = composite_type.fields[1].offset;
        let tag = input.dump.read_u32(addr + t_o) as usize;
        let variant = if tag == 0 {
                None
            }
            else if tag > d_u.fields.len() {
                println!("Invalid tagged union: tag out of range {:#x} > {}", tag, d_u.fields.len());
                None
            }
            else {
                let f = &d_u.fields[tag-1];
                let ty = input.debug.get_type(&f.ty);
                Some((&f.name[..], ty))
            };
        Some(MrustcTaggedUnion {
            data_ofs: d_o,
            variant,
            other_fields: &composite_type.fields[2..]
        })
    }
}

pub struct MrustcRcString {
    pub data_addr: u64,
    pub string_ptr: u64,
    pub string_len: u64,
    //pub refcount: u32,
}
impl MrustcRcString {
    pub fn opt_read<'d>(input: &Input<'d>, ty: &'d Type, addr: u64) -> Option<Self> {
        let Type::Struct(composite_type) = ty else {
            return None;
        };
        if composite_type.name() != "RcString" {
            return None;
        }
        let Type::Pointer(inner_ty,_) = input.debug.get_type(&composite_type.fields[0].ty) else { panic!() };
        let inner_ty = input.debug.get_type(inner_ty);
        if false {
            print!("RCSTRING: "); crate::dump_type_fields(input.debug, inner_ty, 0); println!("");
        }
        let (data_ofs, _) = input.get_field(inner_ty, Path::root().field("data"));
        let (size_ofs, _) = input.get_field(inner_ty, Path::root().field("size"));
        let ptr = input.dump.read_ptr(addr);
        Some(MrustcRcString {
            data_addr: ptr,
            string_ptr: if ptr == 0 { 0 } else { ptr + data_ofs },
            string_len: if ptr == 0 { 0 } else { input.dump.read_u32(ptr + size_ofs) as u64 },
        })
    }
}