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
    gp_registers: [u64; 16],
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
        for (i,(n,v)) in GPR_NAMES.iter().zip(self.gp_registers.iter()).enumerate() {
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
    struct Args {
        path: ::std::path::PathBuf,
        out_file: Option<::std::path::PathBuf>,
        variables: Vec<Variable>,
    }
    struct Variable {
        fcn_name: String,
        var_name: String,
        visited: bool,
    }
    impl Variable {
        pub fn new(fcn: &str, var: &str) -> Self {
            Self { fcn_name: fcn.to_owned(), var_name: var.to_owned(), visited: false }
        }
    }
    let mut args = {
        let mut it = ::std::env::args();
        it.next();   // Executable name
        let mut path = None;
        let mut out_file = None;
        let mut variables = Vec::new();
        while let Some(a) = it.next() {
            if a.starts_with("-") {
                match &a[..] {
                "--output" => {
                    out_file = Some(it.next().unwrap().into());
                    }
                _ => panic!("Unexpected argument"),
                }
            }
            else if path.is_none() {
                path = Some(a.into());
                continue;
            }
            else {
                let (fcn, var) = a.split_once("/").expect("Variable names must be of format `<fcn>/<var>`");
                variables.push(Variable::new(fcn, var));
            }
        }
        // TODO: Error if nothing passed?
        if variables.is_empty() {
            variables.push(Variable::new("main","crate"));
        }
        Args {
            out_file,
            path: path.expect("No dump file passed"),
            variables,
        }
    };
    // Open the dump
    let dump = core_dump::CoreDump::open(args.path.as_ref()).expect("Unable to open core dump");
    // Load debug information for referenced modules
    let mut debug = debug_info::DebugPool::new();
    for f in dump.modules()
    {
        match debug.add_file(&f.path, f.load_base, f.file_base)
        {
        Ok(()) => {},
        Err(e) => panic!("Failed to load {:?}: {:?}", f.path, e),
        }
    }
    debug.index_types();

    let input = Input { debug: &debug, dump: &dump };
    let mut output = Output::default();
    progress::set_total(input.dump.anon_size() as u64);

    // Only consider thread 0
    let mut state = dump.get_thread(0).clone();
    loop {
        let sym = debug.resolve_symbol(state.pc);
        println!("STATE: @{:x?} {}", sym, state);
        if let Some((name,_)) = sym {
            for v in args.variables.iter_mut() {
                if v.fcn_name == name {
                    let (addr, ty) = debug.get_variable(&state, &dump, &v.var_name);
                    match addr
                    {
                    debug_info::VariableLocation::IntegerRegister(_) => todo!(),
                    debug_info::VariableLocation::Memory(addr) =>
                        match visit_type(&input, &mut output, 0, debug.get_type(&ty), addr, Path::root().field(&v.var_name))
                        {
                        Ok(()) => {},
                        Err(()) => {},
                        },
                    }
                    v.visited = true;
                }
            }
            if args.variables.iter().all(|v| v.visited) {
                break
            }
        }
        if let Some(ns) = debug.get_caller(&state, &dump) {
            state = ns;
        }
        else {
            break;
        }
    }
    eprintln!("");
    for v in args.variables.iter() {
        if !v.visited {
            eprintln!("Failed to find function for {} / {}", v.fcn_name, v.var_name);
            eprintln!("> {:#x?}", debug.get_symbol(&v.fcn_name));
        }
    }

    fn write_output(dst: &mut dyn ::std::io::Write, dump: &core_dump::CoreDump, output: &Output) -> ::std::io::Result<()> {
        writeln!(dst, "enum counts: {{")?;
        for (t,vals) in {
            let mut v: Vec<_> = output.enum_variant_counts.iter()
                .map(|(k,v)| (k, v.iter().collect::<Vec<_>>()))
                .collect();
            v.sort_by_key(|(k,v)| (v.iter().map(|(_,b)| *b).sum::<usize>(),&k[..]));
            v.iter_mut().for_each(|(_,v)| v.sort_by_key(|(k,v)| (*v,&k[..])));
            v
        }
        {
            writeln!(dst, "  {t:?}: {{")?;
            for (k,v) in vals {
                writeln!(dst, "    {k:?}: {v},")?;
            }
            writeln!(dst, "  }}")?;
        }
        writeln!(dst, "}}")?;

        writeln!(dst, "top-level type counts: {{")?;
        for (k,v) in {
            let mut v: Vec<_> = output.root_type_counts.iter().collect();
            v.sort_by_key(|(k,v)| (*v,&k[..]));
            v
        }
        {
            writeln!(dst, "  {k:?}: {v},")?;
        }
        writeln!(dst, "}}")?;

        writeln!(dst, "annotated usage: {:#?}", output.usage)?;
        // TODO: Present this sorted by size on each sub-tree

        writeln!(dst, "{} MiB covered (out of {} MiB)",
            output.used_memory.calculate_usage().div_ceil(1024) as f64 / 1024.,
            dump.anon_size().div_ceil(1024) as f64 / 1024.,
        )?;
        Ok(())
    }
    match args.out_file {
        Some(p) => write_output(&mut ::std::fs::File::create(p).unwrap(), &dump, &output).expect("Error on output"),
        None => write_output(&mut ::std::io::stderr().lock(), &dump, &output).expect("Error on output"),
    }
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
    debug_info::Type::TaggedUnion(e) => {
        print!("enum {} {{", e.outer.name());
        for f in e.outer.iter_fields() {
            print!(" {}: ", f.name);
            dump_type_fields(debug, &debug.get_type(&f.ty), ofs+f.offset);
            print!(",");
        }
        for (i,v) in e.variants.iter().enumerate() {
            print!(" #{i}: {{");
            for f in v.fields.iter() {
                print!(" {}: ", f.name);
                dump_type_fields(debug, &debug.get_type(&f.ty), ofs+f.offset);
                print!(",");
            }
            print!("}},")
        }
        print!(" }}");
    }
    _ => print!("@{ofs:#x}: {}", debug.fmt_type(ty)),
    }
}

