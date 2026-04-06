
mod dwarf_parse;

struct ElfFiles<'a> {
    // TODO: Rewrite this again to use `::elf::ElfStream`, but that has some interactions with lifetimes in the closure passed to `Dwarf::load`
    // - Doing so would avoid needing to load the entire executable (including the .text section).
    // - For now, it's not a huge cost
    file_main: ::elf::ElfBytes<'a, ::elf::endian::NativeEndian>,
    file_debug: Option<::elf::ElfBytes<'a, ::elf::endian::NativeEndian>>,
}
impl<'s> ElfFiles<'s> {
    pub fn open(path: &::std::path::Path, storage: &'s mut (Vec<u8>,Vec<u8>)) -> Result<Self, Box<dyn ::std::error::Error>> {
        let path_debug = path.with_added_extension("debug");
        Ok(ElfFiles {
            file_main: {
                storage.0 = ::std::fs::read(path)?;
                ::elf::ElfBytes::minimal_parse(&storage.0)?
            },
            file_debug: if path_debug.exists() {
                storage.1 = ::std::fs::read(path_debug)?;
                Some(::elf::ElfBytes::minimal_parse(&storage.1)?)
            } else {
                None
            },
        })
    }
    pub fn section_headers(&self) -> ::elf::section::SectionHeaderTable<'s, ::elf::endian::LittleEndian> {
        self.file_main.section_headers().unwrap()
    }
    pub fn section_data_opt(&self, name: &str) -> Result<Option<(::elf::section::SectionHeader, &'s [u8])>,::elf::ParseError> {
        Ok(match self.file_main.section_header_by_name(name)?
        {
        Some(v) => Some((v, self.file_main.section_data(&v)?.0.into())),
        None => match self.file_debug
            {
            Some(ref f) => match f.section_header_by_name(name)?
                {
                Some(v) => Some((v, f.section_data(&v)?.0.into())),
                None => None,
                },
            None => None,
            },
        })
    }
}

#[derive(Default)]
pub struct DebugPool {
    functions: ::std::collections::HashMap<String,FunctionRecord>,
    type_lookup: ::std::collections::HashMap< (usize, ::gimli::UnitOffset), TypeRef >,
    backtrace_data: Vec<(u64, u64, BacktraceType,)>,
    types: Vec<Option<Type>>,
    next_unit_index: usize,
}
impl DebugPool {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn add_file(&mut self, path: &::std::path::Path, base: u64, file_base: u64) -> Result<(), Box<dyn ::std::error::Error>>
    {
        println!("{:?} @ {:#x} (from {:#x})", path, base, file_base);
        let mut elf_files = (Vec::new(), Vec::new());
        let elf_files = ElfFiles::open(path, &mut elf_files)?;
        // Load with ELF loade
        let mut lowest_load = !0;
        for s in elf_files.section_headers() {
            if s.sh_flags & ::elf::abi::SHF_ALLOC as u64 != 0 {
                lowest_load = s.sh_addr.min(lowest_load);
            }
        }
        println!("lowest_load={:#x}", lowest_load);

        if let Some((shdr,sdata)) = elf_files.section_data_opt(".debug_frame")?
        {
            let section_base = shdr.sh_addr;
            let debug_frame: ::std::rc::Rc<[u8]> = sdata.into();
            let debug_frame = ::gimli::EndianRcSlice::new(debug_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, section_base, BacktraceType::Debug(::gimli::DebugFrame::from(debug_frame)),));
        }
        else if let Some((shdr,sdata)) = elf_files.section_data_opt(".eh_frame")?
        {
            let section_base = shdr.sh_addr;
            let eh_frame: ::std::rc::Rc<[u8]> = sdata.into();
            let eh_frame = ::gimli::EndianRcSlice::new(eh_frame, ::gimli::LittleEndian);
            self.backtrace_data.push((base, section_base, BacktraceType::Eh(::gimli::EhFrame::from(eh_frame)),));
        }
        else {
            // No unwind/backtrace data
        }

