use super::{TypeRef, Type, CompositeType, CompositeField};
use super::{FunctionRecord, PcRanges, PcRange, VariableRecord, VariablePosition, VariableRange};

impl super::DebugPool
{
    pub(super) fn add_variables_types_from_dwarf(&mut self, debug_info: &::gimli::Dwarf<::gimli::EndianSlice<::gimli::LittleEndian>>)
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
                    InType(String, TypeRef, bool, Vec<CompositeField>),
                    InFunction(String, FunctionRecord),
                    // Should only exist underneath a `InFunction`
                    FcnScope(PcRanges),
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
                        State::InType(n, _, _, _) => return Some(n),
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
                                self.functions.insert(n, fr);
                            } 
                            State::InType(name, ty, is_union, fields) => {
                                println!("END type {}", name);
                                self.types[ty.0] = Some(if is_union {
                                    Type::Union(CompositeType { name, fields })
                                }else {
                                    Type::Struct(CompositeType { name, fields })
                                });
                            }
                            _ => {},
                            }
                        }
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
                            let at_lo = v.attr(::gimli::DW_AT_low_pc).map(|v| unwrap_addr(v));
                            let at_hi = v.attr(::gimli::DW_AT_high_pc).map(|v| unwrap_addr(v));
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
                                        gimli::RawRngListEntry::BaseAddress { addr } => base_addr = addr,
                                        gimli::RawRngListEntry::OffsetPair { begin, end } => rv.push(PcRange { start: base_addr + begin, end: base_addr + end }),
                                        gimli::RawRngListEntry::StartEnd { begin, end } => rv.push(PcRange { start: begin, end }),
                                        gimli::RawRngListEntry::StartLength { begin, length } => rv.push(PcRange { start: begin, end: begin + length }),
                                        _ => todo!("{:?}", v),
                                        }
                                    }
                                    rv
                                },
                                }
                            }
                        };
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
                            stack.push(State::InType(full_name, ty_ref, false, vec![]));
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
                            stack.push(State::InType(full_name, ty_ref, true, Vec::new()));
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
                        State::InType(ty_name, _, is_union, fields) => {
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
                                let offset = match v.attr_value(gimli::DW_AT_data_member_location)
                                    {
                                    None => if *is_union { 0 } else { todo!("No offset? in `{ty_name}` {name:?} - v={:?}", v) },
                                    Some(v) => v.udata_value().unwrap(),
                                    };
                                let ty = match v.attr_value(gimli::DW_AT_type)
                                    {
                                    None => None,
                                    Some(ty) => {
                                        Some(match ty {
                                            gimli::AttributeValue::UnitRef(r) => self.dwarf_type_ref(unit_index, r),
                                            _ => todo!("Register type: {:?} {:?}", ty, ty.offset_value()),
                                            })
                                    },
                                    };
                                //println!("> MEMBER `{ty_name}` FIELD {name:?} @ {pos:?}: {ty:?} {v:?}");
                                fields.push(CompositeField {
                                    name: match name
                                        {
                                        Some(name) => name.to_owned(),
                                        None => todo!("Unnamed fields"),
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
            gimli::UnitType::Type { type_signature, type_offset } => todo!(),
            gimli::UnitType::Partial => todo!(),
            gimli::UnitType::Skeleton(dwo_id) => todo!(),
            gimli::UnitType::SplitCompilation(dwo_id) => todo!(),
            gimli::UnitType::SplitType { type_signature, type_offset } => todo!(),
            }
        }
    }
}