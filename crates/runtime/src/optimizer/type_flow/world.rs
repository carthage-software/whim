use std::iter::Copied;
use std::ops::Deref;
use std::rc::Rc;
use std::slice;

use hashbrown::HashMap;

use crate::bytecode::aliases::TypeAliasLookup;
use crate::bytecode::aliases::expand_aliases_using;
use crate::bytecode::chunk::descriptors::Literal;
use crate::bytecode::chunk::descriptors::TypeDescriptor;
use crate::bytecode::unit::CompiledBuiltInFunction;
use crate::bytecode::unit::CompiledClassLike;
use crate::bytecode::unit::CompiledConstant;
use crate::bytecode::unit::CompiledFunction;
use crate::bytecode::unit::CompiledNewtype;
use crate::bytecode::unit::CompiledTypeAlias;
use crate::bytecode::unit::CompiledUnit;
use crate::bytecode::unit::ConstantInitializer;
use crate::limits::MAX_TYPE_DEPTH;
use crate::optimizer::type_flow::BOOL;
use crate::optimizer::type_flow::CALLABLE;
use crate::optimizer::type_flow::FLOAT;
use crate::optimizer::type_flow::INT;
use crate::optimizer::type_flow::NULL;
use crate::optimizer::type_flow::OBJECT;
use crate::optimizer::type_flow::STRING;
use crate::optimizer::type_flow::descriptors;
use crate::optimizer::type_flow::same_atom;
use crate::value::atom::Atom;

/// A unit with indexes over its declarations.
pub(crate) struct IndexedUnit<'a> {
    unit: &'a CompiledUnit,
    world: &'a World<'a>,
    functions_by_name: HashMap<&'a [u8], usize>,
    classes_by_name: HashMap<&'a [u8], usize>,
    constants_by_name: HashMap<&'a [u8], usize>,
    type_aliases_by_name: HashMap<&'a [u8], usize>,
    newtypes_by_name: HashMap<&'a [u8], usize>,
}

/// Reusable declaration indexes for the engine's current loaded-unit set.
pub(crate) struct WorldCache {
    units: Vec<Rc<CompiledUnit>>,
    index: WorldIndex,
}

/// The declarations loaded before the unit being optimized.
pub(crate) struct World<'a> {
    units: WorldUnits<'a>,
    built_in_functions: &'a [CompiledBuiltInFunction],
    index: WorldIndexStorage<'a>,
}

struct WorldIndex {
    functions_by_name: HashMap<Atom, (usize, usize)>,
    built_in_functions_by_name: HashMap<Atom, usize>,
    classes_by_name: HashMap<Atom, (usize, usize)>,
    constants_by_name: HashMap<Atom, (usize, usize)>,
    type_aliases_by_name: HashMap<Atom, (usize, usize)>,
    newtypes_by_name: HashMap<Atom, (usize, usize)>,
}

enum WorldIndexStorage<'a> {
    Owned(Box<WorldIndex>),
    Cached(&'a WorldIndex),
}

enum WorldUnits<'a> {
    Borrowed(&'a [&'a CompiledUnit]),
    Cached(&'a [Rc<CompiledUnit>]),
}

pub(in crate::optimizer) enum WorldUnitIter<'a> {
    Borrowed(Copied<slice::Iter<'a, &'a CompiledUnit>>),
    Cached(slice::Iter<'a, Rc<CompiledUnit>>),
}

impl<'a> World<'a> {
    pub(crate) fn new(
        units: &'a [&'a CompiledUnit],
        built_in_functions: &'a [CompiledBuiltInFunction],
    ) -> Self {
        let index = WorldIndex::new(units.iter().copied(), built_in_functions);
        Self {
            units: WorldUnits::Borrowed(units),
            built_in_functions,
            index: WorldIndexStorage::Owned(Box::new(index)),
        }
    }

    pub(crate) fn from_cache(
        cache: &'a WorldCache,
        built_in_functions: &'a [CompiledBuiltInFunction],
    ) -> Self {
        Self {
            units: WorldUnits::Cached(&cache.units),
            built_in_functions,
            index: WorldIndexStorage::Cached(&cache.index),
        }
    }

    pub(in crate::optimizer) fn units(&self) -> WorldUnitIter<'_> {
        self.units.iter()
    }

    fn index(&self) -> &WorldIndex {
        match &self.index {
            WorldIndexStorage::Owned(index) => index,
            WorldIndexStorage::Cached(index) => index,
        }
    }
}

impl WorldCache {
    pub(crate) fn new(
        units: Vec<Rc<CompiledUnit>>,
        built_in_functions: &[CompiledBuiltInFunction],
    ) -> Self {
        let index = WorldIndex::new(units.iter().map(Rc::as_ref), built_in_functions);
        Self { units, index }
    }

