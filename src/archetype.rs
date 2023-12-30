use alloc::collections::BTreeMap;
use core::ptr::NonNull;
use std::cmp::Ordering;
use std::collections::btree_map::Entry;
use std::collections::BTreeSet;
use std::ptr;

use slab::Slab;

use crate::access::Access;
use crate::blob_vec::BlobVec;
use crate::component::{ComponentIdx, Components};
use crate::debug_checked::{GetDebugChecked, UnwrapDebugChecked};
use crate::entity::{Entities, EntityId, EntityLocation};
use crate::event::{EntityEventIdx, EventIdx};
use crate::sparse::SparseIndex;
use crate::sparse_map::SparseMap;
use crate::system::{RefreshArchetypeReason, SystemInfo, SystemInfoPtr, SystemList, Systems};

#[derive(Debug)]
pub struct Archetypes {
    archetypes: Slab<Archetype>,
    by_components: BTreeMap<Box<[ComponentIdx]>, ArchetypeIdx>,
}

impl Archetypes {
    pub(crate) fn new() -> Self {
        Self {
            archetypes: Slab::from_iter([(0, Archetype::empty())]),
            by_components: BTreeMap::from_iter([(vec![].into_boxed_slice(), ArchetypeIdx::EMPTY)]),
            // indices: BinaryHeap::from_iter([ArchetypeIdx::EMPTY]),
        }
    }

    pub fn empty(&self) -> &Archetype {
        // SAFETY: The empty archetype is always at index 0.
        unsafe { self.archetypes.get_debug_checked(0) }
    }

    pub(crate) fn empty_mut(&mut self) -> &mut Archetype {
        // SAFETY: The empty archetype is always at index 0.
        unsafe { self.archetypes.get_debug_checked_mut(0) }
    }

    pub fn get(&self, idx: ArchetypeIdx) -> Option<&Archetype> {
        self.archetypes.get(idx.0 as usize)
    }

    pub fn get_by_components(&self, components: &[ComponentIdx]) -> Option<&Archetype> {
        let idx = *self.by_components.get(components)?;
        Some(unsafe { self.get(idx).unwrap_debug_checked() })
    }

    /*
    pub fn max_archetype_index(&self) -> ArchetypeIdx {
        // SAFETY: `indices` is nonempty because the empty archetype is always present.
        *unsafe { self.indices.peek().unwrap_debug_checked() }
    }
    */

    pub fn iter(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter().map(|(_, v)| v)
    }

    /*
    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut Archetype> {
        self.archetypes.iter_mut().map(|(_, v)| v)
    }
    */

    pub(crate) fn register_system(&mut self, info: &mut SystemInfo) {
        // TODO: use a `Component -> Vec<Archetype>` index to make this faster?
        for (_, arch) in self.archetypes.iter_mut() {
            arch.register_system(info);
        }
    }

    /// Traverses one edge of the archetype graph in the insertion direction.
    /// Returns the destination archetype.
    pub(crate) unsafe fn traverse_insert(
        &mut self,
        src_arch_idx: ArchetypeIdx,
        component_idx: ComponentIdx,
        components: &mut Components,
        systems: &mut Systems,
    ) -> ArchetypeIdx {
        let next_arch_idx = self.archetypes.vacant_key();

        let src_arch = unsafe {
            self.archetypes
                .get_debug_checked_mut(src_arch_idx.0 as usize)
        };

        match src_arch.insert_components.entry(component_idx) {
            Entry::Vacant(vacant_insert_components) => {
                let Err(idx) = src_arch
                    .columns
                    .binary_search_by_key(&component_idx, |c| c.component_idx)
                else {
                    // Archetype already has this component.
                    return src_arch_idx;
                };

                let mut new_components = Vec::with_capacity(src_arch.columns.len() + 1);
                new_components.extend(src_arch.columns.iter().map(|c| c.component_idx));
                new_components.insert(idx, component_idx);

                match self.by_components.entry(new_components.into_boxed_slice()) {
                    Entry::Vacant(vacant_by_components) => {
                        if next_arch_idx >= u32::MAX as usize {
                            panic!("too many archetypes");
                        }

                        let arch_id = ArchetypeIdx(next_arch_idx as u32);

                        let mut new_arch = Archetype::new(
                            arch_id,
                            vacant_by_components.key().iter().copied(),
                            components,
                        );

                        new_arch
                            .remove_components
                            .insert(component_idx, src_arch_idx);

                        for info in systems.iter_mut() {
                            new_arch.register_system(info);
                        }

                        vacant_by_components.insert(arch_id);

                        vacant_insert_components.insert(arch_id);

                        self.archetypes.insert(new_arch);

                        arch_id
                    }
                    Entry::Occupied(o) => *vacant_insert_components.insert(*o.get()),
                }
            }
            Entry::Occupied(o) => *o.get(),
        }
    }