#[derive(Default)]
struct SparseBitmap {
    /// [16] bytes per bit, 1024 entries (8KiB) per chunk = 1024*64*16 (1MiB) covered per chunk
    chunks: ::std::collections::BTreeMap<u64, Vec<u64>>,
}
impl SparseBitmap {
    fn mark_area(&mut self, base: u64, len: u64) -> usize {
        /// 16 byte coverage calculation atom
        const COVERAGE_PER_BIT: usize = 16;
        /// 1024 units per `Vec<u64>` (for 64k units)
        const CHUNK_SIZE_ENTS: usize = 1024;
        const CHUNK_SIZE_BITS: usize = CHUNK_SIZE_ENTS * 64;
        //const CHUNK_COVERAGE_BYTES: usize = CHUNK_SIZE_BITS * COVERAGE_PER_BIT;
        let b0 = base / COVERAGE_PER_BIT as u64;
        let bn = (base + len + (COVERAGE_PER_BIT - 1) as u64) / COVERAGE_PER_BIT as u64;
        let mut n_set = 0;
        for b in b0 .. bn {
            let (ci,bit) = (b / CHUNK_SIZE_BITS as u64, b as usize % CHUNK_SIZE_BITS);
            let c = self.chunks.entry(ci).or_insert_with(|| vec![0; CHUNK_SIZE_ENTS]);
            let m = 1 << (bit % 64);
            if c[bit / 64] & m == 0 {
                n_set += 1;
                c[bit / 64] |= m;
            }
        }
        n_set * COVERAGE_PER_BIT
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
    // Not really useful? More interesting to see all composite type counts, to find what takes the most space
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
        progress::add_mem( self.used_memory.mark_area(addr, size) );

        if !assoc {
            return ;
        }

        // Associate the used memory
        if path.len() > 0 {
            *self.usage.entry(String::new()).or_default() += size;
        }
        let mut path = path.get_prefix(4);
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

fn visit_type(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path) -> Result<(),core_dump::ReadError> {
    let ty = input.resolve_alias_chain(ty);

    // Handle virtual types by detecting the presence of a vtable field, then looking up its value
    let ty = if let debug_info::Type::Struct(ct) = ty {
        if ct.fields.len() > 0 && ct.fields[0].name.starts_with("_vptr.") { // cspell:disable-line
            let v_ptr = input.dump.read_ptr(addr + ct.fields[0].offset)?;
            if let Some(ty) = input.debug.find_type_by_vtable(v_ptr) {
                //println!("{:depth$}>>{ty}", "", ty=debug.fmt_type(ty));
                ty
            }
            else {
                println!("FAILED TO FIND VTABLE: {:#x}", v_ptr);
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

        fn read_str<'a>(dump: &core_dump::CoreDump, addr: u64, len: u64, buf: &'a mut [u8]) -> Result<(ByteStr<'a>,&'static str),core_dump::ReadError> {
            let l = (len as usize).min(buf.len());
            let buf = &mut buf[..l];
            dump.read_bytes(addr, buf)?;
            Ok((
                ByteStr(buf),
                if l != len as usize { "..." } else { "" }
                ))
        }

        // --- C++ Standard Template Library ---
        if let Some(s) = type_handlers::CppString::opt_read(input, ty, addr)? {
            assert!(s.capacity == 0 || s.len <= s.capacity, "Malformed std::string - {:#x}+{:#x}(cap={:#x})", s.ptr, s.len, s.capacity);
            assert!(s.capacity < 0x10_00000);   // 16MiB
            // TODO: Get string data, and check for duplicates?
            if s.ptr != 0 {
                let mut buf = [0; 16];
                let (str,t): (ByteStr<'_>, &str) = read_str(input.dump, s.ptr, s.len, &mut buf)?;
                println!("{:depth$}{} = {:?}{} (std::string cap={})", "", path.deref(), str, t, s.capacity);
            }
            if s.capacity == 0 {
                // Inline string, so don't claim the memory (buffer already claimed)
            }
            else {
                output.claim_raw(&path.deref(), s.ptr, s.len, true);
            }
            return Ok(());
        }
        if let Some(p) = type_handlers::CppUniquePtr::opt_read(input, ty, addr)? {
            println!("{:depth$}->{:#x}", "", p.target_addr);
            if p.target_addr != 0 {
                // NOTE: being a c++ smart pointer, this might be invalid - ignore errors
                let _ = visit_type(input, output, depth+1, p.target_ty, p.target_addr, path.deref())?;
            }
            return Ok(());
        }
        if let Some(p) = type_handlers::CppSharedPtr::opt_read(input, ty, addr)? {
            println!("{:depth$}->{:#x}, c={:#x}", "", p.target_addr, p.count_addr);
            // NOTE: being a c++ smart pointer, this might be invalid - ignore errors
            if p.target_addr != 0 && output.shared_pointers.insert(p.target_addr) {
                let _ = visit_type(input, output, depth+1, p.target_ty, p.target_addr, path.field("data").deref())?;
            }
            if p.count_addr != 0 && output.shared_pointers.insert(p.count_addr) {
                //dump_type_fields(output.debug, p.count_ty, 0);
                let _ = visit_type(input, output, depth+1, p.count_ty, p.count_addr, path.field("refcount").deref())?;
            }
            return Ok(());
        }
        if composite_type.name().starts_with("std::vector<bool") {
            // TODO: Claim ownership of pointed-to memory
            return Ok(());
        }
        if let Some(v) = type_handlers::CppVector::opt_read(input, ty, addr)? {
            let inner_size = input.debug.size_of(v.item_ty);
            println!("{:depth$}->{:#x}+{:#x}(+{:#x} s={inner_size:#x})", "", v.begin, v.end - v.begin, v.alloc_end - v.begin);
            if !(v.begin <= v.end && v.end <= v.alloc_end) {
                eprintln!("Malformed std::vector: {:#x} <= {:#x} <= {:#x}", v.begin, v.end, v.alloc_end);
                // TODO: Error?
                return Ok(());
            }
            let mut p = ProgressTracker::new((v.end - v.begin) as usize / inner_size);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                p.update(i, path.index(i));
                output.claim(input, &path.index(i), a, v.item_ty);
                visit_type(input, output, depth+1, v.item_ty, a, path.index(i))?;
            }
            return Ok(());
        }
        if let Some(m) = type_handlers::CppMap::opt_read(input, ty, addr)? {
            let mut n = m.cur_node;
            let mut p = ProgressTracker::new(m.node_count as usize);
            for i in 0 .. m.node_count
            {
                p.update(i as usize, path.index(i as usize));
                assert!(!n.is_nil());
                output.claim(input, &path.index(i as usize), n.data_addr(), m.item_type);
                visit_type(input, output, depth+1, m.item_type, n.data_addr(), path.index(i as usize))?;
                n = n.next(input.dump)?;
            }
            return Ok(());
        }
        if let Some(m) = type_handlers::CppUnorderedMap::opt_read(input, ty, addr)? {
            let mut n = m.first_node;
            let mut i = 0;
            let mut p = ProgressTracker::new(m.element_count as usize);
            while !n.is_nil()
            {
                p.update(i, path.index(i));
                output.claim(input, &path.index(i), n.data_addr(), m.item_type);
                visit_type(input, output, depth+1, m.item_type, n.data_addr(), path.index(i))?;
                n = n.next(input.dump)?;
                i += 1;
            }
            return Ok(());
        }

        // --- rust standard library ---
        if let Some(s) = type_handlers::rust::AllocString::opt_read(input, ty, addr)? {
            // TODO: Get string data, and check for duplicates?
            if s.ptr != 0 {
                let mut buf = [0; 16];
                let (str,t): (ByteStr<'_>, &str) = read_str(input.dump, s.ptr, s.len, &mut buf)?;
                println!("{:depth$}{} = {:?}{} (alloc String cap={})", "", path.deref(), str, t, s.cap);
            }
            if s.ptr != 0 {
                output.claim_raw(&path.deref(), s.ptr, s.len, true);
            }
            return Ok(());
        }
        if let Some(v) = type_handlers::rust::AllocVec::opt_read(input, ty, addr)? {
            let inner_size = input.debug.size_of(v.item_ty);
            assert!(v.begin <= v.end && v.end <= v.alloc_end);
            for (i,a) in (v.begin .. v.end).step_by(inner_size).enumerate() {
                output.claim(input, &path.index(i), a, v.item_ty);
                visit_type(input, output, depth+1, v.item_ty, a, path.index(i))?;
            }
            return Ok(());
        }
        if let Some(mut m) = type_handlers::rust::HashbrownMap::opt_read(input, ty, addr)? {
            // TODO: Claim the entire allocation?
            let mut i = 0;
            while let Some(a) = m.next(input)? {
                output.claim(input, &path.index(i), a, m.item_ty);
                visit_type(input, output, depth+1, m.item_ty, a, path.index(i))?;
                i += 1;
            }
            return Ok(());
        }
        if let Some(p) = type_handlers::rust::AllocRc::opt_read(input, ty, addr)? {
            visit_type(input, output, depth+1, p.inner_ty, p.addr, path.deref())?;
            return Ok(());
        }

        // --- MRustC helper types ---
        if let Some(tu) = type_handlers::mrustc::TaggedUnion::opt_read(input, ty, addr)? {
            if false {
                print!("TU: "); dump_type_fields(input.debug, ty, 0); println!("");
            }
            if let Some((name,ty)) = tu.variant {
                *output.enum_variant_counts
                    .entry(composite_type.name().to_owned()).or_default()
                    .entry(name.to_owned()).or_default()
                    += 1;
                visit_type(input, output, depth+1, ty, addr + tu.data_ofs, path.field(name))?;
            }
            for f in tu.other_fields {
                visit_type(input, output, depth+1, &input.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name))?;
            }
            return Ok(());
        }
        if let Some(v) = type_handlers::mrustc::ThinVector::opt_read(input, ty, addr)? {
            let inner_size = input.debug.size_of(v.inner_ty);
            let mut p = ProgressTracker::new(v.len as usize);
            output.claim_raw(&path, v.data_ptr, v.cap * inner_size as u64, false);
            for i in 0 .. v.len as usize {
                p.update(i, path.index(i));
                visit_type(input, output, depth, v.inner_ty, v.data_ptr + (i * inner_size) as u64, path.index(i))?;
            }
            return Ok(());
        }
        if let Some(v) = type_handlers::mrustc::RcString::opt_read(input, ty, addr)? {
            let mut buf = [0; 16];
            let (s,t) = if v.string_len > 0 {
                read_str(input.dump, v.string_ptr, v.string_len, &mut buf)?
            } else {
                (ByteStr(b""),"")
            };
            println!("{:depth$}{} = {:?}{} (RcString a={:#x})", "", path.deref(), s, t, v.data_addr);
            if v.data_addr != 0 && output.shared_pointers.insert(v.data_addr) {
                output.claim_raw(&path, v.data_addr, input.debug.size_of(ty) as u64 + v.string_len, false);
            }
            return Ok(());
        }

        fn rc_ptr(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path, ptr_path: Path) -> Result<(),core_dump::ReadError> {
            let (ptr_o,ptr_ty) = input.get_field(ty, ptr_path);
            let ptr_t = input.debug.get_type(&ptr_ty);
            let debug_info::Type::Pointer(inner_ty, ..) = ptr_t else { panic!("Expected pointer, got  {:?}", ptr_t); };
            let inner_ty = input.debug.get_type(inner_ty);
            let ptr_val = input.dump.read_ptr(addr + ptr_o)?;
            println!("{:depth$}->{:#x}", "", ptr_val);
            if ptr_val != 0 && output.shared_pointers.insert(ptr_val) {
                let _ = visit_type(input, output, depth+1, inner_ty, ptr_val, path.deref())?;
            }
            Ok(())
        }
        fn unique_ptr(input: &Input, output: &mut Output, depth: usize, ty: &debug_info::Type, addr: u64, path: Path, ptr_path: Path) -> Result<(),core_dump::ReadError> {
            let (ptr_o,ptr_ty) = input.get_field(ty, ptr_path);
            let ptr_t = input.debug.get_type(&ptr_ty);
            let debug_info::Type::Pointer(inner_ty, ..) = ptr_t else { panic!("Expected pointer, got  {:?}", ptr_t); };
            let inner_ty = input.debug.get_type(inner_ty);
            let ptr_val = input.dump.read_ptr(addr + ptr_o)?;
            println!("{:depth$}->{:#x}", "", ptr_val);
            if ptr_val != 0 {
                let _ = visit_type(input, output, depth+1, inner_ty, ptr_val, path.deref())?;
            }
            Ok(())
        }
        // Some mrustc special types
        match composite_type.name() {
        // interned string type, ignore for now (TODO: Look for duplicates?)
        "RcString" => return Ok(()),
        // `Span`: A fixed-size reference-counted type
        "Span" => return rc_ptr(input, output, depth, ty, addr, path, Path::root().field("m_ptr")),
        // `HIR::TypeRef` - Shared pointer to type data
        "HIR::TypeRef" => return rc_ptr(input, output, depth, ty, addr, path, Path::root().field("m_ptr")),
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


        // --- Fallback: Recurse into the type ---
        fn visit_ct_inner(input: &Input, output: &mut Output, depth: usize, composite_type: &debug_info::CompositeType, addr: u64, path: Path) -> Result<(),core_dump::ReadError> {
            let mut p = ProgressTracker::new(composite_type.parents().count() + composite_type.iter_fields().count());
            let mut idx = 0;
            for (i,(ofs,ty)) in composite_type.parents().enumerate() {
                idx += 1;
                let debug_info::Type::Struct(ct) = input.debug.get_type(ty) else { panic!("Parent type not a struct"); };
                println!("{:depth$}{ty} @ {addr:#x} ({path})", "", depth=depth+1, addr=addr+ofs, ty=ct.name(), path=path.parent(i));
                p.update(idx-1, path.parent(i));
                visit_ct_inner(input, output, depth+1, ct, addr + ofs, path.parent(i))?;
            }
            for f in composite_type.iter_fields() {
                idx += 1;
                if f.name.starts_with("_vptr.") {   // cspell::disable-line
                    // Skip VTable pointers
                    continue ;
                }
                if composite_type.name().starts_with("std::_Sp_counted_ptr<") && f.name == "_M_ptr" {
                    // Skip the data pointer stored in shared_ptr's count (avoids even attempting to double-visit)
                    continue;
                }
                p.update(idx-1, path.field(&f.name));
                visit_type(input, output, depth+1, input.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name))?;
            }
            Ok(())
        }
        visit_ct_inner(input, output, depth, composite_type, addr, path)?
    },
    debug_info::Type::TaggedUnion(e) => {
        fn find_variant<'a>(input: &Input, variants: &'a [debug_info::EnumVariant], discr_addr: u64) -> Result<Option<&'a debug_info::EnumVariant>,core_dump::ReadError> {
            for var in variants.iter() {
                for dv in var.discr_vals.iter() {
                    match *dv {
                    debug_info::VariantDiscr::Data(ref des) => {
                        assert!(des.len() <= 16, "TODO: Long enum discriminant ({} bytes > 16)", des.len());
                        let mut buf = [0; 16];
                        let buf = &mut buf[..des.len()];
                        input.dump.read_bytes(discr_addr, buf)?;
                        if buf == des {
                            return Ok(Some(var));
                        }
                    },
                    debug_info::VariantDiscr::SingleU(v,s) => {
                        let des = {
                            let mut b = [0u8; 16];
                            match s {
                            1 => b[..1].copy_from_slice(&(v as u8).to_ne_bytes()),
                            2 => b[..2].copy_from_slice(&(v as u16).to_ne_bytes()),
                            4 => b[..4].copy_from_slice(&(v as u32).to_ne_bytes()),
                            8 => b[..8].copy_from_slice(&(v as u64).to_ne_bytes()),
                            _ => todo!(),
                            }
                            b
                        };
                        let mut buf = [0; 16];
                        input.dump.read_bytes(discr_addr, &mut buf[..s as usize])?;
                        if buf == des {
                            return Ok(Some(var));
                        }
                    },
                    }
                }
            }
            Ok(variants.iter().find(|v| v.discr_vals.is_empty()))
        }
        //print!("TaggedUnion: "); dump_type_fields(input.debug, ty, 0); println!("");
        for f in &e.outer.fields {
            visit_type(input, output, depth+1, input.debug.get_type(&f.ty), addr + f.offset, path.field(&f.name))?;
        }
        let variant = if let Some(o) = e.discr_ofs {
            find_variant(input, &e.variants, addr + o)?
        }
        else {
            e.variants.first()
        };
        if let Some(v) = variant {
            let vi = unsafe { (v as *const debug_info::EnumVariant).offset_from_unsigned(e.variants.as_ptr()) };
			let name = if v.fields.len() == 1 { v.fields[0].name.clone() } else { format!("#{vi}") };
			*output.enum_variant_counts
				.entry(e.outer.name().to_owned()).or_default()
				.entry(name).or_default()
				+= 1;
            //println!("Matched {} #{vi}", e.outer.name());
            // TODO: Record the variant in the stats for this type
            // Recurse
            for f in &v.fields {
                visit_type(input, output, depth+1, input.debug.get_type(&f.ty), addr + f.offset, path.index(vi).field(&f.name))?;
            }
        }
        else {
            // No variant
            println!("No matching variant for {}", input.debug.fmt_type(ty));
        }
    },
    debug_info::Type::Union(composite_type) => {
        println!("Not recursing into union: {:?}", composite_type.name());
    },
    debug_info::Type::Array(..) => todo!("visit_type: array"),
    debug_info::Type::Enum(_) => {},
    debug_info::Type::Primitive(_) => {},
    debug_info::Type::Pointer(dst_ty, _, name) => {
        let addr = input.dump.read_ptr(addr)?;
        println!("{:depth$}->{:#x}", "", addr);
        if addr != 0 {
            // Only visit into rust's `Box` type
            if name.starts_with("alloc::boxed::Box<") {
                let _ = visit_type(input, output, depth+1, input.debug.get_type(dst_ty), addr, path.deref())?;
            }
        }
    },
    }
    Ok(())
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

use progress::ProgressTracker;
mod progress {
    use ::std::sync::atomic::Ordering;
    use ::std::sync::atomic::{AtomicUsize,AtomicU64};
    
