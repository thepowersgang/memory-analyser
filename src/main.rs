mod core_dump;
mod debug_info;

struct CpuState {
}

fn main() {
    let path = "memory_dump-0.dmp";
    let dump = core_dump::CoreDump::open(path.as_ref());
    let mut debug = debug_info::DebugPool::new();
    for (module_path, base) in dump.modules() {
        debug.add_file(&module_path, base);
    }

    let state_in_dump = dump.get_thread(0);
    let state_main = debug.get_caller(&state_in_dump, &dump);

    let (addr, ty) = debug.get_variable(&state_main, &dump, "hir_crate");
    visit_type(&dump, &debug.get_type(&ty), addr);
}

fn visit_type(dump: &core_dump::CoreDump, ty: &debug_info::Type, addr: u64) {
    match ty {
    debug_info::Type::Composite(composite_type) => todo!(),
    debug_info::Type::Primtive(primitive_type) => todo!(),
    debug_info::Type::Pointer(dst_ty) => todo!(),
    }
}
