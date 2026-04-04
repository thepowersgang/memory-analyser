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
        if composite_type.name().starts_with(":std::struct vector<") {
            // Get the item count, and seek through
            return ;
        }
        for f in composite_type.iter_fields() {
            visit_type(depth+1, debug, dump, &debug.get_type(&f.ty), addr + f.offset);
        }
    },
    debug_info::Type::Union(composite_type) => {
        todo!("Found union, needs handling: {:?}", composite_type.name());
    },
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty) => {
        let addr = dump.read_ptr(addr);
        println!("{:0$}->{:#x}", depth, addr);
        if addr != 0 {
            if false {
                visit_type(depth+1, debug, dump, &debug.get_type(dst_ty), addr);
            }
        }
    },
    }
}
