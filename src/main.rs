mod core_dump;
mod debug_info;

#[derive(Clone)]
struct CpuState {
    // AMD64:
    pc: u64,
    gprs: [u64; 16],
}
impl CpuState {
    fn stub() -> Self {
        CpuState { pc: 0, gprs: [0; 16] }
    }
    fn get_pc(&self) -> u64 {
        self.pc
    }
}
impl ::std::fmt::Display for CpuState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PC={:#x}", self.pc)?;
        const GPR_NAMES: [&str;16] = ["RAX","RDX","RCX","RBX", "RSI","RDI","RBP","RSP", "R8","R9","R10","R11", "R12","R12","R14","R15"];
        for (i,(n,v)) in GPR_NAMES.iter().zip(self.gprs.iter()).enumerate() {
            if i % 4 == 0 {
                f.write_str("\n")?;
            }
            else {
                f.write_str(" ")?;
            }
            write!(f,"{n:3}:{v:016x}")?;
        }
        Ok(())
    }
}

fn main() {
    let path = ::std::env::args().nth(1);
    let dump = if let Some(path) = path {
        core_dump::CoreDump::open(path.as_ref()).expect("Unable to open core dump")
    }
    else {
        core_dump::CoreDump::new_stub()
    };
    let mut debug = debug_info::DebugPool::new();
    for (module_path, base, file_base) in dump.modules()
    {
        match debug.add_file(&module_path, base, file_base)
        {
        Ok(()) => {},
        Err(e) => panic!("Failed to load {:?}: {:?}", module_path, e),
        }
    }
    
    let state_in_dump = dump.get_thread(0);
    println!("STATE: {}", state_in_dump);
    let state_main = debug.get_caller(&state_in_dump, &dump);
    println!("STATE: {}", state_main);

    let (addr, ty) = debug.get_variable(&state_main, &dump, "crate");
    visit_type(0, &debug, &dump, &debug.get_type(&ty), addr, Path::root());
}

fn dump_type_fields(debug: &debug_info::DebugPool, ty: &debug_info::Type, ofs: u64) {
    match ty {
    debug_info::Type::Alias(ty) => dump_type_fields(debug, debug.get_type(ty), ofs),
    debug_info::Type::Struct(composite_type) => {
        print!("{} {{", composite_type.name());
        for (o,ty) in composite_type.parents() {
            dump_type_fields(debug, &debug.get_type(ty), ofs + o);
        }
        for f in composite_type.iter_fields() {
            print!(" {}: ", f.name);
            dump_type_fields(debug, &debug.get_type(&f.ty), ofs+f.offset);
            print!(",");
        }
        print!(" }}");
        },
    debug_info::Type::Union(_) => todo!(),
    _ => print!("@{ofs:#x}: {}", debug.fmt_type(ty)),
    }
}

struct Path<'a> {
    parent: Option<&'a Path<'a>>,
    node: PathNode<'a>,
}
impl<'a> Path<'a> {
    fn root() -> Self {
        Path {
            parent: None,
            node: PathNode::Null,
        }
    }
    fn add<'r>(&'r self, node: PathNode<'r>) -> Path<'r> {
        Path {
            parent: if let PathNode::Null = self.node { None } else { Some(self) },
            node,
        }
    }
    fn index(&self, index: usize) -> Path<'_> {
        self.add(PathNode::Index(index))
    }
    fn parent(&self, index: usize) -> Path<'_> {
        self.add(PathNode::Parent(index))
    }
    fn field<'r>(&'r self, name: &'r str) -> Path<'r> {
        self.add(PathNode::Field(name))
    }
    fn deref(&self) -> Path<'_> {
        self.add(PathNode::Deref)
    }
}
impl<'a> ::std::fmt::Display for Path<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(v) = self.parent {
            v.fmt(f)?;
        }
        match self.node {
        PathNode::Null => Ok(()),
        PathNode::Field(name) => write!(f, ".{}", name),
        PathNode::Parent(idx) => write!(f, "#{}", idx),
        PathNode::Index(idx) => write!(f, "[{}]", idx),
        PathNode::Deref => write!(f, ".*"),
        }
    }
}
enum PathNode<'a> {
    Null,
    Field(&'a str),
    Parent(usize),
    Index(usize),
    Deref,
}

