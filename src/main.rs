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
use visit_helpers::Path;
mod type_handlers;

#[derive(Clone)]
struct CpuState {
    // AMD64:
    pc: u64,
    gprs: [u64; 16],
}
impl CpuState {
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
    let path = ::std::env::args().nth(1).expect("pass a core dump");
    let dump = core_dump::CoreDump::open(path.as_ref()).expect("Unable to open core dump");
    let mut debug = debug_info::DebugPool::new();
    for f in dump.modules()
    {
        match debug.add_file(&f.path, f.virt_base, f.file_base)
        {
        Ok(()) => {},
        Err(e) => panic!("Failed to load {:?}: {:?}", f.path, e),
        }
    }
    debug.index_types();
    
    let state_in_dump = dump.get_thread(0);
    println!("STATE: {}", state_in_dump);
    let state_main = debug.get_caller(&state_in_dump, &dump);
    println!("STATE: {}", state_main);

    let (addr, ty) = debug.get_variable(&state_main, &dump, "crate");
    let input = Input { debug: &debug, dump: &dump };
    let mut output = Output::default();
    visit_type(&input, &mut output, 0, debug.get_type(&ty), addr, Path::root());
    eprintln!("enum counts: {:#?}", output.enum_variant_counts);
    eprintln!("top-level type counts: {:#?}", output.root_type_counts);
    eprintln!("annotated usage: {:#?}", output.usage);
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

struct Input<'a> {
    debug: &'a debug_info::DebugPool,
    dump: &'a core_dump::CoreDump,
}
impl<'a> Input<'a> {
    fn resolve_alias_chain_tr(&self, ty: &debug_info::TypeRef) -> &'a debug_info::Type {
        self.resolve_alias_chain(self.debug.get_type(ty))
    }
    fn resolve_alias_chain(&self, ty: &'a debug_info::Type) -> &'a debug_info::Type {
        visit_helpers::resolve_alias_chain(self.debug, ty)
    }

    fn get_field(&self, ty: &'a debug_info::Type, path: visit_helpers::Path) -> (u64, debug_info::TypeRef) {
        visit_helpers::get_field(self.debug, ty, &path)
    }
}
#[derive(Default)]
struct Output {
    /// Memory usage associated with various paths through memory (think `du`'s output)
    usage: ::std::collections::BTreeMap<String, u64>,
    /// A sparse bitmap representing used (visited) memory
    used_memory: SparseBitmap,
    /// Set of seen shared pointers (any sort of shared pointer, not just `std::shared_ptr`)
    shared_pointers: ::std::collections::BTreeSet<u64>,

    /// Type instance counts, only if they're at the top level (i.e. `claim` called with this type)
    // Not really useful? More intersting to see all composite type counts, to find what takes the most space
    root_type_counts: ::std::collections::HashMap<String, usize>,