    pub(crate) fn push(&mut self, unit: Rc<CompiledUnit>) {
        self.index.insert_unit(self.units.len(), &unit);
        self.units.push(unit);
    }
}

impl WorldIndex {
    fn new<'a>(
        units: impl IntoIterator<Item = &'a CompiledUnit>,
        built_in_functions: &[CompiledBuiltInFunction],
    ) -> Self {
        let mut index = Self {
            functions_by_name: HashMap::new(),
            built_in_functions_by_name: HashMap::with_capacity(built_in_functions.len()),
            classes_by_name: HashMap::new(),
            constants_by_name: HashMap::new(),
            type_aliases_by_name: HashMap::new(),
            newtypes_by_name: HashMap::new(),
        };
        for (position, function) in built_in_functions.iter().enumerate() {
            index
                .built_in_functions_by_name
                .entry(function.name.clone())
                .or_insert(position);
        }
        for (position, unit) in units.into_iter().enumerate() {
            index.insert_unit(position, unit);
        }
        index
    }

    fn insert_unit(&mut self, position: usize, unit: &CompiledUnit) {
        insert_positions(
            &mut self.functions_by_name,
            position,
            &unit.functions,
            |entry| &entry.name,
        );
        insert_positions(
            &mut self.classes_by_name,
            position,
            &unit.classes,
            |entry| &entry.name,
        );
        insert_positions(
            &mut self.constants_by_name,
            position,
            &unit.constants,
            |entry| &entry.name,
        );
        insert_positions(
            &mut self.type_aliases_by_name,
            position,
            &unit.type_aliases,
            |entry| &entry.name,
        );
        insert_positions(
            &mut self.newtypes_by_name,
            position,
            &unit.newtypes,
            |entry| &entry.name,
        );
    }
}

impl WorldUnits<'_> {
    fn get(&self, position: usize) -> &CompiledUnit {
        match self {
            Self::Borrowed(units) => units[position],
            Self::Cached(units) => &units[position],
        }
    }

    fn iter(&self) -> WorldUnitIter<'_> {
        match self {
            Self::Borrowed(units) => WorldUnitIter::Borrowed(units.iter().copied()),
            Self::Cached(units) => WorldUnitIter::Cached(units.iter()),
        }
    }
}

impl<'a> Iterator for WorldUnitIter<'a> {
    type Item = &'a CompiledUnit;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Borrowed(units) => units.next(),
            Self::Cached(units) => units.next().map(Rc::as_ref),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Borrowed(units) => units.size_hint(),
            Self::Cached(units) => units.size_hint(),
        }
    }
}

impl ExactSizeIterator for WorldUnitIter<'_> {}

impl<'a> IndexedUnit<'a> {
    pub(crate) fn with_world(unit: &'a CompiledUnit, world: &'a World<'a>) -> Self {
        Self {
            unit,
            world,
            functions_by_name: positions_by_name(&unit.functions, |entry| &entry.name),
            classes_by_name: positions_by_name(&unit.classes, |entry| &entry.name),
            constants_by_name: positions_by_name(&unit.constants, |entry| &entry.name),
            type_aliases_by_name: positions_by_name(&unit.type_aliases, |entry| &entry.name),
            newtypes_by_name: positions_by_name(&unit.newtypes, |entry| &entry.name),
        }
    }

    pub(in crate::optimizer) fn function_by_name(&self, name: &Atom) -> Option<&CompiledFunction> {
        self.declaration_by_name(
            name,
            &self.functions_by_name,
            &self.world.index().functions_by_name,
            |unit| &unit.functions,
        )
    }

    pub(in crate::optimizer) fn local_function_index(&self, name: &Atom) -> Option<usize> {
        self.functions_by_name.get(name.as_bytes()).copied()
    }

    pub(in crate::optimizer) fn built_in_function_by_name(
        &self,
        name: &Atom,
    ) -> Option<&CompiledBuiltInFunction> {
        Some(
            &self.world.built_in_functions
                [*self.world.index().built_in_functions_by_name.get(name)?],
        )
    }

    pub(in crate::optimizer) fn class_by_name(&self, name: &Atom) -> Option<&CompiledClassLike> {
        self.declaration_by_name(
            name,
            &self.classes_by_name,
            &self.world.index().classes_by_name,
            |unit| &unit.classes,
        )
    }

    pub(in crate::optimizer) fn constant_by_name(&self, name: &Atom) -> Option<&CompiledConstant> {
        self.declaration_by_name(
            name,
            &self.constants_by_name,
            &self.world.index().constants_by_name,
            |unit| &unit.constants,
        )
    }