    /// Traverses one edge of the archetype graph in the remove direction.
    /// Returns the destination archetype.
    pub(crate) unsafe fn traverse_remove(
        &mut self,
        src_arch_idx: ArchetypeIdx,
        component_idx: ComponentIdx,
        components: &mut Components,
        systems: &mut Systems,
    ) -> ArchetypeIdx {
        let next_arch_idx = self.archetypes.vacant_key();

        let src_arch = unsafe {
            self.archetypes
                .get_debug_checked_mut(src_arch_idx.0 as usize)
        };

        match src_arch.remove_components.entry(component_idx) {
            Entry::Vacant(vacant_remove_components) => {
                if src_arch
                    .columns
                    .binary_search_by_key(&component_idx, |c| c.component_idx)
                    .is_err()
                {
                    // Archetype already doesn't have the component.
                    return src_arch_idx;
                }

                let mut new_components = Vec::with_capacity(src_arch.columns.len() - 1);
                new_components.extend(
                    src_arch
                        .columns
                        .iter()
                        .map(|c| c.component_idx)
                        .filter(|&c| c != component_idx),
                );

                match self.by_components.entry(new_components.into_boxed_slice()) {
                    Entry::Vacant(vacant_by_components) => {
                        if next_arch_idx >= u32::MAX as usize {
                            panic!("too many archetypes");
                        }

                        let arch_id = ArchetypeIdx(next_arch_idx as u32);

                        let mut new_arch = Archetype::new(
                            arch_id,
                            vacant_by_components.key().iter().copied(),
                            components,
                        );

                        new_arch
                            .insert_components
                            .insert(component_idx, src_arch_idx);

                        for info in systems.iter_mut() {
                            new_arch.register_system(info);
                        }

                        vacant_by_components.insert(arch_id);

                        vacant_remove_components.insert(arch_id);

                        self.archetypes.insert(new_arch);

                        arch_id
                    }
                    Entry::Occupied(o) => *vacant_remove_components.insert(*o.get()),
                }
            }
            Entry::Occupied(o) => *o.get(),
        }
    }

