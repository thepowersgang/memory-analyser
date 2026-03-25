
#[derive(Default)]
pub struct DebugPool {
    functions: ::std::collections::HashMap<String,FunctionRecord>,
    type_lookup: ::std::collections::HashMap<::gimli::UnitOffset,TypeRef>,
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


                #[derive(Debug)]
                enum State {
                    Root,
                    InFunction(String),
                    InType(String),
                }
                {
                    let mut stack = vec![State::Root];
                    let mut ents = u.entries(&abbr);
                    while let Some(v) = ents.next_dfs().unwrap()
                    {
                        while stack.len() > v.depth as usize {
                            match stack.pop() {
                            Some(State::InFunction(ref n)) if n == "::main" && matches!(stack.last(), Some(State::Root)) => return,
                            _ => {},
                            }
                        }
                        if (v.depth as usize) > stack.len() {
                            continue ;
                        }
                        //println!("{} {:?} {:x?}", v.depth, stack, v);
                        match v.tag()
                        {
                        gimli::DW_TAG_subprogram => {
                            let name = get_name(&debug_info, &unit, v);
                            println!("function name={:?} @ {}", name, v.depth);
                            let mut full_name = String::new();
                            for v in &stack {
                                full_name.clear();
                                match v {
                                State::Root => {},
                                State::InFunction(n) => full_name.push_str(n),
                                State::InType(n) => full_name.push_str(n),
                                }
                                full_name.push_str("::");
                            }
                            use ::std::fmt::Write;
                            match name {
                            None => { let _ = write!(full_name, "@{}", v.offset.0); },
                            Some(n) => full_name.push_str(n),
                            }
                            stack.push(State::InFunction(full_name));
                            continue;
                        },

                        gimli::DW_TAG_typedef => {
                            println!("> typedef: {:?}", get_name(&debug_info, &unit, v));
                            continue
                        },
                        gimli::DW_TAG_structure_type | gimli::DW_TAG_class_type => {
                            let name = get_name(&debug_info, &unit, v);
                            let mut full_name = String::new();
                            for v in &stack {
                                full_name.clear();
                                match v {
                                State::Root => {},
                                State::InFunction(n) => full_name.push_str(n),
                                State::InType(n) => full_name.push_str(n),
                                }
                                full_name.push_str("::");
                            }
                            use ::std::fmt::Write;
                            match name {
                            None => { let _ = write!(full_name, "struct@{}", v.offset.0); },
                            Some(n) => { let _ = write!(full_name, "struct {}", n); },
                            }
                            stack.push(State::InType(full_name));
                            continue;
                        },
                        gimli::DW_TAG_enumeration_type => {
                            println!("> enum: {:?}", get_name(&debug_info, &unit, v));
                            continue;
                        },
                        gimli::DW_TAG_union_type => {
                            let name = get_name(&debug_info, &unit, v);
                            println!("> union_type: {:?}", name);
                            let name = match name
                                {
                                Some(v) => format!("union {}", v),
                                None => format!("union#{:?}", v.offset),
                                };
                            stack.push(State::InType(name));
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
                        State::InFunction(n) => {
                            match v.tag()
                            {
                            gimli::DW_TAG_lexical_block => {
                                // TODO: Get code ranges, to be included in downstream `DW_TAG_variable` items
                                stack.push(State::InFunction(n.clone()));
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
                                println!("{n} > Variable {:?} @ {:?} {:?}", name, loc, ty);
                                // Record the type and offset
                                let pos = match loc {
                                    Some(v) => match v
                                        {
                                        gimli::AttributeValue::Addr(v) => VariablePosition::Fixed(v),
                                        gimli::AttributeValue::Exprloc(expression) => {
                                            VariablePosition::Expr(expression.0[..].to_owned())
                                        }
                                        gimli::AttributeValue::LocationListsRef(r) => {
                                            let expression = debug_info.locations(&unit, r).unwrap().next().unwrap().unwrap().data;
                                            VariablePosition::Expr(expression.0[..].to_owned())
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
                                },
                            
                            _ => {},
                            }
                        },
                        State::InType(ty_name) => {
                            match v.tag()
                            {
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
                            _ => todo!("{:x?}", v),
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
        *self.type_lookup.entry(ofs)
            .or_insert_with(|| {
                let rv = TypeRef(self.types.len());
                self.types.push(None);
                rv
            })
    }
}

struct PcRange {
    start: u64,
    end: u64,
}
struct FunctionRecord {
    /// Range of PC values covered by this function
    pc_range: PcRange,
    variables: ::std::collections::HashMap<String,VariableRecord>,
}
struct VariableRecord {
    ty: TypeRef,
    pc_ranges: Vec<PcRange>,
    stack_offset: u64,
}
enum VariablePosition {
    OptimisedOut,
    Fixed(u64),
    Expr(Vec<u8>),  // see `gimli::read::Expression`
}

/// Reference to a type in the debug tree
#[derive(Clone, Copy)]
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
pub struct CompositeField {
    pub offset: u64,
    pub name: String,
    pub ty: TypeRef,
}