fn resolve_alias_chain<'a>(debug: &'a debug_info::DebugPool, mut ty: &'a debug_info::Type) -> &'a debug_info::Type {
    while let debug_info::Type::Alias(tr) = ty {
        ty = debug.get_type(tr);
    }
    ty
}
fn get_field(debug: &debug_info::DebugPool, ty: &debug_info::Type, path: &Path) -> (u64, debug_info::TypeRef) {
    println!("get_field: {} in {}", path, debug.fmt_type(ty));
    let (base, ty)  = if let Some(p) = path.parent {
        let (base,ty) = get_field(debug, ty, p);
        (base, debug.get_type(&ty))
    }
    else {
        (0, ty)
    };
    let ty = resolve_alias_chain(debug, ty);
    match ty {
    debug_info::Type::Struct(composite_type) =>
        match path.node {
        PathNode::Field(name) => {
            let Some(f) = composite_type.iter_fields().find(|f| f.name == name) else {
                panic!("Failed to find {:?} in {} ({})", name, composite_type.name(), path);
            };
            (base + f.offset, f.ty)
        },
        PathNode::Parent(index) => {
            let Some((ofs, ty)) = composite_type.parents().nth(index) else {
                panic!("Failed to parent #{} in {} ({})", index, composite_type.name(), path);
            };
            (base + ofs, *ty)
        },
        PathNode::Null|PathNode::Index(_)|PathNode::Deref => panic!("Unexpected path node for `struct` in {}", path),
        },
    debug_info::Type::Union(composite_type) => 
        match path.node {
        PathNode::Field(name) => {
            let f = composite_type.iter_fields().find(|f| f.name == name).unwrap();
            (base + f.offset, f.ty)
        },
        PathNode::Null|
        PathNode::Parent(_)|PathNode::Index(_)|PathNode::Deref => panic!("Unexpected path node for `union`"),
        },
    debug_info::Type::Array(_, _) => todo!("array"),
    debug_info::Type::Primtive(_) => panic!("Getting field of a primitive"),
    debug_info::Type::Pointer(_) => todo!("Pointer"),
    debug_info::Type::Alias(_) => panic!("Alias should be resolved"),
    debug_info::Type::Enum(_) => panic!("Getting field of an enum"),
    }
}

struct StdVector {
    inner_ty: debug_info::TypeRef,
    begin: u64,
    end: u64,
    alloc_end: u64,
}
fn get_std_vector(debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64) -> StdVector {
    let inner_ty = {
        let (_, ty) = get_field(debug, ty, &Path::root().parent(0).field("_M_impl").parent(1).field("_M_start"));
        let ty = debug.get_type(&ty);
        let ty = resolve_alias_chain(debug, ty);
        let debug_info::Type::Pointer(ty) = ty else { panic!("Expected pointer, got {:?}", ty); };
        *ty
        };
    let m_start = dump.read_ptr(addr + 0);
    let m_finish = dump.read_ptr(addr + 8);
    let m_end_of_storage = dump.read_ptr(addr + 16);
    StdVector {
        inner_ty,
        begin: m_start,
        end: m_finish,
        alloc_end: m_end_of_storage,
    }
}