    /// Move an entity from one archetype to another. Returns the entity's row
    /// in the new archetype.
    pub(crate) unsafe fn move_entity(
        &mut self,
        src: EntityLocation,
        dst: ArchetypeIdx,
        new_components: impl IntoIterator<Item = (ComponentIdx, *mut u8)>,
        entities: &mut Entities,
    ) -> ArchetypeRow {
        if src.archetype == dst {
            return src.row;
        }

        let (src_arch, dst_arch) = self
            .archetypes
            .get2_mut(src.archetype.0 as usize, dst.0 as usize)
            .unwrap();

        let dst_row = ArchetypeRow(dst_arch.entity_ids.len() as u32);

        // Will the columns of the destination archetype reallocate once we move the
        // entity to it? All columns should have the same capacity and length,
        // so we only need to look at one of them. The `Vec` holding the Entity IDs
        // might have a different reallocation strategy, so check that too.
        //
        // Note that we cannot just compare pointers before and after to check for
        // reallocation, as that would lead to Undefined Behavior.
        // `std::alloc::GlobalAlloc::realloc` says:
        // > Any access to the old `ptr` is Undefined Behavior, even if the allocation
        // > remained in-place.
        let dst_arch_reallocated = dst_arch
            .columns
            .first()
            .map_or(false, |col| col.data.len() == col.data.capacity())
            || dst_arch.entity_ids.capacity() == dst_arch.entity_ids.len();

        let mut src_it = src_arch.columns.iter_mut().peekable();
        let mut dst_it = dst_arch.columns.iter_mut().peekable();

        let mut new_components = new_components.into_iter();

        // TODO: does this optimize better with raw pointers?
        loop {
            match (src_it.peek_mut(), dst_it.peek_mut()) {
                (None, None) => break,
                (None, Some(dst_col)) => {
                    let (component_id, component_ptr) =
                        new_components.next().unwrap_debug_checked();

                    debug_assert_eq!(component_id, dst_col.component_index());

                    ptr::copy_nonoverlapping(
                        component_ptr,
                        dst_col.data.push().as_ptr(),
                        dst_col.data.elem_layout().size(),
                    );

                    dst_it.next();
                }
                (Some(src_col), None) => {
                    src_col.data.swap_remove(src.row.0 as usize);
                    src_it.next();
                }
                (Some(src_col), Some(dst_col)) => {
                    match src_col.component_index().cmp(&dst_col.component_index()) {
                        Ordering::Less => {
                            src_col.data.swap_remove(src.row.0 as usize);
                            src_it.next();
                        }
                        Ordering::Equal => {
                            src_col
                                .data
                                .transfer_elem(&mut dst_col.data, src.row.0 as usize);

                            src_it.next();
                            dst_it.next();
                        }
                        Ordering::Greater => {
                            let (component_id, component_ptr) =
                                new_components.next().unwrap_debug_checked();

                            debug_assert_eq!(component_id, dst_col.component_index());

                            ptr::copy_nonoverlapping(
                                component_ptr,
                                dst_col.data.push().as_ptr(),
                                dst_col.data.elem_layout().size(),
                            );

                            dst_it.next();
                        }
                    }
                }
            }
        }

        debug_assert!(new_components.next().is_none());

        let entity_id = src_arch.entity_ids.swap_remove(src.row.0 as usize);
        dst_arch.entity_ids.push(entity_id);

        *unsafe { entities.get_mut(entity_id).unwrap_debug_checked() } = EntityLocation {
            archetype: dst,
            row: dst_row,
        };

        if let Some(&swapped_entity_id) = src_arch.entity_ids.get(src.row.0 as usize) {
            unsafe { entities.get_mut(swapped_entity_id).unwrap_debug_checked() }.row = src.row;
        }

        if src_arch.entity_ids.is_empty() {
            for &sys in src_arch.refresh_listeners.iter() {
                unsafe {
                    (*sys.as_ptr())
                        .system
                        .refresh_archetype(RefreshArchetypeReason::Empty, src_arch);
                }
            }
        }

        if dst_arch_reallocated {
            for &sys in dst_arch.refresh_listeners.iter() {
                unsafe {
                    (*sys.as_ptr())
                        .system
                        .refresh_archetype(RefreshArchetypeReason::RefreshPointers, dst_arch);
                }
            }
        }

        if dst_arch.entity_ids.len() == 1 {
            for &sys in dst_arch.refresh_listeners.iter() {
                unsafe {
                    (*sys.as_ptr())
                        .system
                        .refresh_archetype(RefreshArchetypeReason::Nonempty, dst_arch);
                }
            }
        }

        dst_row
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct ArchetypeIdx(pub u32);

impl ArchetypeIdx {
    /// Index of the archetype with no components.
    pub const EMPTY: Self = Self(0);
    /// The archetype index that is always invalid.
    pub const NULL: Self = Self(u32::MAX);
}

unsafe impl SparseIndex for ArchetypeIdx {
    const MAX: Self = ArchetypeIdx::NULL;

    fn index(self) -> usize {
        self.0.index()
    }

    fn from_index(idx: usize) -> Self {
        Self(u32::from_index(idx))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ArchetypeRow(pub u32);

#[derive(Debug)]
pub struct Archetype {
    /// The index of this archetype. Provided here for convenience.
    index: ArchetypeIdx,
    /// Entity IDs of the entities in this archetype.
    entity_ids: Vec<EntityId>,
    /// Columns of component data in this archetype. Sorted by component index.
    columns: Box<[Column]>,
    insert_components: BTreeMap<ComponentIdx, ArchetypeIdx>,
    remove_components: BTreeMap<ComponentIdx, ArchetypeIdx>,
    /// Systems that need to be notified about column changes.
    refresh_listeners: BTreeSet<SystemInfoPtr>,
    /// Entity event listeners for this archetype.
    event_listeners: SparseMap<EntityEventIdx, SystemList>,
}

impl Archetype {
    fn empty() -> Self {
        Self {
            index: ArchetypeIdx::EMPTY,
            entity_ids: vec![],
            columns: Box::new([]),
            insert_components: BTreeMap::new(),
            remove_components: BTreeMap::new(),
            refresh_listeners: BTreeSet::new(),
            event_listeners: SparseMap::new(),
        }
    }

    /// # Safety
    ///
    /// Iterator must be sorted in ascending order and all IDs must be valid.
    unsafe fn new(
        index: ArchetypeIdx,
        ids: impl IntoIterator<Item = ComponentIdx>,
        comps: &Components,
    ) -> Self {
        Self {
            entity_ids: vec![],
            columns: ids
                .into_iter()
                .map(|idx| {
                    let comp = unsafe {
                        comps
                            .get_by_index(idx)
                            .expect_debug_checked("invalid component ID")
                    };

                    Column {
                        data: unsafe { BlobVec::new(comp.layout(), comp.drop()) },
                        component_idx: idx,
                    }
                })
                .collect(),
            insert_components: BTreeMap::new(),
            remove_components: BTreeMap::new(),
            refresh_listeners: BTreeSet::new(),
            event_listeners: SparseMap::new(),
            index,
        }
    }

    /// Add an entity to this archetype.
    pub(crate) unsafe fn add_entity(
        &mut self,
        id: EntityId,
    ) -> (ArchetypeRow, impl Iterator<Item = NonNull<u8>> + '_) {
        debug_assert!(self.entity_ids.len() <= u32::MAX as usize);

        let row = ArchetypeRow(self.entity_ids.len() as u32);
        self.entity_ids.push(id);

        // TODO: refresh archetype notification for systems?

        let iter = self.columns.iter_mut().map(|col| col.data.push());

        (row, iter)
    }

    fn register_system(&mut self, info: &mut SystemInfo) {
        if self
            .columns
            .iter()
            .any(|c| info.component_access().access.get(c.component_idx) != Access::None)
            && info
                .component_access()
                .expr
                .eval(|idx| self.column_of(idx).is_some())
        {
            unsafe {
                info.system_mut()
                    .refresh_archetype(RefreshArchetypeReason::New, self)
            };

            self.refresh_listeners.insert(info.ptr());
        }

        if let (Some(expr), EventIdx::Entity(entity_event_idx)) =
            (info.entity_event_expr(), info.received_event().index())
        {
            if expr.eval(|idx| self.column_of(idx).is_some()) {
                if let Some(list) = self.event_listeners.get_mut(entity_event_idx) {
                    list.insert(info.ptr(), info.priority());
                } else {
                    let mut list = SystemList::new();
                    list.insert(info.ptr(), info.priority());

                    self.event_listeners.insert(entity_event_idx, list);
                }
            }
        }
    }

    pub(crate) fn system_list_for(&self, idx: EntityEventIdx) -> Option<&SystemList> {
        self.event_listeners.get(idx)
    }

    pub fn index(&self) -> ArchetypeIdx {
        self.index
    }

    pub fn entity_count(&self) -> u32 {
        self.entity_ids.len() as u32
    }

    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    pub fn column_of(&self, idx: ComponentIdx) -> Option<&Column> {
        let idx = self
            .columns
            .binary_search_by_key(&idx, |c| c.component_idx)
            .ok()?;

        Some(unsafe { self.columns.get_debug_checked(idx) })
    }
}

#[derive(Debug)]
pub struct Column {
    /// Component data in this column.
    data: BlobVec,
    /// Type of data in this column.
    component_idx: ComponentIdx,
}

impl Column {
    pub fn data(&self) -> NonNull<u8> {
        self.data.as_ptr()
    }

    pub fn component_index(&self) -> ComponentIdx {
        self.component_idx
    }
}

// SAFETY: Components are guaranteed `Send` and `Sync`.
unsafe impl Send for Column {}
unsafe impl Sync for Column {}