        let debug_info = ::gimli::Dwarf::load::<_,::elf::ParseError>(|section| {
            let s = elf_files.section_data_opt(section.name())?;
            //println!("{} {}: {:?}", path.display(), section.name(), s.as_ref().map(|v| v.0));
            let section_data = match s {
                Some((_,sdata)) => sdata,
                None => b"",
            };
            Ok(::gimli::EndianSlice::new(section_data, ::gimli::LittleEndian))
        })?;
        self.add_variables_types_from_dwarf(base, &debug_info);
        println!("LOADED {}", path.display());
        Ok( () )
    }

    fn get_unwind<'ctxt>(&self, ctx: &'ctxt mut ::gimli::UnwindContext<usize>, address: u64) -> Option<&'ctxt ::gimli::UnwindTableRow<usize>> {
        for (base, eh_base, info) in &self.backtrace_data {
            //println!("get_unwind: {:#x} + {:#x}", base, eh_base);
            let bases = ::gimli::BaseAddresses::default()
                .set_text(*base)
                .set_eh_frame(*base + *eh_base)
                ;
            use ::gimli::UnwindSection;
            match match info 
                {
                BacktraceType::Debug(debug_frame) =>
                    debug_frame.unwind_info_for_address(&bases, ctx, address, ::gimli::DebugFrame::cie_from_offset),
                BacktraceType::Eh(eh_frame) => 
                    eh_frame.unwind_info_for_address(&bases, ctx, address, ::gimli::EhFrame::cie_from_offset),
                }
            {
            // HACK: Launder the pointer, avoiding a double-borrow issue with `ctx`
            Ok(i) => return Some(unsafe { &*(i as *const _)}),
            Err(gimli::Error::NoUnwindInfoForAddress) => continue,
            Err(e) => todo!("Unwind error: {:?}", e),
            }
        }
        return None;
    }
    fn get_cfa(state: &crate::CpuState, cfa: &gimli::CfaRule<usize>) -> u64 {
        match cfa
        {
        &gimli::CfaRule::RegisterAndOffset { register, offset } => get_register(state, &register).checked_add_signed(offset).unwrap(),
        &gimli::CfaRule::Expression(_unwind_expression) => todo!("CfaRule::Expression"),
        }
    }
    pub fn get_caller(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump) -> crate::CpuState {
        println!("get_caller: {:#x}", state.get_pc());
        let mut context = ::gimli::UnwindContext::new();
        let Some(i) = self.get_unwind(&mut context, state.get_pc()) else {
            todo!("get_caller: no entry for PC={:#x}", state.get_pc());
        };
        let cfa = Self::get_cfa(state, i.cfa());
        println!("get_caller: cfa={:#x}", cfa);
        let mut rv = state.clone();
        for (r_name,rule) in i.registers() {
            let v = match rule
                {
                ::gimli::RegisterRule::Undefined => 0,
                ::gimli::RegisterRule::SameValue => state.gprs[r_name.0 as usize],
                ::gimli::RegisterRule::Offset(cfa_ofs) => memory.read_ptr(cfa.wrapping_add_signed(*cfa_ofs)),
                ::gimli::RegisterRule::ValOffset(cfa_ofs) => cfa.wrapping_add_signed(*cfa_ofs),
                ::gimli::RegisterRule::Register(register) => get_register(state, register),
                ::gimli::RegisterRule::Expression(_unwind_expression) => todo!("RegisterRule::Expression"),
                ::gimli::RegisterRule::ValExpression(_unwind_expression) => todo!("RegisterRule::ValExpression"),
                ::gimli::RegisterRule::Architectural => todo!("RegisterRule::Architectural"),
                ::gimli::RegisterRule::Constant(v) => *v,
                };
            println!("> {:?}: {:?} = {:#x}", r_name, rule, v);
            match r_name.0 {
            i @ 0 .. 16 => rv.gprs[i as usize] = v,
            16 => rv.pc = v,
            _ => {},
            }
        }
        // Not sure if this is documented, but it seems to work
        println!("> RSP = CFA {:#x}", cfa);
        rv.gprs[7] = cfa;
        return rv;
    }

    
    fn evaluate_position(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump, pos: &VariablePosition, fcn_rec: &FunctionRecord) -> u64 {
        match pos {
        VariablePosition::OptimisedOut => todo!("Optimsed out variable"),
        VariablePosition::Fixed(p) => *p,
        VariablePosition::Expr(items, encoding) => {
            let r = ::gimli::EndianReader::new(items.as_slice(), ::gimli::NativeEndian);
            let e = ::gimli::read::Expression(r);
            print!("EVAL: ");
            e.operations(*encoding).for_each(|v| print!(">> {:?}", v.unwrap()));
            println!("");
            let mut e = e.evaluation(*encoding);
            let mut r = e.evaluate();
            loop {
                use gimli::EvaluationResult as E;
                r = match r.expect("Failure evaluating")
                {
                E::Complete => {
                    let r= e.result();
                    assert!(r.len() == 1, "Multiple (or zero) pieces? {:?}", r);
                    match r[0].location
                    {
                    gimli::Location::Address { address } => break address,
                    a @ _ => todo!("Location: {:?}", a),
                    }
                    },
                E::RequiresMemory { address, size, space, base_type } => todo!("RequiresMemory"),
                E::RequiresRegister { register, base_type }
                    => e.resume_with_register(::gimli::Value::U64(get_register(state, &register))),
                E::RequiresFrameBase => e.resume_with_frame_base(self.evaluate_position(state, memory, &fcn_rec.frame_base, fcn_rec)),
                E::RequiresTls(_) => todo!("RequiresTls"),
                E::RequiresCallFrameCfa => {
                    let mut context = ::gimli::UnwindContext::new();
                    let Some(i) = self.get_unwind(&mut context, state.get_pc()) else {
                        todo!("get_variable: no backtrace for PC={:#x} to get CFA", state.get_pc());
                    };
                    let cfa = Self::get_cfa(state, i.cfa());
                    println!("cfa={:#x}", cfa);
                    e.resume_with_call_frame_cfa(cfa)
                },
                E::RequiresAtLocation(_die_reference) => todo!("RequiresAtLocation"),
                E::RequiresEntryValue(_expression) => todo!(),
                E::RequiresParameterRef(_unit_offset) => todo!(),
                E::RequiresRelocatedAddress(_) => todo!(),
                E::RequiresIndexedAddress { index, relocate } => todo!(),
                E::RequiresBaseType(_unit_offset) => todo!("RequiresBaseType"),
                };
            }
        },
        }
    }

    // Get the storage address of a variable
    pub fn get_variable(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump, name: &str) -> (u64, TypeRef) {
        let pc = state.get_pc();
        let Some((fcn_name,fcn_rec)) = self.functions.iter().find(|(_,r)| r.pc_range.contains(pc)) else {
            panic!("get_variable: {:?} - Failed to find function for PC={:#x} ({})", name, pc, self.functions.len())
        };
        let Some(var) = &fcn_rec.variables.get(name) else {
            panic!("get_variable: Variable {} not in function {}", name, fcn_name);
        };
        let Some(r) = var.ranges.iter().find(|r| r.pc_range.contains(pc)) else {
            panic!("Unable to find variable def in {} at PC {:#x}", fcn_name, pc);
        };
        let p = self.evaluate_position(state, memory, &r.position, fcn_rec);
        return (p, var.ty);
    }
    #[track_caller]
    pub fn get_type(&self, ty: &TypeRef) -> &Type {
        match self.types[ty.0] {
        Some(ref v) => v,
        None => panic!("Type not populated: {:?} = {:?}", ty, self.type_lookup.iter().find(|(_,v)| v.0 == ty.0)),
        }
    }
    pub fn size_of(&self, ty: &Type) -> usize {
        match ty {
        Type::Struct(composite_type) => composite_type.size,
        Type::Union(composite_type) => composite_type.size,
        Type::Primtive(primitive_type) => (primitive_type.bits as usize + 7) / 8,
        Type::Pointer(_) => 8,
        Type::Alias(type_ref) => self.size_of(self.get_type(type_ref)),
        Type::Enum(_) => todo!("size_of: enum"),
        }
    }

    fn dwarf_type_ref(&mut self, unit_index: usize, ofs: ::gimli::UnitOffset) -> TypeRef {
        *self.type_lookup.entry((unit_index, ofs))
            .or_insert_with(|| {
                let rv = TypeRef(self.types.len());
                self.types.push(None);
                rv
            })
    }

    fn fmt_type_ref_inner(&self, f: &mut std::fmt::Formatter<'_>, ty: &TypeRef) -> ::std::fmt::Result {
        match self.types[ty.0] {
        Some(ref ty) => self.fmt_type_inner(f, ty),
        None => write!(f, "?#{}", ty.0)
        }
    }
    fn fmt_type_inner(&self, f: &mut std::fmt::Formatter<'_>, ty: &Type) -> ::std::fmt::Result {
        match ty {
        Type::Enum(name) => f.write_str(name),
        Type::Struct(composite_type) => f.write_str(&composite_type.name),
        Type::Union(composite_type) => f.write_str(&composite_type.name),
        Type::Primtive(primitive_type) => write!(f, "prim{}", primitive_type.bits),
        Type::Pointer(type_ref) => {
            f.write_str("*")?;
            self.fmt_type_ref_inner(f, type_ref)
        },
        Type::Alias(type_ref) => {
            // TODO: have the name save too?
            f.write_str("=")?;
            self.fmt_type_ref_inner(f, type_ref)
        }
        }
    }
    pub fn fmt_type<'a>(&'a self, ty: &'a Type) -> impl ::std::fmt::Display + 'a {
        struct F<'a>(&'a DebugPool, &'a Type);
        impl<'a> ::std::fmt::Display for F<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt_type_inner(f, self.1)
            }
        }
        F(self, ty)
    }
    pub fn fmt_type_ref<'a>(&'a self, ty: &TypeRef) -> impl ::std::fmt::Display + 'a {
        struct F<'a>(&'a DebugPool, TypeRef);
        impl<'a> ::std::fmt::Display for F<'a> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt_type_ref_inner(f, &self.1)
            }
        }
        F(self, *ty)
    }
}