fn visit_type(depth: usize, debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64, path: Path) {
    println!("{:depth$}{ty} @ {addr:#x} ({path})", "", ty=debug.fmt_type(ty));
    match ty {
    debug_info::Type::Alias(ty) => visit_type(depth+1, debug, dump, debug.get_type(ty), addr, path),
    debug_info::Type::Struct(composite_type) => {
        // TODO: Special case some structs
        if composite_type.name() == "std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >" {
            // Get string data, and check for duplicates?
            return ;
        }
        if composite_type.name().starts_with("std::unique_ptr<") {
            let (o, ptr_ty) = get_field(debug, ty, &Path::root().field("_M_t").parent(0).field("_M_t").parent(0).parent(1).field("_M_head_impl"));
            let ptr_ty = resolve_alias_chain(debug, debug.get_type(&ptr_ty));
            let ptr = dump.read_ptr(addr + o);
            let debug_info::Type::Pointer(inner_ty) = ptr_ty else { panic!("Expected pointer") };
            let inner_ty = resolve_alias_chain(debug, debug.get_type(&inner_ty));
            if ptr != 0 {
                visit_type(depth, debug, dump, inner_ty, ptr, path.deref());
            }
            return ;
        }
        if composite_type.name().starts_with("std::vector<") {
            let v = get_std_vector(debug, dump, ty, addr);
            println!("VECTOR: {} {:#x}--{:#x}--{:#x}: `{}`", composite_type.name(), v.begin, v.end, v.alloc_end, debug.fmt_type_ref(&v.inner_ty));
            let inner_ty = debug.get_type(&v.inner_ty);
            let inner_size = debug.size_of(inner_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                visit_type(depth+1, debug, dump, inner_ty, a, path.index(i));
            }
            return ;
        }
        if composite_type.name().starts_with("std::map<") {
            println!("MAP: @{:#x}: TODO", addr);
            if false {
                print!("MAP: "); dump_type_fields(debug, ty, 0); println!("");
            }
            let item_type = debug.get_type(&composite_type.sub_types["value_type"]);
            // Get the inner (not type-erased) node type
            let (_, rb_ty) = get_field(debug, ty, &Path::root().field("_M_t"));
            let rb_ty = resolve_alias_chain(debug, debug.get_type(&rb_ty));
            let debug_info::Type::Struct(ct) = rb_ty else { panic!("RB Tree not a struct/class") };
            let node_type = resolve_alias_chain(debug, debug.get_type(&ct.sub_types["_Link_type"]));
            let debug_info::Type::Pointer(node_type) = node_type else { panic!("Expected pointer, got {:?}", node_type)};
            let node_type = resolve_alias_chain(debug, debug.get_type(node_type));
            println!("> Item type: {}", debug.fmt_type(&item_type));
            println!("> Node type: {}", debug.fmt_type(&node_type));
            print!("MAP NODE: "); dump_type_fields(debug, node_type, 0); println!("");

            struct Node {
                addr: u64,
                left_addr: u64,
                parent_addr: u64,
                right_addr: u64,
            }
            impl Node {
                fn read(dump: &core_dump::CoreDump, addr: u64) -> Node {
                    if addr == 0 {
                        return Node { addr, left_addr: 0, parent_addr: 0, right_addr: 0 };
                    }
                    Node {
                        addr,
                        parent_addr: dump.read_ptr(addr + 0x8),
                        left_addr: dump.read_ptr(addr + 0x10),
                        right_addr: dump.read_ptr(addr + 0x18),
                    }
                }
                fn is_nil(&self) -> bool {
                    self.addr == 0
                }
            }
            let node_count = dump.read_ptr(addr + 0x28);
            println!("> node_count={node_count}");
            if node_count > 0 {
                // Read the root node
                let mut cur_n = Node::read(dump, addr + 8);
                // Traverse into the first LHS node
                cur_n = Node::read(dump, cur_n.left_addr);
                while !cur_n.is_nil() {
                    // Visit inner
                    println!("> VISIT {:#x}: {}", cur_n.addr + 0x20, debug.fmt_type(item_type));

                    // Increment iterator (See `_Rb_tree_increment` implementtion)
                    if cur_n.right_addr != 0 {
                        // Iterate into the RHS until no more LHS
                        cur_n = Node::read(dump, cur_n.right_addr);
                        while cur_n.left_addr != 0 {
                            cur_n = Node::read(dump, cur_n.left_addr);
                        }
                    }
                    else {
                        let mut p = Node::read(dump, cur_n.parent_addr);
                        while cur_n.addr == p.right_addr {
                            let pa = p.parent_addr;
                            cur_n = p;
                            p = Node::read(dump, pa);
                        }
                        if cur_n.right_addr != p.addr {
                            cur_n = p;
                        }
                    }
                }
            }
            return ;
        }
        if composite_type.name().starts_with("std::unordered_map<") {
            println!("UNORDERED MAP: @{:#x}: TODO", addr);
            //print!("UNORDERED MAP: "); dump_type_fields(debug, ty, 0); println!("");
            //todo!("unordered_map");
            // TODO: Same problem as `std::map`, there's no contained type in here
            // _M_buckets: @0x0: *=*=::std::__detail::struct _Hash_node_base,
            // _M_bucket_count: @0x8: prim64,
            // _M_before_begin._M_nxt: @0x10: *::std::__detail::struct _Hash_node_base,
            // _M_element_count: @0x18: prim64,
            // _M_rehash_policy._M_max_load_factor: @0x20: prim32,
            // _M_rehash_policy._M_next_resize: @0x28: prim64,
            // _M_single_bucket: @0x30: *=::std::__detail::struct _Hash_node_base,
            return ;
        }
        for (i,(ofs,ty)) in composite_type.parents().enumerate() {
            visit_type(depth+1, debug, dump, &debug.get_type(ty), addr + ofs, path.parent(i));
        }
        for f in composite_type.iter_fields() {
            visit_type(depth+1, debug, dump, &debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
        }
    },
    debug_info::Type::Union(composite_type) => {
        todo!("Found union, needs handling: {:?}", composite_type.name());
    },
    debug_info::Type::Array(..) => todo!("visit_type: array"),
    debug_info::Type::Enum(_) => {},
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty) => {
        let addr = dump.read_ptr(addr);
        println!("{:depth$}->{:#x}", "", addr);
        if addr != 0 {
            if false {
                visit_type(depth+1, debug, dump, &debug.get_type(dst_ty), addr, path.deref());
            }
        }
    },
    }
}
