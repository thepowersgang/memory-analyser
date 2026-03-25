
#[derive(Default)]
pub struct DebugPool {
    functions: ::std::collections::HashMap<String,FunctionRecord>,
    type_lookup: ::std::collections::HashMap< (usize, ::gimli::UnitOffset), TypeRef >,
    types: Vec<Option<Type>>,
    next_unit_index: usize,
}
impl DebugPool {
    pub fn new() -> Self {
        Default::default()
    }
    pub fn add_file(&mut self, path: &::std::path::Path, base: u64)
    {
        let tmp_path = path.with_added_extension("debug");
        let path = if tmp_path.exists() {
            tmp_path.as_path()
        }
        else {
            path
        };
        // Load with ELF loader
        let bytes = ::std::fs::read(path).unwrap();
        let elf_file = ::elf::ElfBytes::<::elf::endian::LittleEndian>::minimal_parse(&bytes).expect("Open test1");
        let debug_info = ::gimli::Dwarf::load::<_,::elf::ParseError>(|section| {
            let s = elf_file.section_header_by_name(section.name())?;
            let section_data = match s {
                Some(s) => elf_file.section_data(&s)?.0,
                None => b"",
            };
            Ok(::gimli::EndianSlice::new(section_data, ::gimli::LittleEndian))
        }).unwrap();
        //::gimli::DebugFrame::new(section, endian)
        fn get_name<'a, E>(debug_info: &::gimli::Dwarf<::gimli::EndianSlice<'a, E>>, unit: &gimli::Unit<::gimli::EndianSlice<'a, E>>, v: &::gimli::DebuggingInformationEntry<::gimli::EndianSlice<'a, E>>) -> Option<&'a str>
        where
            E: ::gimli::Endianity,
        {
            match v.attr_value(gimli::DW_AT_name)
            {
            Some(a) => {
                let sa = debug_info.attr_string(&unit, a).unwrap();
                Some(::std::str::from_utf8(&sa.slice()[..]).unwrap())
                },
            None => None,
            }
        }