    /// Number of instances of each enum variants
    enum_variant_counts: ::std::collections::HashMap<String, ::std::collections::HashMap<String, usize>>,
}
impl Output {
    /// Annotate the existence of a top-level type at a location (records memory usage)
    fn claim(&mut self, input: &Input, path: &Path, addr: u64, ty: &debug_info::Type) {
        // Get the size of this type
        let size = input.debug.size_of(ty) as u64;
        self.claim_raw(path, addr, size, true);
        *self.root_type_counts.entry(format!("{}", input.debug.fmt_type(ty))).or_default() += 1;
    }
    fn claim_raw(&mut self, path: &Path, addr: u64, size: u64, assoc: bool) {
        println!("@{} += {}", path, size);
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

fn visit_type(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path) {
    let ty = input.resolve_alias_chain(ty);

    // Handle virtual types by detecting the presense of a vtable field, then looking up its value
    let ty = if let debug_info::Type::Struct(ct) = ty {
        if ct.fields.len() > 0 && ct.fields[0].name.starts_with("_vptr.") {
            let vptr = input.dump.read_ptr(addr + ct.fields[0].offset);
            if let Some(ty) = input.debug.find_type_by_vtable(vptr) {
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
    println!("{:depth$}{ty} @ {addr:#x} ({path})", "", ty=input.debug.fmt_type(ty));
    // if the last entry in the path is a deref, or is the root - then get the direct size of this type and add to total used
    if path.is_root_or_deref() {
        // Get size of this type, and return it (also claim ownership of the memory range)
        output.claim(input, &path, addr, ty);
    }

    match ty {
    debug_info::Type::Alias(_) => panic!("Should be resolved above"),
    debug_info::Type::Struct(composite_type) => {

        fn read_str<'a>(dump: &core_dump::CoreDump, addr: u64, len: u64, buf: &'a mut [u8]) -> (ByteStr<'a>,&'static str) {
            let l = (len as usize).min(buf.len());
            let buf = &mut buf[..l];
            dump.read_bytes(addr, buf);
            (
                ByteStr(buf),
                if l != len as usize { "..." } else { "" }
                )
        }

        if let Some(s) = type_handlers::CppString::opt_read(input, ty, addr) {
            // TODO: Get string data, and check for duplicates?
            if s.ptr != 0 {
                let mut buf = [0; 16];
                let (str,t): (ByteStr<'_>, &str) = read_str(input.dump, s.ptr, s.len, &mut buf);
                println!("{:depth$}{} = {:?}{} (std::string cap={})", "", path.deref(), str, t, s.capacity);
            }
            if s.capacity == 0 {
                // Inline string, so don't claim the memory (buffer already claimed)
            }
            else {
                output.claim_raw(&path.deref(), s.ptr, s.len - s.ptr, true);
            }
            return ;
        }
        if let Some(p) = type_handlers::CppUniquePtr::opt_read(input, ty, addr) {
            if p.target_addr != 0 {
                visit_type(input, output, depth+1, p.target_ty, p.target_addr, path.deref());
            }
            return ;
        }
        if let Some(p) = type_handlers::CppSharedPtr::opt_read(input, ty, addr) {
            if p.target_addr != 0 && output.shared_pointers.insert(p.target_addr) {
                visit_type(input, output, depth+1, p.target_ty, p.target_addr, path.field("data").deref());
            }
            if p.count_addr != 0 && output.shared_pointers.insert(p.count_addr) {
                //dump_type_fields(output.debug, p.count_ty, 0);
                visit_type(input, output, depth+1, p.count_ty, p.count_addr, path.field("refcount").deref());
            }
            return ;
        }
        if let Some(v) = type_handlers::CppVector::opt_read(input, ty, addr) {
            let inner_size = input.debug.size_of(v.item_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                output.claim(input, &path.index(i), a, v.item_ty);
                visit_type(input, output, depth+1, v.item_ty, a, path.index(i));
            }
            return ;
        }
        if let Some(m) = type_handlers::CppMap::opt_read(input, ty, addr) {
            let mut n = m.cur_node;
            let mut i = 0;
            while !n.is_nil()
            {
                output.claim(input, &path.index(i), n.data_addr(), m.item_type);
                visit_type(input, output, depth+1, m.item_type, n.data_addr(), path.index(i));
                n = n.next(input.dump);
                i += 1;
            }
            return ;
        }
        if let Some(m) = type_handlers::CppUnorderedMap::opt_read(input, ty, addr) {
            let mut n = m.first_node;
            let mut i = 0;
            while !n.is_nil()
            {
                output.claim(input, &path.index(i), n.data_addr(), m.item_type);
                visit_type(input, output, depth+1, m.item_type, n.data_addr(), path.index(i));
                n = n.next(input.dump);
                i += 1;
            }
            return ;
        }

        if let Some(tu) = type_handlers::MrustcTaggedUnion::opt_read(input, ty, addr) {
            if false {
                print!("TU: "); dump_type_fields(input.debug, ty, 0); println!("");
            }
            if let Some((name,ty)) = tu.variant {
                *output.enum_variant_counts
                    .entry(composite_type.name().to_owned()).or_default()
                    .entry(name.to_owned()).or_default()
                    += 1;
                visit_type(input, output, depth+1, ty, addr + tu.data_ofs, path.field(name));
            }
            for f in tu.other_fields {
                visit_type(input, output, depth+1, &input.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
            }
            return ;
        }

        if let Some(v) = type_handlers::MrustcRcString::opt_read(input, ty, addr) {
            let mut buf = [0; 16];
            let (s,t) = if v.string_len > 0 {
                read_str(input.dump, v.string_ptr, v.string_len, &mut buf)
            } else {
                (ByteStr(b""),"")
            };
            println!("{:depth$}{} = {:?}{} (RcString a={:#x})", "", path.deref(), s, t, v.data_addr);
            if v.data_addr != 0 && output.shared_pointers.insert(v.data_addr) {
                output.claim_raw(&path, v.data_addr, input.debug.size_of(ty) as u64 + v.string_len, false);
            }
            return ;
        }

        fn rc_ptr(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path, ptr_path: Path) {
            let (ptr_o,ptr_ty) = input.get_field(ty, ptr_path);
            let ptr_t = input.debug.get_type(&ptr_ty);
            let debug_info::Type::Pointer(inner_ty, _) = ptr_t else { panic!("Expected pointer, got  {:?}", ptr_t); };
            let inner_ty = input.debug.get_type(inner_ty);
            let ptr_val = input.dump.read_ptr(addr + ptr_o);
            if ptr_val != 0 && output.shared_pointers.insert(ptr_val) {
                visit_type(input, output, depth+1, inner_ty, ptr_val, path.deref());
            }
        }
        fn unique_ptr(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path, ptr_path: Path) {
            let (ptr_o,ptr_ty) = input.get_field(ty, ptr_path);
            let ptr_t = input.debug.get_type(&ptr_ty);
            let debug_info::Type::Pointer(inner_ty, _) = ptr_t else { panic!("Expected pointer, got  {:?}", ptr_t); };
            let inner_ty = input.debug.get_type(inner_ty);
            let ptr_val = input.dump.read_ptr(addr + ptr_o);
            if ptr_val != 0 {
                visit_type(input, output, depth+1, inner_ty, ptr_val, path.deref());
            }
        }
        // Some mrustc special types
        match composite_type.name() {
        // interned string type, ignore for now (TODO: Look for duplicates?)
        "RcString" => return,
        // `Span`: A fixed-size reference-counted type
        "Span" => return rc_ptr(input, output, depth, ty, addr, path, Path::root().field("m_ptr")),
        // --- Various fixed-size smart pointers ---
        | "MIR::FunctionPointer"
        | "HIR::ExprStatePtr"
        | "HIR::ExprPtrInner"
            => return unique_ptr(input, output, depth, ty, addr, path, Path::root().field("ptr")),
        | "EncodedLiteralPtr"
        | "MIR::EnumCachePtr"
            => return unique_ptr(input, output, depth, ty, addr, path, Path::root().field("p")),
        | "MacroRulesPtr"
        | "HIR::CratePtr"
        | "AST::ExprNodeP"
            => return unique_ptr(input, output, depth, ty, addr, path, Path::root().field("m_ptr")),
        _ => {},
        }

        fn visit_ct_inner(input: &Input, output: &mut Output, depth: usize, composite_type: &debug_info::CompositeType, addr: u64, path: Path) {
            for (i,(ofs,ty)) in composite_type.parents().enumerate() {
                let debug_info::Type::Struct(ct) = input.debug.get_type(ty) else { panic!("Parent type not a struct"); };
                println!("{:depth$}{ty} @ {addr:#x} ({path})", "", depth=depth+1, addr=addr+ofs, ty=ct.name(), path=path.parent(i));
                visit_ct_inner(input, output, depth+1, ct, addr + ofs, path.parent(i));
            }
            for f in composite_type.iter_fields() {
                if f.name.starts_with("_vptr.") {
                    // Skip VTable pointers
                    continue ;
                }
                if composite_type.name().starts_with("std::_Sp_counted_ptr<") && f.name == "_M_ptr" {
                    // Skip the data pointer stored in shared_ptr's count (avoids even attempting to double-visit)
                    continue;
                }
                visit_type(input, output, depth+1, input.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name));
            }
        }
        visit_ct_inner(input, output, depth, composite_type, addr, path)
    },
    debug_info::Type::Union(composite_type) => {
        println!("Not recursing into union: {:?}", composite_type.name());
    },
    debug_info::Type::Array(..) => todo!("visit_type: array"),
    debug_info::Type::Enum(_) => {},
    debug_info::Type::Primtive(_) => {},
    debug_info::Type::Pointer(dst_ty, _) => {
        let addr = input.dump.read_ptr(addr);
        println!("{:depth$}->{:#x}", "", addr);
        if addr != 0 {
            if false {
                visit_type(input, output, depth+1, input.debug.get_type(dst_ty), addr, path.deref());
            }
        }
    },
    }
}


struct ByteStr<'a>(&'a[u8]);
impl ::std::fmt::Debug for ByteStr<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"")?;
        for &b in self.0 {
            match b {
            0 => f.write_str("\\0"),
            b'\t' => f.write_str("\\t"),
            b'\n' => f.write_str("\\n"),
            b'\r' => f.write_str("\\r"),
            b'"' => f.write_str("\\\""),
            0x20..0x7F => write!(f, "{}", b as char),
            _ => write!(f, "\\x{:02X}", b),
            }?
        }
        f.write_str("\"")
    }
}
