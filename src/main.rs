//! 
//! Load a core dump and enumerate memory usage
//! 
//! Outputs:
//! - Structure instance counts
//! - String duplication
//! - Memory usage by structure
//! - Memory for each variable (or member of a struct, to a limited depth)
//! - Memory fragmentation
//! 
mod core_dump;
mod debug_info;

mod visit_helpers;
use visit_helpers::{Path,resolve_alias_chain};
mod type_handlers;

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

/// Dump the immediate fields of a structure (all direct data)
fn dump_type_fields(debug: &debug_info::DebugPool, ty: &debug_info::Type, ofs: u64) {
    match ty {
    debug_info::Type::Alias(ty) => dump_type_fields(debug, debug.get_type(ty), ofs),
    debug_info::Type::Struct(composite_type) => {
        print!("{} {{", composite_type.name());
        for (name,ty) in &composite_type.sub_types {
            print!(" type {} = {};", name, debug.fmt_type_ref(ty));
        }
        for (o,ty) in composite_type.parents() {
            print!(" ");
            dump_type_fields(debug, &debug.get_type(ty), ofs + o);
            print!(";");
        }
        for f in composite_type.iter_fields() {
            print!(" {}: ", f.name);
            dump_type_fields(debug, &debug.get_type(&f.ty), ofs+f.offset);
            print!(",");
        }
        print!(" }}");
        },
    debug_info::Type::Union(u) => {
        print!("union {} {{", u.name());
        for f in u.iter_fields() {
            print!(" {}: ", f.name);
            dump_type_fields(debug, &debug.get_type(&f.ty), ofs+f.offset);
            print!(",");
        }
        print!(" }}");
    },
    _ => print!("@{ofs:#x}: {}", debug.fmt_type(ty)),
    }
}

fn visit_type(depth: usize, debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64, path: Path) {
    // TODO: if the last entry in the path is a deref, or is the root - then get the direct size of this type and add to total used
    let ty = resolve_alias_chain(debug, ty);
    println!("{:depth$}{ty} @ {addr:#x} ({path})", "", ty=debug.fmt_type(ty));
    match ty {
    debug_info::Type::Alias(_) => panic!("Should be resolved above"),
    debug_info::Type::Struct(composite_type) => {

        if composite_type.name() == "std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >" {
            // TODO: Get string data, and check for duplicates?
            return ;
        }
        if let Some(p) = type_handlers::CppUniquePtr::opt_read(debug, dump, ty, addr) {
            if p.target_addr != 0 {
                visit_type(depth+1, debug, dump, p.target_ty, p.target_addr, path.deref());
            }
            return ;
        }
        if let Some(p) = type_handlers::CppSharedPtr::opt_read(debug, dump, ty, addr) {
            if p.target_addr != 0 {
                visit_type(depth+1, debug, dump, p.target_ty, p.target_addr, path.field("data").deref());
            }
            if p.count_addr != 0 {
                visit_type(depth+1, debug, dump, p.count_ty, p.count_addr, path.field("refcount").deref());
            }
            return ;
        }
        if let Some(v) = type_handlers::CppVector::opt_read(debug, dump, ty, addr) {
            let inner_size = debug.size_of(v.item_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                visit_type(depth+1, debug, dump, v.item_ty, a, path.index(i));
            }
            return ;
        }
        if let Some(m) = type_handlers::CppMap::opt_read(debug, dump, ty, addr) {
            let mut n = m.cur_node;
            let mut i = 0;
            while !n.is_nil()
            {
                visit_type(depth+1, debug, dump, m.item_type, n.data_addr(), path.index(i));
                n = n.next(dump);
                i += 1;
            }
            return ;
        }
        if let Some(m) = type_handlers::CppUnorderedMap::opt_read(debug, dump, ty, addr) {
            let mut n = m.first_node;
            let mut i = 0;
            while !n.is_nil()
            {
                visit_type(depth+1, debug, dump, m.item_type, n.data_addr(), path.index(i));
                n = n.next(dump);
                i += 1;
            }
            return ;
        }

        if let Some(tu) = type_handlers::MrustcTaggedUnion::opt_read(debug, dump, ty, addr) {
            if false {
                print!("TU: "); dump_type_fields(debug, ty, 0); println!("");
            }
            if let Some((name,ty)) = tu.variant {
                visit_type(depth+1, debug, dump, ty, addr + tu.data_ofs, path.field(name));
            }
            for f in tu.other_fields {
                visit_type(depth+1, debug, dump, &debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
            }
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
