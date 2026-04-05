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
    visit_type(0, &debug, &dump, &debug.get_type(&ty), addr);
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

struct StdVector {
    inner_ty: debug_info::TypeRef,
    begin: u64,
    end: u64,
    alloc_end: u64,
}
fn get_std_vector(debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, composite_type: &debug_info::CompositeType, addr: u64) -> StdVector {
    let inner_ty = {
        let (_, ty) = composite_type.parents().next().unwrap();
        let ty = debug.get_type(ty);    // vector_base
        let debug_info::Type::Struct(ct) = ty else { panic!("Expected struct, got {:?}", ty); };
        //println!("> {}", ct.name());
        let f = ct.iter_fields().next().unwrap();   // _M_impl
        let ty = debug.get_type(&f.ty);
        let debug_info::Type::Struct(ct) = ty else { panic!("Expected struct, got {:?}", ty); };
        //println!("> {}", ct.name());
        let (_, ty) = ct.parents().nth(1).unwrap();
        let ty = debug.get_type(ty);    // _Vector_impl_data
        let debug_info::Type::Struct(ct) = ty else { panic!("Expected struct, got {:?}", ty); };
        //println!("> {}", ct.name());
        let f = ct.iter_fields().next().unwrap();   // _M_start
        //println!("> {}: {}", f.name, debug.fmt_type_ref(&f.ty));
        let mut ty = debug.get_type(&f.ty);
        let ty = loop {
            ty = match ty {
            debug_info::Type::Alias(ty) => debug.get_type(ty),
            _ => break ty,
            };
        };
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

fn visit_type(depth: usize, debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64) {
    println!("{:w$}{ty} @ {addr:#x}", "", ty=debug.fmt_type(ty), w=depth);
    match ty {
    debug_info::Type::Alias(ty) => visit_type(depth+1, debug, dump, debug.get_type(ty), addr),
    debug_info::Type::Struct(composite_type) => {
        // TODO: Special case some structs
        if composite_type.name() == "::std::__cxx11::struct basic_string<char, std::char_traits<char>, std::allocator<char> >" {
            // Get string data, and check for duplicates?
            return ;
        }
        if composite_type.name().starts_with("::std::struct vector<") {
            let v = get_std_vector(debug, dump, composite_type, addr);
            println!("VECTOR: {} {:#x}--{:#x}--{:#x}: `{}`", composite_type.name(), v.begin, v.end, v.alloc_end, debug.fmt_type_ref(&v.inner_ty));
            let inner_ty = debug.get_type(&v.inner_ty);
            let inner_size = debug.size_of(inner_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for a in (v.begin .. v.end).step_by(inner_size) {
                visit_type(depth+1, debug, dump, inner_ty, a);
            }
            return ;
        }
        if composite_type.name().starts_with("::std::struct map<") {
            println!("MAP: @{:#x}: TODO", addr);
            if false {
                print!("MAP: "); dump_type_fields(debug, ty, 0); println!("");
            }
            //todo!("map");
            // harder :( - Don't have an easy way of getting the inner type. No contained type has it
            // - Need to parse the type name, and find a matching inner type

            // header:
            //_M_color: @0x8: ::std::enum _Rb_tree_color,
            //_M_parent: @0x10: *::std::struct _Rb_tree_node_base,
            //_M_left: @0x18: *::std::struct _Rb_tree_node_base,
            //_M_right: @0x20: *::std::struct _Rb_tree_node_base,
            // meta:
            //_M_node_count: @0x28: prim64,
            return ;
        }
        if composite_type.name().starts_with("::std::struct unordered_map<") {
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
        for (ofs,ty) in composite_type.parents() {
            visit_type(depth+1, debug, dump, &debug.get_type(ty), addr + ofs);
        }
        for f in composite_type.iter_fields() {
            visit_type(depth+1, debug, dump, &debug.get_type(&f.ty), addr + f.offset);
        }
    },
    debug_info::Type::Union(composite_type) => {
        todo!("Found union, needs handling: {:?}", composite_type.name());
    },
    debug_info::Type::Enum(_) => {},
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty) => {
        let addr = dump.read_ptr(addr);
        println!("{:depth$}->{:#x}", "", addr);
        if addr != 0 {
            if false {
                visit_type(depth+1, debug, dump, &debug.get_type(dst_ty), addr);
            }
        }
    },
    }
}