    static CUR_LEVEL: AtomicUsize = AtomicUsize::new(0);
    static CUR_PROGRESS: AtomicU64 = AtomicU64::new(0);
    static CUR_SPAN: AtomicU64 = AtomicU64::new(1.0f64.to_bits());

    fn atomic_f64_load(dst: &AtomicU64) -> f64 {
        f64::from_bits(dst.load(Ordering::Relaxed))
    }
    fn atomic_f64_store(dst: &AtomicU64, v: f64) {
        dst.store(v.to_bits(), Ordering::Relaxed)
    }

    /// Track nested progress
    pub struct ProgressTracker {
        level: usize,
        max: usize,
        base_frac: f64,
        span_frac: f64,
    }
    impl ProgressTracker {
        pub fn new(max: usize) -> Self {
            Self {
                level: CUR_LEVEL.fetch_add(1, Ordering::Relaxed),
                max,
                base_frac: atomic_f64_load(&CUR_PROGRESS),
                span_frac: {
                    let s = atomic_f64_load(&CUR_SPAN);
                    atomic_f64_store(&CUR_SPAN, s / max as f64);
                    s
                },
            }
        }
        pub fn update(&mut self, i: usize, path: crate::visit_helpers::Path) {
            let _ = path;
            assert!(i <= self.max);
            atomic_f64_store(&CUR_PROGRESS, self.base_frac + i as f64 * self.span_frac / self.max as f64);
            redraw(Some(&path));
        }
    }
    impl ::core::ops::Drop for ProgressTracker {
        fn drop(&mut self) {
            assert!(self.level == CUR_LEVEL.fetch_sub(1, Ordering::SeqCst) - 1);
            atomic_f64_store(&CUR_SPAN, self.span_frac);
            atomic_f64_store(&CUR_PROGRESS, self.base_frac + self.span_frac);
            redraw(None);
        }
    }
    fn redraw(v: Option<&crate::visit_helpers::Path>) {
        if true {
            return ;
        }
        // TODO: Also get the percentage of covered memory, for a secondary measure
        static LAST_UPDATE: AtomicU64 = AtomicU64::new(0);
        let p = atomic_f64_load(&CUR_PROGRESS);
        if p - atomic_f64_load(&LAST_UPDATE) > 0.001 {
            eprint!("\r{:.1}%", p * 100.);
            if let Some(v) = v {
                eprint!(" {}", v);
            }
            eprint!("   \r");
            let _ = ::std::io::Write::flush(&mut ::std::io::stdout());
            atomic_f64_store(&LAST_UPDATE, p);
        }
    }

    static TOTAL_BYTES: AtomicU64 = AtomicU64::new(!0);
    static VISITED_BYTES: AtomicU64 = AtomicU64::new(0);
    pub fn set_total(n_bytes: u64) {
        TOTAL_BYTES.store(n_bytes, Ordering::Relaxed);
    }
    pub fn add_mem(n_bytes: usize) {
        VISITED_BYTES.fetch_add(n_bytes as u64, Ordering::Relaxed);
        static LAST_UPDATE: AtomicU64 = AtomicU64::new(0);
        let visited = VISITED_BYTES.load(Ordering::Relaxed);
        let total = TOTAL_BYTES.load(Ordering::Relaxed);
        let p = visited as f64 / total as f64;
        if p - atomic_f64_load(&LAST_UPDATE) > 0.001 {
            eprint!("\r{:.1}% memory ({} MiB / {} MiB)", p * 100., visited >> 20, total >> 20);
            eprint!("   \r");
            let _ = ::std::io::Write::flush(&mut ::std::io::stdout());
            atomic_f64_store(&LAST_UPDATE, p);
        }
    }
}