    pub(in crate::optimizer) fn newtype_by_name(&self, name: &Atom) -> Option<&CompiledNewtype> {
        self.declaration_by_name(
            name,
            &self.newtypes_by_name,
            &self.world.index().newtypes_by_name,
            |unit| &unit.newtypes,
        )
    }

    pub(in crate::optimizer) fn descriptor_mask(
        &self,
        descriptor: &TypeDescriptor,
        depth: usize,
    ) -> Option<u16> {
        if depth > MAX_TYPE_DEPTH {
            return None;
        }
        match descriptor {
            TypeDescriptor::Named {
                name, arguments, ..
            } => {
                if self.class_by_name(name).is_some() {
                    return Some(OBJECT);
                }
                if self.function_by_name(name).is_some()
                    || self.built_in_function_by_name(name).is_some()
                {
                    return Some(CALLABLE);
                }
                if let Some(constant) = self.constant_by_name(name) {
                    return initializer_mask(&constant.initializer);
                }
                let newtype = self.newtype_by_name(name)?;
                let backing = descriptors::substitute_parameters(
                    &newtype.backing,
                    &newtype.type_parameters,
                    arguments.as_deref(),
                    depth + 1,
                );
                self.descriptor_mask(&backing, depth + 1)
            }
            TypeDescriptor::Member { class, member, .. } => {
                let class = self.class_by_name(class)?;
                if class
                    .methods
                    .iter()
                    .any(|method| same_atom(&method.name, member))
                {
                    return Some(CALLABLE);
                }
                if let Some(constant) = class
                    .constants
                    .iter()
                    .find(|constant| same_atom(&constant.name, member))
                {
                    return initializer_mask(&constant.initializer);
                }
                class
                    .cases
                    .iter()
                    .any(|case| same_atom(&case.name, member))
                    .then_some(OBJECT)
            }
            TypeDescriptor::Union(members) => {
                let mut mask = 0;
                for member in members {
                    mask |= self.descriptor_mask(member, depth + 1)?;
                }
                Some(mask)
            }
            TypeDescriptor::Intersection(members) => {
                let mut members = members.iter();
                let mut mask = self.descriptor_mask(members.next()?, depth + 1)?;
                for member in members {
                    mask &= self.descriptor_mask(member, depth + 1)?;
                }
                Some(mask)
            }
            _ => descriptors::descriptor_mask(descriptor),
        }
    }

    pub(in crate::optimizer) fn expand_aliases(
        &self,
        descriptor: &TypeDescriptor,
    ) -> TypeDescriptor {
        expand_aliases_using(descriptor, self)
    }

    fn declaration_by_name<T>(
        &self,
        name: &Atom,
        local_positions: &HashMap<&[u8], usize>,
        world_positions: &HashMap<Atom, (usize, usize)>,
        entries: impl for<'unit> Fn(&'unit CompiledUnit) -> &'unit [T],
    ) -> Option<&T> {
        if let Some(position) = local_positions.get(name.as_bytes()) {
            return Some(&entries(self.unit)[*position]);
        }

        let (unit, position) = *world_positions.get(name)?;
        Some(&entries(self.world.units.get(unit))[position])
    }
}

impl TypeAliasLookup for IndexedUnit<'_> {
    fn find_alias(&self, name: &Atom) -> Option<&CompiledTypeAlias> {
        self.declaration_by_name(
            name,
            &self.type_aliases_by_name,
            &self.world.index().type_aliases_by_name,
            |unit| &unit.type_aliases,
        )
    }
}

impl Deref for IndexedUnit<'_> {
    type Target = CompiledUnit;

    fn deref(&self) -> &CompiledUnit {
        self.unit
    }
}

fn positions_by_name<T>(entries: &[T], name: impl Fn(&T) -> &Atom) -> HashMap<&[u8], usize> {
    let mut positions = HashMap::with_capacity(entries.len());
    for (position, entry) in entries.iter().enumerate() {
        positions.entry(name(entry).as_bytes()).or_insert(position);
    }

    positions
}

fn insert_positions<T>(
    positions: &mut HashMap<Atom, (usize, usize)>,
    unit: usize,
    entries: &[T],
    name: impl Fn(&T) -> &Atom,
) {
    for (position, entry) in entries.iter().enumerate() {
        positions
            .entry(name(entry).clone())
            .or_insert((unit, position));
    }
}

fn initializer_mask(initializer: &ConstantInitializer) -> Option<u16> {
    let ConstantInitializer::Literal(literal) = initializer else {
        return None;
    };
    Some(match literal {
        Literal::Null => NULL,
        Literal::Bool(_) => BOOL,
        Literal::Int(_) => INT,
        Literal::Float(_) => FLOAT,
        Literal::String(_) => STRING,
    })
}
