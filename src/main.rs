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
    debug.index_types();
    
    let state_in_dump = dump.get_thread(0);
    println!("STATE: {}", state_in_dump);
    let state_main = debug.get_caller(&state_in_dump, &dump);
    println!("STATE: {}", state_main);

    let (addr, ty) = debug.get_variable(&state_main, &dump, "crate");
    let mut output = Output {
        debug: &debug,
        dump: &dump,
        usage: Default::default(),
        used_memory: Default::default(),
    };
    visit_type(&mut output, 0, debug.get_type(&ty), addr, Path::root());
    eprintln!("{:#?}", output.usage);
    eprintln!("{} KiB covered (out of {} KiB)", output.used_memory.calculate_usage() / 1024, dump.anon_size() / 1024)
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

#[derive(Default)]
struct SparseBitmap {
    /// [16] bytes per bit, 1024 entries (8KiB) per chunk = 1024*64*16 (1MiB) covered per chunk
    chunks: ::std::collections::BTreeMap<u64, Vec<u64>>,
}
impl SparseBitmap {
    fn mark_area(&mut self, base: u64, len: u64) {
        /// 16 byte coverage calculation atom
        const COVERAGE_PER_BIT: usize = 16;
        /// 1024 units per `Vec<u64>` (for 64k units)
        const CHUNK_SIZE_ENTS: usize = 1024;
        const CHUNK_SIZE_BITS: usize = CHUNK_SIZE_ENTS * 64;
        //const CHUNK_COVERAGE_BYTES: usize = CHUNK_SIZE_BITS * COVERAGE_PER_BIT;
        let b0 = base / COVERAGE_PER_BIT as u64;
        let bn = (base + len + (COVERAGE_PER_BIT - 1) as u64) / COVERAGE_PER_BIT as u64;
        for b in b0 .. bn {
            let (ci,bit) = (b / CHUNK_SIZE_BITS as u64, b as usize % CHUNK_SIZE_BITS);
            let c = self.chunks.entry(ci).or_insert_with(|| vec![0; CHUNK_SIZE_ENTS]);
            c[bit / 64] |= 1 << (bit % 64);
        }
    }

    fn calculate_usage(&self) -> usize {
        let mut n_units = 0;
        for c in self.chunks.values() {
            for v in c {
                n_units += v.count_ones() as usize;
            }
        }
        n_units * 16
    }
}

struct Output<'a> {
    debug: &'a debug_info::DebugPool,
    dump: &'a core_dump::CoreDump,
    /// Memory usage associated with various paths through memory (think `du`'s output)
    usage: ::std::collections::BTreeMap<String, u64>,
    // TODO: (sparse) Bitmap of used memory
    used_memory: SparseBitmap,
}
impl Output<'_> {
    /// Annotate the existence of a top-level type at a location (records memory usage)
    fn claim(&mut self, path: &Path, addr: u64, ty: &debug_info::Type) {
        // Get the size of this type
        let size = self.debug.size_of(ty) as u64;
        self.claim_raw(path, addr, size, true);
    }
    fn claim_raw(&mut self, path: &Path, addr: u64, size: u64, assoc: bool) {
        // Mark it as used in the memory map
        self.used_memory.mark_area(addr, size);

        if !assoc {
            return ;
        }

        // Associate the used memory
        if path.len() > 0 {
            *self.usage.entry(String::new()).or_default() += size;
        }
        let mut path = path.get_prefix(3);
        loop {
            *self.usage.entry(format!("{}", path)).or_default() += size;
            if let Some(p) = path.get_parent() {
                path = p;
            }
            else {
                break;
            }
        }
    }
}

