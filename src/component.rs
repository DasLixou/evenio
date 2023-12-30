use alloc::collections::BTreeMap;
use core::alloc::Layout;
use core::any::TypeId;
use std::borrow::Cow;
use std::collections::btree_map::Entry;
use std::ops::Index;
use std::ptr::NonNull;

pub use evenio_macros::Component;

use crate::debug_checked::UnwrapDebugChecked;
use crate::slot_map::{Key, SlotMap};
use crate::sparse::SparseIndex;
use crate::DropFn;

#[derive(Debug)]
pub struct Components {
    sm: SlotMap<ComponentInfo>,
    by_type_id: BTreeMap<TypeId, ComponentId>,
}

impl Components {
    pub(crate) fn new() -> Self {
        Self {
            sm: SlotMap::new(),
            by_type_id: BTreeMap::new(),
        }
    }

    pub(crate) fn add(&mut self, desc: ComponentDescriptor) -> (ComponentId, bool) {
        if let Some(type_id) = desc.type_id {
            return match self.by_type_id.entry(type_id) {
                Entry::Vacant(v) => {
                    let Some(k) = self.sm.insert_with(|k| ComponentInfo {
                        name: desc.name,
                        id: ComponentId(k),
                        type_id: desc.type_id,
                        layout: desc.layout,
                        drop: desc.drop,
                    }) else {
                        panic!("too many components")
                    };

                    (*v.insert(ComponentId(k)), true)
                }
                Entry::Occupied(o) => (*o.get(), false),
            };
        }

        let Some(k) = self.sm.insert_with(|k| ComponentInfo {
            name: desc.name,
            id: ComponentId(k),
            type_id: desc.type_id,
            layout: desc.layout,
            drop: desc.drop,
        }) else {
            panic!("too many components")
        };

        (ComponentId(k), true)
    }

    pub fn get(&self, id: ComponentId) -> Option<&ComponentInfo> {
        self.sm.get(id.0)
    }

    pub fn get_by_index(&self, idx: ComponentIdx) -> Option<&ComponentInfo> {
        self.sm.get_by_index(idx.0).map(|(_, v)| v)
    }

    pub fn get_by_type_id(&self, type_id: TypeId) -> Option<&ComponentInfo> {
        let id = *self.by_type_id.get(&type_id)?;
        Some(unsafe { self.get(id).unwrap_debug_checked() })
    }
}

impl Index<ComponentId> for Components {
    type Output = ComponentInfo;

    fn index(&self, index: ComponentId) -> &Self::Output {
        if let Some(info) = self.get(index) {
            info
        } else {
            panic!("no such component with ID of {index:?} exists")
        }
    }
}

impl Index<ComponentIdx> for Components {
    type Output = ComponentInfo;

    fn index(&self, index: ComponentIdx) -> &Self::Output {
        if let Some(info) = self.get_by_index(index) {
            info
        } else {
            panic!("no such component with index of {index:?} exists")
        }
    }
}

impl Index<TypeId> for Components {
    type Output = ComponentInfo;

    fn index(&self, index: TypeId) -> &Self::Output {
        if let Some(info) = self.get_by_type_id(index) {
            info
        } else {
            panic!("no such component with type ID of {index:?} exists")
        }
    }
}

#[derive(Debug)]
pub struct ComponentInfo {
    name: Cow<'static, str>,
    id: ComponentId,
    type_id: Option<TypeId>,
    layout: Layout,
    drop: DropFn,
}

impl ComponentInfo {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn id(&self) -> ComponentId {
        self.id
    }

    pub fn type_id(&self) -> Option<TypeId> {
        self.type_id
    }

    pub fn layout(&self) -> Layout {
        self.layout
    }

    pub fn drop(&self) -> DropFn {
        self.drop
    }
}

pub trait Component: Send + Sync + 'static {
    /// If this component permits mutation through mutable references (`&mut
    /// T`).
    ///
    /// ```compile_fail
    /// use evenio::prelude::*;
    ///
    /// struct MyComponent;
    ///
    /// impl Component for MyComponent {
    ///     const MUTABLE: bool = false;
    /// }
    ///
    /// fn my_system(_: Fetcher<&mut MyComponent>) {}
    ///
    /// let mut world = World::new();
    ///
    /// // Fails to compile because `&mut MyComponent` is not allowed.
    /// world.add_system(my_system);
    /// ```
    const MUTABLE: bool = true;
}

#[derive(Clone, Debug)]
pub struct ComponentDescriptor {
    pub name: Cow<'static, str>,
    pub type_id: Option<TypeId>,
    pub layout: Layout,
    pub drop: DropFn,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct ComponentId(Key);

impl ComponentId {
    pub const NULL: Self = Self(Key::NULL);

    pub const fn index(self) -> ComponentIdx {
        ComponentIdx(self.0.index())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct ComponentIdx(pub u32);

unsafe impl SparseIndex for ComponentIdx {
    const MAX: Self = Self(u32::MAX);

    fn index(self) -> usize {
        self.0.index()
    }

    fn from_index(idx: usize) -> Self {
        Self(u32::from_index(idx))
    }
}