        // Parse type and variable information.
        for u in debug_info.units() {
            let unit_index = self.next_unit_index;
            self.next_unit_index += 1;

            let u = u.unwrap();
            let unit = debug_info.unit(u).unwrap();
            match unit.type_() {
            gimli::UnitType::Compilation => {
                let abbr = debug_info.abbreviations(&u).unwrap();

                enum State {
                    Root,
                    InType(String, TypeRef, Vec<CompositeField>),
                    InFunction(String, FunctionRecord),
                    // Should only exist underneath a `InFunction`
                    FcnScope(PcRange),
                }
                impl ::std::fmt::Debug for State {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        match self {
                        Self::Root => write!(f, "Root"),
                        Self::InType(name, ..) => f.debug_tuple("InType").field(name).finish(),
                        Self::InFunction(name, ..) => f.debug_tuple("InFunction").field(name).finish(),
                        Self::FcnScope(arg0) => f.debug_tuple("FcnScope").field(arg0).finish(),
                        }
                    }
                }
                fn parent_name(stack: &[State]) -> Option<&str> {
                    for v in stack.iter().rev() {
                        match v {
                        State::Root => {},
                        State::InFunction(n, ..) => return Some(n),
                        State::FcnScope(..) => {},
                        State::InType(n, _, _) => return Some(n),
                        }
                    }
                    None
                }
                fn get_scoped_name(stack: &[State], prefix: &str, name: Option<&str>, ofs: ::gimli::UnitOffset) -> String {
                    let mut full_name = parent_name(stack).unwrap_or_default().to_owned();
                    full_name.push_str("::");
                    full_name.push_str(prefix);
                    use ::std::fmt::Write;
                    match name {
                    None => { let _ = write!(full_name, "@{}", ofs.0); },
                    Some(n) => full_name.push_str(n),
                    }
                    full_name
                }
                {
                    let mut stack = vec![State::Root];
                    let mut ents = u.entries(&abbr);
                    while let Some(v) = ents.next_dfs().unwrap()
                    {
                        while stack.len() > v.depth as usize {
                            match stack.pop().unwrap() {
                            State::InFunction(n, fr) => {
                                println!("END fn {}", n);
                                println!("{:?}", fr.variables);
                                let was_main = n == "::main";
                                self.functions.insert(n, fr);
                                if was_main {
                                    return
                                }
                            } 
                            State::InType(name, ty, fields) => {
                                println!("END type {}", name);
                                self.types[ty.0] = Some(Type::Composite(CompositeType { name, fields }));
                            }
                            _ => {},
                            }
                        }
                        if (v.depth as usize) > stack.len() {
                            continue ;
                        }
                        fn get_pc_range(v: &gimli::DebuggingInformationEntry<::gimli::EndianSlice<::gimli::LittleEndian>>) -> PcRange {
                            fn unwrap_addr(v: &::gimli::Attribute<::gimli::EndianSlice<::gimli::LittleEndian>>) -> u64 {
                                match v.value() {
                                ::gimli::AttributeValue::Addr(a) => a,
                                ::gimli::AttributeValue::Udata(a) => a,
                                _ => todo!("{v:?} : {:?}", v.value()),
                                }
                            }
                            let at_lo = v.attr(::gimli::DW_AT_low_pc).map(|v| unwrap_addr(v));
                            let at_hi = v.attr(::gimli::DW_AT_high_pc).map(|v| unwrap_addr(v));
                            let ranges = v.attr(::gimli::DW_AT_ranges).map(|v| v.value());
                            match (at_lo, at_hi, ranges)
                            {
                            (Some(lo), None, _) => PcRange { start: lo, end: lo },
                            (Some(lo), Some(hi), _) => PcRange { start: lo, end: hi },
                            (None, _, None) => PcRange { start: 0, end: 0 },
                            (None, _, Some(r)) => PcRange { start: 0, end: 0 },
                            }
                        }
                        //println!("{} {:?} {:x?}", v.depth, stack, v);
                        match v.tag()
                        {
                        gimli::DW_TAG_subprogram => {
                            let name = get_name(&debug_info, &unit, v);
                            let full_name = get_scoped_name(&stack, "", name, v.offset);
                            println!("fn {}: name={:?} @ {}", full_name, name, v.depth);
                            let pc_range = get_pc_range(v);
                            stack.push(State::InFunction(full_name, FunctionRecord {
                                pc_range,
                                variables: Default::default(),
                            }));
                            continue;
                        },

                        gimli::DW_TAG_typedef => {
                            println!("> typedef: {:?}", get_name(&debug_info, &unit, v));
                            continue
                        },
                        gimli::DW_TAG_structure_type | gimli::DW_TAG_class_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let full_name = get_scoped_name(&stack, "struct ", name, v.offset);
                            stack.push(State::InType(full_name, ty_ref, vec![]));
                            continue;
                        },
                        gimli::DW_TAG_enumeration_type => {
                            println!("> enum: {:?}", get_name(&debug_info, &unit, v));
                            continue;
                        },
                        gimli::DW_TAG_union_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let full_name = get_scoped_name(&stack, "union ", name, v.offset);
                            stack.push(State::InType(full_name, ty_ref, Vec::new()));
                            continue;
                        },
                        gimli::DW_TAG_const_type => {
                            println!("> const_type: {:?}", get_name(&debug_info, &unit, v));
                            continue
                        },
                        gimli::DW_TAG_pointer_type => {
                            println!("> pointer_type: {:?}", get_name(&debug_info, &unit, v));
                            continue
                        },
                        gimli::DW_TAG_reference_type => {
                            println!("> reference type: {:?}", get_name(&debug_info, &unit, v));
                            continue
                        },
                        gimli::DW_TAG_rvalue_reference_type => {},
                        _ => {},
                        }
                        match stack.last().unwrap_or(&State::Root) {
                        State::Root => {
                            match v.tag()
                            {
                            gimli::DW_TAG_compile_unit => {
                                stack.push(State::Root);
                            },
                            _ => {},
                            }
                        },
                        &State::InFunction(_, FunctionRecord { pc_range, .. }) | &State::FcnScope(pc_range) => {
                            match v.tag()
                            {
                            gimli::DW_TAG_lexical_block => {
                                // TODO: Get code ranges, to be included in downstream `DW_TAG_variable` items
                                let pc_range = get_pc_range(v);
                                stack.push(State::FcnScope(pc_range));
                                },
                            gimli::DW_TAG_GNU_template_parameter_pack => {},
                            gimli::DW_TAG_GNU_formal_parameter_pack => {},

                            gimli::DW_TAG_formal_parameter => {},
                            gimli::DW_TAG_unspecified_parameters => {},
                            gimli::DW_TAG_inlined_subroutine => {},

                            gimli::DW_TAG_template_type_parameter => {},
                            gimli::DW_TAG_template_value_parameter => {},
                            //gimli::DW_TAG_member => {},
                            gimli::DW_TAG_call_site => {},

                            gimli::DW_TAG_typedef => {},

                            gimli::DW_TAG_label => {},

                            gimli::DW_TAG_variable => {
                                let name = get_name(&debug_info, &unit, v);
                                let loc = v.attr_value(gimli::DW_AT_location);
                                let ty = v.attr_value(gimli::DW_AT_type);
                                println!(" > Variable {:?} @ {:?} {:?}", name, loc, ty);
                                // Record the type and offset
                                let pos = match loc {
                                    Some(v) => match v
                                        {
                                        gimli::AttributeValue::Addr(v) => VariablePosition::Fixed(v),
                                        gimli::AttributeValue::Exprloc(expression) => {
                                            VariablePosition::Expr(expression.0[..].to_owned())
                                        }
                                        gimli::AttributeValue::LocationListsRef(r) => {
                                            let expression = debug_info.locations(&unit, r)
                                                .unwrap()
                                                .next()
                                                .unwrap()
                                                ;
                                            match expression {
                                            Some(expression) => VariablePosition::Expr(expression.data.0[..].to_owned()),
                                            None => VariablePosition::OptimisedOut,
                                            }
                                        }
                                        _ => todo!("Position: {:?}", v),
                                        },
                                    None => VariablePosition::OptimisedOut,
                                    };
                                let ty = match ty
                                    {
                                    None => None,
                                    Some(ty) => {
                                        Some(match ty {
                                            gimli::AttributeValue::UnitRef(r) => self.dwarf_type_ref(unit_index, r),
                                            _ => todo!("Register type: {:?} {:?}", ty, ty.offset_value()),
                                            })
                                    },
                                    };
                                // Get the current function (look up the stack) and add this variable, along with its contained scope
                                if let Some(name) = name {
                                    stack.iter_mut()
                                        .filter_map(|v| match v { State::InFunction(_, fr) => Some(fr), _ => None })
                                        .next()
                                        .unwrap()
                                        .variables.entry(name.to_owned())
                                        .or_insert(VariableRecord { ty: ty.unwrap(), ranges: Vec::new() })
                                        .ranges
                                        .push(VariableRange { pc_range, position: pos });
                                }
                                },
                            
                            _ => {},
                            }
                        },
                        State::InType(ty_name, _, _) => {
                            match v.tag()
                            {
                            gimli::DW_TAG_GNU_template_template_param => {},

                            gimli::DW_TAG_template_type_parameter => {},
                            gimli::DW_TAG_template_value_parameter => {},
                            gimli::DW_TAG_inheritance => {},

                            // static
                            gimli::DW_TAG_variable => {},

                            gimli::DW_TAG_member => {
                                let name = get_name(&debug_info, &unit, v);
                                let pos = v.attr(gimli::DW_AT_data_member_location);
                                let ty = v.attr(gimli::DW_AT_type);
                                println!("> {ty_name}.field {name:?} @ {pos:?}: {ty:?} {v:?}");
                            },
                            _ => todo!("InType: {:x?}", v),
                            }
                        },
                        }
                    }
                }
            },
            gimli::UnitType::Type { type_signature, type_offset } => todo!(),
            gimli::UnitType::Partial => todo!(),
            gimli::UnitType::Skeleton(dwo_id) => todo!(),
            gimli::UnitType::SplitCompilation(dwo_id) => todo!(),
            gimli::UnitType::SplitType { type_signature, type_offset } => todo!(),
            }
        }
        // Parse type info
        todo!()
    }

    pub fn get_caller(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump) -> crate::CpuState {
        todo!()
    }

    // Get the storage address of a variable
    pub fn get_variable(&self, state: &crate::CpuState, memory: &crate::core_dump::CoreDump, name: &str) -> (u64, TypeRef) {
        todo!()
    }
    pub fn get_type(&self, ty: &TypeRef) -> &Type {
        todo!();
    }

    fn dwarf_type_ref(&mut self, unit_index: usize, ofs: ::gimli::UnitOffset) -> TypeRef {
        *self.type_lookup.entry((unit_index, ofs))
            .or_insert_with(|| {
                let rv = TypeRef(self.types.len());
                self.types.push(None);
                rv
            })
    }
}

