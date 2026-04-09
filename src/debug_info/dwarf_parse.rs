use super::{TypeRef, Type, CompositeType, CompositeField};
use super::{FunctionRecord, PcRanges, PcRange, VariableRecord, VariablePosition, VariableRange};

impl super::DebugPool
{
    fn get_typeref_from_attr(&mut self, unit_index: usize, v: &::gimli::DebuggingInformationEntry<::gimli::EndianSlice<::gimli::LittleEndian>>) -> Option<TypeRef>
    {
        match v.attr_value(::gimli::DW_AT_type)
        {
        None => None,
        Some(ty) => 
            Some(match ty {
                gimli::AttributeValue::UnitRef(r) => self.dwarf_type_ref(unit_index, r),
                _ => todo!("Register type: {:?} {:?}", ty, ty.offset_value()),
                }),
        }
    }
    pub(super) fn add_variables_types_from_dwarf(&mut self, load_base: u64, debug_info: &::gimli::Dwarf<::gimli::EndianSlice<::gimli::LittleEndian>>)
    {
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
                    Namespace(String),
                    InType(CompositeType, TypeRef, bool),
                    InFunction(String, FunctionRecord),
                    // Should only exist underneath a `InFunction`
                    FcnScope(PcRanges),
                }
                impl ::std::fmt::Debug for State {
                    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        match self {
                        Self::Root => write!(f, "Root"),
                        Self::Namespace(name) => f.debug_tuple("NamedScope").field(name).finish(),
                        Self::InType(ct, ..) => f.debug_tuple("InType").field(&ct.name).finish(),
                        Self::InFunction(name, ..) => f.debug_tuple("InFunction").field(name).finish(),
                        Self::FcnScope(arg0) => f.debug_tuple("FcnScope").field(arg0).finish(),
                        }
                    }
                }
                fn parent_name(stack: &[State]) -> Option<&str> {
                    for v in stack.iter().rev() {
                        match v {
                        State::Root => {},
                        State::Namespace(n) => return Some(n),
                        State::InFunction(n, ..) => return Some(n),
                        State::FcnScope(..) => {},
                        State::InType(ct, _, _) => return Some(&ct.name),
                        }
                    }
                    None
                }
                fn get_scoped_name(stack: &[State], class: &str, name: Option<&str>, ofs: ::gimli::UnitOffset) -> String {
                    let mut full_name = parent_name(stack).unwrap_or_default().to_owned();
                    if !full_name.is_empty() {
                        full_name.push_str("::");
                    }
                    use ::std::fmt::Write;
                    match name {
                    None => { let _ = write!(full_name, "{class}@{}", ofs.0); },
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
                                self.functions.insert(n, fr);
                            } 
                            State::InType(ct, ty, is_union) => {
                                if false {
                                    print!("{} {}: {{", if is_union { "union" } else { "struct" }, ct.name);
                                    for f in &ct.fields {
                                        print!(" {}: {:?},", f.name, f.ty);
                                    }
                                    println!(" }}");
                                }
                                
                                self.types[ty.0] = Some(if is_union {
                                    Type::Union(ct)
                                }else {
                                    Type::Struct(ct)
                                });
                            }
                            _ => {},
                            }
                        }
                        //println!("{} {:?} {:x?} ({}/@{})", v.depth, stack, v.tag(), unit_index, v.offset.0);
                        if (v.depth as usize) > stack.len() {
                            continue ;
                        }
                        let get_pc_range = |v: &gimli::DebuggingInformationEntry<::gimli::EndianSlice<::gimli::LittleEndian>>| -> PcRanges {
                            fn unwrap_addr(v: &::gimli::Attribute<::gimli::EndianSlice<::gimli::LittleEndian>>) -> u64 {
                                match v.value() {
                                ::gimli::AttributeValue::Addr(a) => a,
                                ::gimli::AttributeValue::Udata(a) => a,
                                _ => todo!("{v:?} : {:?}", v.value()),
                                }
                            }
                            let at_lo = v.attr(::gimli::DW_AT_low_pc).map(|v| load_base + unwrap_addr(v));
                            let at_hi = v.attr(::gimli::DW_AT_high_pc).map(|v| load_base + unwrap_addr(v));
                            let ranges = v.attr(::gimli::DW_AT_ranges).map(|v| v.value());
                            PcRanges {
                                ranges: match (at_lo, at_hi, ranges)
                                {
                                (Some(lo), None, _) => vec![PcRange { start: lo, end: lo }],
                                (Some(lo), Some(hi), _) => vec![PcRange { start: lo, end: hi }],
                                (None, _, None) => vec![],
                                (None, _, Some(r)) => {
                                    let r = match r {
                                        gimli::AttributeValue::RangeListsRef(r) => debug_info.raw_ranges(&unit, gimli::RangeListsOffset(r.0)).unwrap(),
                                        _ => todo!(),
                                        };
                                    let mut rv = Vec::new();
                                    let mut base_addr = 0;
                                    for v in r.map(|v| v.unwrap())
                                    {
                                        match v {
                                        gimli::RawRngListEntry::BaseAddress { addr } => base_addr = load_base + addr,
                                        gimli::RawRngListEntry::OffsetPair { begin, end } => rv.push(PcRange { start: base_addr + begin, end: base_addr + end }),
                                        gimli::RawRngListEntry::StartEnd { begin, end } => rv.push(PcRange { start: load_base + begin, end: load_base + end }),
                                        gimli::RawRngListEntry::StartLength { begin, length } => rv.push(PcRange { start: load_base + begin, end: load_base + begin + length }),
                                        _ => todo!("{:?}", v),
                                        }
                                    }
                                    rv
                                },
                                }
                            }
                        };
                        match v.tag()
                        {
                        gimli::DW_TAG_namespace => {
                            let name = get_name(&debug_info, &unit, v);
                            let full_name = get_scoped_name(&stack, "ns", name, v.offset);
                            stack.push(State::Namespace(full_name));
                        },
                        gimli::DW_TAG_subprogram => {
                            let name = get_name(&debug_info, &unit, v);
                            let full_name = get_scoped_name(&stack, "fn", name, v.offset);
                            let frame_base = v.attr_value(::gimli::DW_AT_frame_base).map(|fb| {
                                match fb
                                {
                                ::gimli::AttributeValue::Exprloc(expr) => VariablePosition::Expr(expr.0.to_vec(), unit.encoding()),
                                _ => todo!("Frame base: {:?}", fb),
                                }
                            }).unwrap_or(VariablePosition::OptimisedOut);
                            //println!("fn {}: name={:?} @ {}", full_name, name, v.depth);
                            let pc_range = get_pc_range(v);
                            stack.push(State::InFunction(full_name, FunctionRecord {
                                pc_range,
                                frame_base,
                                variables: Default::default(),
                            }));
                            continue;
                        },

                        gimli::DW_TAG_base_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let size_bits = v.attr_value(::gimli::DW_AT_byte_size)
                                .map(|v| v.udata_value().unwrap() * 8)
                                .or(v.attr_value(::gimli::DW_AT_bit_size).map(|v| v.udata_value().unwrap()));
                            //println!("> {ty_ref:?} base type: {:?}", get_name(&debug_info, &unit, v));
                            self.types[ty_ref.0] = Some(Type::Primtive(super::PrimitiveType {
                                bits: size_bits.expect("No size?") as u32,
                                name: get_scoped_name(&stack, "prim", get_name(&debug_info, &unit, v), v.offset),
                            }));
                            continue
                        },
                        gimli::DW_TAG_typedef => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let target_ty = self.get_typeref_from_attr(unit_index, v);
                            let name = get_name(&debug_info, &unit, v);
                            //println!("> {ty_ref:?} typedef: {:?} ({})", name, get_scoped_name(&stack, "tydef", name, v.offset));
                            if let Some(target_ty) = target_ty {
                                self.types[ty_ref.0] = Some(Type::Alias(target_ty))
                            }
                            // If in a type, save against that type
                            if let State::InType(ct, _, _) = stack.last_mut().unwrap() {
                                if let Some(n) = name {
                                    ct.sub_types.insert(n.to_owned(), ty_ref);
                                }
                            }
                            continue
                        },
                        gimli::DW_TAG_array_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let target_ty = self.get_typeref_from_attr(unit_index, v).expect("No inner type for array");
                            //println!("> {ty_ref:?} array of {:?}", target_ty);
                            self.types[ty_ref.0] = Some(Type::Array(target_ty, 0));
                        },
                        gimli::DW_TAG_structure_type | gimli::DW_TAG_class_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let size = v.attr_value(::gimli::DW_AT_byte_size).map(|v| v.udata_value().expect("not UData")).unwrap_or(0);
                            let name = get_name(&debug_info, &unit, v);
                            let name = get_scoped_name(&stack, "struct", name, v.offset);
                            //println!("> {ty_ref:?} struct: {:?}", name);
                            stack.push(State::InType(CompositeType::new(name, size as usize), ty_ref, false, ));
                            continue;
                        },
                        gimli::DW_TAG_enumeration_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let name = get_scoped_name(&stack, "enum", name, v.offset);
                            //println!("> {ty_ref:?} enum: {:?}", name);
                            self.types[ty_ref.0] = Some(Type::Enum(name));
                            continue;
                        },
                        gimli::DW_TAG_union_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let size = v.attr_value(::gimli::DW_AT_byte_size).unwrap().udata_value().expect("not UData");
                            let name = get_name(&debug_info, &unit, v);
                            let name = get_scoped_name(&stack, "union", name, v.offset);
                            //println!("> {ty_ref:?} union: {:?}", name);
                            stack.push(State::InType(CompositeType::new(name, size as usize), ty_ref, true, ));
                            continue;
                        },
                        gimli::DW_TAG_const_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let target_ty = self.get_typeref_from_attr(unit_index, v);
                            //println!("> {ty_ref:?} const_type: {name:?} = {target_ty:?}", );
                            if let Some(target_ty) = target_ty {
                                self.types[ty_ref.0] = Some(Type::Alias(target_ty))
                            }
                            else {
                                println!("> {ty_ref:?} const_type: {name:?} = {target_ty:?}", );
                            }
                            continue
                        },
                        gimli::DW_TAG_pointer_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let target_ty = self.get_typeref_from_attr(unit_index, v);
                            //println!("> {ty_ref:?} pointer_type: {:?} = {target_ty:?}", get_name(&debug_info, &unit, v));
                            if let Some(target_ty) = target_ty {
                                self.types[ty_ref.0] = Some(Type::Pointer(target_ty, super::PointerClass::Bare));
                            }
                            else {
                                println!("> {ty_ref:?} pointer_type: {:?} = {target_ty:?}", get_name(&debug_info, &unit, v));
                            }
                            continue
                        },
                        gimli::DW_TAG_reference_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let target_ty = self.get_typeref_from_attr(unit_index, v);
                            if let Some(target_ty) = target_ty {
                                self.types[ty_ref.0] = Some(Type::Pointer(target_ty, super::PointerClass::Reference));
                            }
                            else {
                                println!("> {ty_ref:?} reference type: {name:?} = {target_ty:?}",);
                            }
                            continue
                        },
                        gimli::DW_TAG_rvalue_reference_type => {
                            let ty_ref = self.dwarf_type_ref(unit_index, v.offset);
                            let name = get_name(&debug_info, &unit, v);
                            let target_ty = self.get_typeref_from_attr(unit_index, v);
                            if let Some(target_ty) = target_ty {
                                self.types[ty_ref.0] = Some(Type::Pointer(target_ty, super::PointerClass::RValueReference));
                            }
                            else {
                                println!("> {ty_ref:?} rvalue reference type: {name:?} = {target_ty:?}",);
                            }
                            continue
                        },
                        _ => {},
                        }
                        match stack.last_mut().unwrap_or(&mut State::Root) {
                        State::Root => {
                            match v.tag()
                            {
                            gimli::DW_TAG_compile_unit => {
                                stack.push(State::Root);
                            },
                            _ => {},
                            }
                        },
                        State::Namespace(_) => {},
                        &mut State::InFunction(_, FunctionRecord { ref pc_range, .. }) | &mut State::FcnScope(ref pc_range) => {
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
                                //println!(" > Variable {:?} @ {:?} {:?}", name, loc, ty);
                                // Record the type and offset
                                let pos = match loc {
                                    Some(v) => match v
                                        {
                                        gimli::AttributeValue::Addr(v) => VariablePosition::Fixed(v),
                                        gimli::AttributeValue::Exprloc(expression) => {
                                            VariablePosition::Expr(expression.0[..].to_owned(), unit.encoding())
                                        }
                                        gimli::AttributeValue::LocationListsRef(r) => {
                                            let expression = debug_info.locations(&unit, r)
                                                .unwrap()
                                                .next()
                                                .unwrap()
                                                ;
                                            match expression {
                                            Some(expression) => VariablePosition::Expr(expression.data.0[..].to_owned(), unit.encoding()),
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
                                    let pc_range = pc_range.clone();
                                    if !pc_range.is_empty() {
                                        stack.iter_mut()
                                            .filter_map(|v| match v { State::InFunction(_, fr) => Some(fr), _ => None })
                                            .next()
                                            .unwrap()
                                            .variables.entry(name.to_owned())
                                            .or_insert(VariableRecord { ty: ty.unwrap(), ranges: Vec::new() })
                                            .ranges
                                            .push(VariableRange { pc_range, position: pos });
                                    }
                                }
                                },
                            
                            _ => {},
                            }
                        },
                        State::InType(ct, _, is_union) => {
                            match v.tag()
                            {
                            gimli::DW_TAG_GNU_template_template_param => {},
                            gimli::DW_TAG_GNU_template_parameter_pack => {},

                            gimli::DW_TAG_template_type_parameter => {},
                            gimli::DW_TAG_template_value_parameter => {},
                            gimli::DW_TAG_inheritance => {
                                //println!("in {n:?}: {v:?}", n = ct.name);
                                let inner_type = self.get_typeref_from_attr(unit_index, v).expect("No parent type");
                                let inner_ofs = v.attr_value(::gimli::DW_AT_data_member_location)
                                    .expect("no data_member_location");
                                let inner_ofs = match inner_ofs {
                                    gimli::AttributeValue::Exprloc(_) => {
                                        // Ignore?
                                        continue ;
                                    },
                                    gimli::AttributeValue::Udata(v) => v,
                                    gimli::AttributeValue::Data8(v) => v,
                                    a => todo!("{:?}", a),
                                    };
                                ct.parents.push((inner_ofs, inner_type));
                            },

                            gimli::DW_TAG_imported_declaration => {},

                            // static
                            gimli::DW_TAG_variable => {},

                            gimli::DW_TAG_member => {
                                let name = get_name(&debug_info, &unit, v);
                                let offset = match v.attr_value(gimli::DW_AT_data_member_location)
                                    {
                                    None => if *is_union { 0 } else {
                                        // TODO: bitfields have no offset - ignore for now
                                        println!("No offset? in `{ty_name}` {name:?} - v={v:?}", ty_name=ct.name);
                                        continue ;
                                    },
                                    Some(v) => v.udata_value().unwrap(),
                                    };
                                let ty = self.get_typeref_from_attr(unit_index, v);
                                //println!("> MEMBER `{ty_name}` FIELD {name:?} @ {pos:?}: {ty:?} {v:?}");
                                ct.fields.push(CompositeField {
                                    name: match name
                                        {
                                        Some(name) => name.to_owned(),
                                        None => format!("_#{}", v.offset.0),
                                        },
                                    ty: ty.unwrap(),
                                    offset,
                                });
                            },
                            _ => todo!("InType: {:x?}", v),
                            }
                        },
                        }
                    }
                }
            },
            gimli::UnitType::Type { .. } => todo!("gimli::UnitType::Type"),
            gimli::UnitType::Partial => todo!("gimli::UnitType::Partial"),
            gimli::UnitType::Skeleton(..) => todo!("gimli::UnitType::Skeleton"),
            gimli::UnitType::SplitCompilation(..) => todo!("gimli::UnitType::SplitCompilation"),
            gimli::UnitType::SplitType { .. } => todo!("gimli::UnitType::SplitType"),
            }
        }
    }
}