fn get_register(state: &crate::CpuState, register: &::gimli::Register) -> u64 {
    match register.0 {
    i @ 0 .. 16 => state.gprs[i as usize],
    16 => state.pc,
    _ => todo!("get_register: {:?}", register),
    }
}

#[derive(Debug)]
enum BacktraceType {
    Debug(::gimli::DebugFrame<::gimli::EndianRcSlice<::gimli::LittleEndian>>),
    Eh(::gimli::EhFrame<::gimli::EndianRcSlice<::gimli::LittleEndian>>),
}

#[derive(Clone, Copy)]
struct PcRange {
    start: u64,
    end: u64,
}
impl PcRange {
    fn contains(&self, pc: u64) -> bool {
        self.start <= pc && pc < self.end
    }
}
impl ::std::fmt::Debug for PcRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:x}--0x{:x}", self.start, self.end)
    }
}
#[derive(Debug,Clone)]
struct PcRanges {
    ranges: Vec<PcRange>,
}
impl PcRanges {
    fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }
    fn contains(&self, pc: u64) -> bool {
        self.ranges.iter().any(|v| v.contains(pc))
    }
}
struct FunctionRecord {
    /// Range of PC values covered by this function
    pc_range: PcRanges,
    frame_base: VariablePosition,
    variables: ::std::collections::HashMap<String,VariableRecord>,
}
#[derive(Debug)]
struct VariableRecord {
    ty: TypeRef,
    ranges: Vec<VariableRange>,
}
#[derive(Debug)]
struct VariableRange {
    pc_range: PcRanges,
    position: VariablePosition,
}
#[derive(Debug)]
enum VariablePosition {
    OptimisedOut,
    Fixed(u64),
    Expr(Vec<u8>, ::gimli::Encoding),  // see `gimli::read::Expression`
}

