mod core_dump;
mod debug_info;

struct CpuState {
}
impl CpuState {
    fn get_pc(&self) -> u64 {
        todo!("get_pc")
    }
}

fn main() {
    let path = "memory_dump-0.dmp";
    let dump = if true {
        core_dump::CoreDump::new_stub()
    }
    else {
        core_dump::CoreDump::open(path.as_ref()).expect("Unable to open core dump")
    };
    let mut debug = debug_info::DebugPool::new();
    for (module_path, base) in dump.modules()
    {
        match debug.add_file(&module_path, base)
        {
        Ok(()) => {},
        Err(e) => panic!("Failed to load {:?}: {:?}", module_path, e),
        }
    }

    let state_in_dump = dump.get_thread(0);
    let state_main = debug.get_caller(&state_in_dump, &dump);

    let (addr, ty) = debug.get_variable(&state_main, &dump, "hir_crate");
    visit_type(&debug, &dump, &debug.get_type(&ty), addr);
}

fn visit_type(debug: &debug_info::DebugPool, dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64) {
    match ty {
    debug_info::Type::Struct(composite_type) => {
        for f in composite_type.iter_fields() {
            visit_type(debug, dump, &debug.get_type(&f.ty), addr + f.offset);
        }
    },
    debug_info::Type::Union(composite_type) => {
        todo!("Found union, needs handling: {:?}", composite_type.name());
    },
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty) => {
        let addr = dump.read_ptr(addr);
        visit_type(debug, dump, &debug.get_type(dst_ty), addr);
    },
    }
}