#[derive(Debug, Clone, Copy)]
struct PcRange {
    start: u64,
    end: u64,
}
struct FunctionRecord {
    /// Range of PC values covered by this function
    pc_range: PcRange,
    variables: ::std::collections::HashMap<String,VariableRecord>,
}
#[derive(Debug)]
struct VariableRecord {
    ty: TypeRef,
    ranges: Vec<VariableRange>,
}
#[derive(Debug)]
struct VariableRange {
    pc_range: PcRange,
    position: VariablePosition,
}
#[derive(Debug)]
enum VariablePosition {
    OptimisedOut,
    Fixed(u64),
    Expr(Vec<u8>),  // see `gimli::read::Expression`
}

/// Reference to a type in the debug tree
#[derive(Clone, Copy)]
#[derive(Debug)]
pub struct TypeRef(usize);

pub enum Type {
    Composite(CompositeType),
    Primtive(PrimitiveType),
    Pointer(TypeRef),
}
pub struct PrimitiveType {
    bits: u32,
}
pub struct CompositeType {
    name: String,
    fields: Vec<CompositeField>,
}
impl CompositeType {
    /// Check if this is a named type with a given prefix (e.g. for finding `std::vector<`)
    pub fn is_name_prefix(&self, prefix: &str) -> bool {
        false
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