/// Reference to a type in the debug tree
#[derive(Clone, Copy)]
#[derive(Debug)]
pub struct TypeRef(usize);

#[derive(Debug)]
pub enum Type {
    Struct(CompositeType),
    Union(CompositeType),
    Primtive(PrimitiveType),
    Pointer(TypeRef),
    Alias(TypeRef),
    Enum(String),
}
#[derive(Debug)]
pub struct PrimitiveType {
    name: String,
    bits: u32,
}
#[derive(Debug)]
pub struct CompositeType {
    name: String,
    size: usize,
    fields: Vec<CompositeField>,
    parents: Vec<(u64, TypeRef)>,
    pub sub_types: ::std::collections::HashMap<String,TypeRef>,
}
impl CompositeType {
    fn new(name: String, size: usize) -> Self {
        CompositeType {
            name,
            size,
            fields: Default::default(),
            parents: Default::default(),
            sub_types: Default::default()
        }
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn parents(&self) -> impl Iterator<Item=&(u64, TypeRef)> {
        self.parents.iter()
    }
    pub fn iter_fields(&self) -> impl Iterator<Item=&CompositeField> {
        self.fields.iter()
    }
}
#[derive(Debug)]
pub struct CompositeField {
    pub offset: u64,
    pub name: String,
    pub ty: TypeRef,
}