fn visit_type(output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path) {
    let ty = resolve_alias_chain(output.debug, ty);

    // Handle virtual types by detecting the presense of a vtable field, then looking up its value
    let ty = if let debug_info::Type::Struct(ct) = ty {
        if ct.fields.len() > 0 && ct.fields[0].name.starts_with("_vptr.") {
            let vptr = output.dump.read_ptr(addr + ct.fields[0].offset);
            if let Some(ty) = output.debug.find_type_by_vtable(vptr) {
                //println!("{:depth$}>>{ty}", "", ty=debug.fmt_type(ty));
                ty
            }
            else {
                println!("FAILED TO FIND VTABLE: {:#x}", vptr);
                ty
            }
        }
        else {
            ty
        }
    }
    else {
        ty
    };
    println!("{:depth$}{ty} @ {addr:#x} ({path})", "", ty=output.debug.fmt_type(ty));
    // if the last entry in the path is a deref, or is the root - then get the direct size of this type and add to total used
    if path.is_root_or_deref() {
        // Get size of this type, and return it (also claim ownership of the memory range)
        output.claim(&path, addr, ty);
    }

    match ty {
    debug_info::Type::Alias(_) => panic!("Should be resolved above"),
    debug_info::Type::Struct(composite_type) => {

        if composite_type.name() == "std::__cxx11::basic_string<char, std::char_traits<char>, std::allocator<char> >" {
            // TODO: Get string data, and check for duplicates?
            //output.claim(path, base, end - base);
            return ;
        }
        if let Some(p) = type_handlers::CppUniquePtr::opt_read(output.debug, output.dump, ty, addr) {
            if p.target_addr != 0 {
                visit_type(output, depth+1, p.target_ty, p.target_addr, path.deref());
            }
            return ;
        }
        if let Some(p) = type_handlers::CppSharedPtr::opt_read(output.debug, output.dump, ty, addr) {
            if p.target_addr != 0 {
                visit_type(output, depth+1, p.target_ty, p.target_addr, path.field("data").deref());
            }
            if p.count_addr != 0 {
                output.claim(&path.field("refcount").deref(), p.count_addr, p.count_ty);
                visit_type(output, depth+1, p.count_ty, p.count_addr, path.field("refcount").deref());
            }
            return ;
        }
        if let Some(v) = type_handlers::CppVector::opt_read(output.debug, output.dump, ty, addr) {
            let inner_size = output.debug.size_of(v.item_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                output.claim(&path.index(i), a, v.item_ty);
                visit_type(output, depth+1, v.item_ty, a, path.index(i));
            }
            return ;
        }
        if let Some(m) = type_handlers::CppMap::opt_read(output.debug, output.dump, ty, addr) {
            let mut n = m.cur_node;
            let mut i = 0;
            while !n.is_nil()
            {
                output.claim(&path.index(i), n.data_addr(), m.item_type);
                visit_type(output, depth+1, m.item_type, n.data_addr(), path.index(i));
                n = n.next(output.dump);
                i += 1;
            }
            return ;
        }
        if let Some(m) = type_handlers::CppUnorderedMap::opt_read(output.debug, output.dump, ty, addr) {
            let mut n = m.first_node;
            let mut i = 0;
            while !n.is_nil()
            {
                output.claim(&path.index(i), n.data_addr(), m.item_type);
                visit_type(output, depth+1, m.item_type, n.data_addr(), path.index(i));
                n = n.next(output.dump);
                i += 1;
            }
            return ;
        }

        if let Some(tu) = type_handlers::MrustcTaggedUnion::opt_read(output.debug, output.dump, ty, addr) {
            if false {
                print!("TU: "); dump_type_fields(output.debug, ty, 0); println!("");
            }
            if let Some((name,ty)) = tu.variant {
                visit_type(output, depth+1, ty, addr + tu.data_ofs, path.field(name));
            }
            for f in tu.other_fields {
                visit_type(output, depth+1, &output.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
            }
            return ;
        }

        fn visit_ct_inner(output: &mut Output, depth: usize, composite_type: &debug_info::CompositeType, addr: u64, path: Path) {
            for (i,(ofs,ty)) in composite_type.parents().enumerate() {
                let debug_info::Type::Struct(ct) = output.debug.get_type(ty) else { panic!("Parent type not a struct"); };
                println!("{:depth$}{ty} @ {addr:#x} ({path})", "", depth=depth+1, addr=addr+ofs, ty=ct.name(), path=path.parent(i));
                visit_ct_inner(output, depth+1, ct, addr + ofs, path.parent(i));
            }
            for f in composite_type.iter_fields() {
                visit_type(output, depth+1, output.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
            }
        }
        visit_ct_inner(output, depth, composite_type, addr, path)
    },
    debug_info::Type::Union(composite_type) => {
        println!("Not recursing into union: {:?}", composite_type.name());
    },
    debug_info::Type::Array(..) => todo!("visit_type: array"),
    debug_info::Type::Enum(_) => {},
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty, _) => {
        let addr = output.dump.read_ptr(addr);
        println!("{:depth$}->{:#x}", "", addr);
        if addr != 0 {
            if false {
                visit_type(output, depth+1, output.debug.get_type(dst_ty), addr, path.deref());
            }
        }
    },
    }
}
